//! toolnet's core for the Android app. The Kotlin side (io.manwe.toolnet.Core)
//! calls these over JNI; there are no callbacks into Kotlin. The app polls
//! `status()`, which says what VPN the core wants (tailnet IP, prefix, routes,
//! with a generation number), builds it with VpnService and hands the TUN file
//! descriptor back with `setTun()`.

use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use anyhow::{Context, Result};
use jni::{
    JNIEnv,
    objects::{JClass, JString},
    sys::{jboolean, jint, jstring},
};
use nostr_sdk::prelude::{Keys, ToBech32};
use toolnet::{
    model::Status,
    node::{MEMBERSHIP_GRACE, Node, NodeOpts, peers_of},
    state::{Config, State, join_payload, parse_pubkey, relays_or_default},
};
use tracing::{info, warn};

/// What the core wants the VPN to look like; the app rebuilds the VPN when
/// `generation` changes
#[derive(Clone, Default, PartialEq)]
struct VpnWanted {
    generation: u64,
    ip: String,
    prefix: u8,
    /// Extra prefixes to route into the VPN (approved subnet routes)
    routes: Vec<String>,
}

#[derive(Default)]
struct Shared {
    status: String,
    member: bool,
    vpn: Option<VpnWanted>,
    peers: Vec<serde_json::Value>,
    error: Option<String>,
}

struct Running {
    runtime: tokio::runtime::Runtime,
    engine: Option<toolnet::engine::Engine>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
static SHARED: Mutex<Option<Shared>> = Mutex::new(None);
static LOGS: OnceLock<()> = OnceLock::new();

fn logs() {
    LOGS.get_or_init(|| {
        #[cfg(target_os = "android")]
        {
            use tracing_subscriber::prelude::*;
            let filter = tracing_subscriber::filter::Targets::new()
                .with_default(tracing::Level::INFO)
                .with_target("toolnet", tracing::Level::DEBUG);
            let _ = tracing_subscriber::registry().with(paranoid_android::layer("toolnet")).with(filter).try_init();
        }
    });
}

fn string(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(|s| s.into()).unwrap_or_default()
}

fn jstr(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn set_shared(f: impl FnOnce(&mut Shared)) {
    let mut g = SHARED.lock().unwrap();
    f(g.get_or_insert_with(Default::default));
}

/// Creates the device identity if needed and sets owner + network.
/// Returns this device's join payload (for the owner app), or "error: ...".
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_manwe_toolnet_Core_setup<'l>(
    mut env: JNIEnv<'l>,
    _: JClass<'l>,
    dir: JString<'l>,
    owner: JString<'l>,
    net: JString<'l>,
    name: JString<'l>,
) -> jstring {
    logs();
    let (dir, owner, net, name) = (string(&mut env, &dir), string(&mut env, &owner), string(&mut env, &net), string(&mut env, &name));
    let result = (|| -> Result<String> {
        let state = State::new(Some(PathBuf::from(&dir)))?;
        let owner = parse_pubkey(owner.trim())?;
        let net = net.trim();
        anyhow::ensure!(!net.is_empty() && !net.contains('.'), "network name can't be empty or contain dots");
        if state.read_secret("device.key")?.is_none() {
            state.write_secret("device.key", &Keys::generate().secret_key().to_secret_hex())?;
        }
        let relays = state.config().map(|c| c.relays).unwrap_or_else(|_| relays_or_default(vec![]));
        state.save_config(&Config { owner: owner.to_hex(), net: net.to_string(), relays })?;
        state.endpoint_key()?; // create it now
        Ok(join_payload(&state.device_keys()?.public_key().to_bech32()?, &name))
    })();
    let out = result.unwrap_or_else(|e| format!("error: {e:#}"));
    jstr(&mut env, &out)
}

/// This device's join payload if set up, "" if not
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_manwe_toolnet_Core_joinCode<'l>(
    mut env: JNIEnv<'l>,
    _: JClass<'l>,
    dir: JString<'l>,
    name: JString<'l>,
) -> jstring {
    let (dir, name) = (string(&mut env, &dir), string(&mut env, &name));
    let code = State::new(Some(PathBuf::from(dir)))
        .ok()
        .filter(|s| s.config().is_ok())
        .and_then(|s| s.device_keys().ok())
        .and_then(|k| k.public_key().to_bech32().ok())
        .map(|npub| join_payload(&npub, &name))
        .unwrap_or_default();
    jstr(&mut env, &code)
}

/// The QR code of `text` as rows of '1' (dark) and '0', joined by '\n'
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_manwe_toolnet_Core_qr<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, text: JString<'l>) -> jstring {
    let text = string(&mut env, &text);
    let rows = qrcode::QrCode::new(text.as_bytes())
        .map(|code| {
            let w = code.width();
            let colors = code.to_colors();
            colors
                .chunks(w)
                .map(|row| row.iter().map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' }).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    jstr(&mut env, &rows)
}

/// Starts the node in the background. Returns "" or an error message
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_manwe_toolnet_Core_start<'l>(
    mut env: JNIEnv<'l>,
    _: JClass<'l>,
    dir: JString<'l>,
    name: JString<'l>,
) -> jstring {
    logs();
    let (dir, name) = (string(&mut env, &dir), string(&mut env, &name));
    let mut running = RUNNING.lock().unwrap();
    if running.is_some() {
        return jstr(&mut env, "");
    }
    let result = (|| -> Result<()> {
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build()?;
        *SHARED.lock().unwrap() = Some(Shared { status: "starting".into(), ..Default::default() });
        runtime.spawn(async move {
            if let Err(e) = run(PathBuf::from(dir), name).await {
                warn!("toolnet stopped: {e:#}");
                set_shared(|s| s.error = Some(format!("{e:#}")));
            }
        });
        *running = Some(Running { runtime, engine: None });
        Ok(())
    })();
    let out = result.err().map(|e| format!("{e:#}")).unwrap_or_default();
    jstr(&mut env, &out)
}

async fn run(dir: PathBuf, name: String) -> Result<()> {
    let state = State::new(Some(dir))?;
    let node = Node::start(
        &state,
        NodeOpts { offline: false, bind: None, iroh_relays: vec![], relay_ca: None, advertise_routes: vec![], hostname: name },
    )
    .await
    .context("starting")?;
    if let Some(r) = RUNNING.lock().unwrap().as_mut() {
        r.engine = Some(node.engine.clone());
    }
    let me = node.me();
    let mut views = node.views.clone();
    let mut generation = 0u64;
    let mut wanted: Option<VpnWanted> = None;
    let mut teardown_at: Option<tokio::time::Instant> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        let grace = async {
            match teardown_at {
                Some(t) => tokio::time::sleep_until(t).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            r = views.changed() => r?,
            _ = tick.tick() => {}
            _ = grace => {
                teardown_at = None;
                warn!("not a member anymore: dropping peers and the VPN");
                node.engine.set_peers(vec![]).await;
                node.engine.stop_tun().await;
                wanted = None;
                set_shared(|s| { s.vpn = None; s.member = false; });
            }
        }
        let (status, view) = views.borrow().clone();
        let peers_json = peer_status(&node).await;
        let text = match &status {
            Status::NoNetwork => format!("waiting for network {} (owner {})", node.cfg.net, short(&node.id.owner.to_hex())),
            Status::NotMember { .. } => "waiting for approval".into(),
            Status::Member => format!("member of {}", node.cfg.net),
        };
        set_shared(|s| {
            s.status = text;
            s.peers = peers_json;
        });
        let (Status::Member, Some(view)) = (&status, &view) else {
            if wanted.is_some() && teardown_at.is_none() {
                teardown_at = Some(tokio::time::Instant::now() + MEMBERSHIP_GRACE);
            }
            continue;
        };
        teardown_at = None;
        let peers = peers_of(view, &me, None);
        node.engine.set_peers(peers).await;
        let Some(m) = view.config.members.get(&me.to_hex()) else { continue };
        let (_, prefix) = view.config.prefix()?;
        let routes: Vec<String> = node.engine.routed_prefixes().await.iter().filter(|c| c.len > 0).map(|c| c.to_string()).collect();
        let next = VpnWanted { generation, ip: m.ip.to_string(), prefix, routes };
        let changed = wanted.as_ref().is_none_or(|w| (w.ip.as_str(), w.prefix, &w.routes) != (next.ip.as_str(), next.prefix, &next.routes));
        if changed {
            generation += 1;
            let next = VpnWanted { generation, ..next };
            info!(ip = %next.ip, prefix, routes = ?next.routes, generation, "VPN wanted");
            wanted = Some(next.clone());
            set_shared(|s| {
                s.vpn = Some(next);
                s.member = true;
            });
        }
    }
}

fn short(s: &str) -> String {
    s.chars().take(10).collect()
}

async fn peer_status(node: &Node) -> Vec<serde_json::Value> {
    node.engine
        .status()
        .await
        .into_iter()
        .map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next().unwrap_or("").to_string();
            let ip = parts.next().unwrap_or("").to_string();
            let path: Vec<&str> = parts.collect();
            serde_json::json!({ "name": name, "ip": ip, "path": path.join(" ") })
        })
        .collect()
}

/// The VPN's TUN for VPN generation `generation`, from VpnService.establish()
/// (detached: the core owns and closes it)
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_manwe_toolnet_Core_setTun<'l>(
    _env: JNIEnv<'l>,
    _: JClass<'l>,
    fd: jint,
    generation: jint,
) -> jboolean {
    let running = RUNNING.lock().unwrap();
    let Some(Running { runtime, engine: Some(engine) }) = running.as_ref() else { return 0 };
    let engine = engine.clone();
    runtime.spawn(async move {
        engine.stop_tun().await;
        // SAFETY: Kotlin detached this fd from its ParcelFileDescriptor for us
        match unsafe { engine.start_tun_fd(fd) }.await {
            Ok(()) => info!(fd, generation, "VPN up"),
            Err(e) => warn!("VPN fd {fd}: {e:#}"),
        }
    });
    1
}

/// Status as JSON: { status, member, error, vpn: {generation, ip, prefix,
/// routes} | null, peers: [{name, ip, path}] }
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_manwe_toolnet_Core_status<'l>(mut env: JNIEnv<'l>, _: JClass<'l>) -> jstring {
    let running = RUNNING.lock().unwrap().is_some();
    let json = {
        let g = SHARED.lock().unwrap();
        let s = g.as_ref();
        serde_json::json!({
            "running": running,
            "status": s.map(|s| s.status.clone()).unwrap_or_else(|| "stopped".into()),
            "member": s.is_some_and(|s| s.member),
            "error": s.and_then(|s| s.error.clone()),
            "vpn": s.and_then(|s| s.vpn.clone()).map(|v| serde_json::json!({
                "generation": v.generation, "ip": v.ip, "prefix": v.prefix, "routes": v.routes,
            })),
            "peers": s.map(|s| s.peers.clone()).unwrap_or_default(),
        })
    };
    jstr(&mut env, &json.to_string())
}

/// Stops everything (the VPN's fd closes with the engine)
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_manwe_toolnet_Core_stop<'l>(_env: JNIEnv<'l>, _: JClass<'l>) {
    let running = RUNNING.lock().unwrap().take();
    if let Some(Running { runtime, engine }) = running {
        if let Some(engine) = engine {
            runtime.block_on(async {
                engine.stop_tun().await;
                engine.endpoint.close().await;
            });
        }
        runtime.shutdown_timeout(Duration::from_secs(2));
    }
    *SHARED.lock().unwrap() = None;
}
