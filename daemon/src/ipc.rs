//! The daemon's local control socket: one JSON request per line, one JSON
//! response per line. Used by the CLI (`toolnet status`, `connect`, ...) and
//! the tray. Anyone on the machine may read the status; changing anything
//! needs root, the daemon's own user, or the configured operator.

use std::{net::Ipv4Addr, path::{Path, PathBuf}, time::Duration};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{mpsc, oneshot, watch},
};
use tracing::{debug, info, warn};

use crate::engine::Engine;

pub const DEFAULT_SOCKET: &str = "/run/toolnet/toolnet.sock";

/// `$TOOLNET_SOCKET`, or the system service's socket
pub fn socket_path() -> PathBuf {
    std::env::var_os("TOOLNET_SOCKET").map(PathBuf::from).unwrap_or_else(|| DEFAULT_SOCKET.into())
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Status,
    /// Bring the tunnel up (the service keeps following the network either way)
    Connect,
    /// Take the tunnel down: no TUN, routes, DNS or peers
    Disconnect,
    /// Use this member as exit node (None: no exit node)
    ExitNode { name: Option<String> },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<DaemonStatus>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct DaemonStatus {
    pub version: String,
    /// "member", "pending" (not approved, or removed) or "no network"
    pub state: String,
    /// Whether the tunnel should be up (the user's choice)
    pub connected: bool,
    pub net: String,
    pub domain: String,
    pub device: String,
    /// Our member name and tailnet IP, once approved
    pub name: Option<String>,
    pub ip: Option<Ipv4Addr>,
    /// The chosen exit node, and whether traffic goes through it right now
    pub exit_node: Option<String>,
    pub exit_active: bool,
    /// Members usable as exit nodes (allowed by the owner and offered)
    pub exit_nodes: Vec<String>,
    pub peers: Vec<PeerStatus>,
    /// The port of the owner app this daemon serves on 127.0.0.1, if any
    #[serde(default)]
    pub owner_app: Option<u16>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PeerStatus {
    pub name: String,
    pub ip: Ipv4Addr,
    /// `<name>.<domain>`
    pub dns: String,
    /// Heard from in the last 15 minutes (or connected right now)
    pub online: bool,
    /// "direct" or "relay" while connected
    pub path: Option<String>,
    pub rtt_ms: Option<u64>,
    pub exit_node: bool,
    /// Subnets it routes for us (approved and offered)
    pub routes: Vec<String>,
}

/// A command for the daemon's main loop, with where to send the answer
pub type Command = (Request, oneshot::Sender<Result<(), String>>);

/// Serves the socket. Status comes from `status` plus live path info from
/// the engine; other requests go to `commands` (from allowed users only)
pub async fn serve(
    path: PathBuf,
    operator: Option<u32>,
    engine: Engine,
    status: watch::Receiver<DaemonStatus>,
    commands: mpsc::Sender<Command>,
) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
    // Readable and writable by all: permissions are checked per request
    // from the peer's credentials
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))?;
    info!(socket = %path.display(), "control socket");
    let me = unsafe { libc::geteuid() };
    loop {
        let (stream, _) = listener.accept().await?;
        let uid = stream.peer_cred().map(|c| c.uid()).ok();
        let may_change = uid.is_some_and(|u| u == 0 || u == me || Some(u) == operator);
        let (engine, status, commands) = (engine.clone(), status.clone(), commands.clone());
        tokio::spawn(async move {
            if let Err(e) = handle(stream, may_change, engine, status, commands).await {
                debug!("control connection: {e:#}");
            }
        });
    }
}

async fn handle(
    stream: UnixStream,
    may_change: bool,
    engine: Engine,
    status: watch::Receiver<DaemonStatus>,
    commands: mpsc::Sender<Command>,
) -> Result<()> {
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    while let Some(line) = lines.next_line().await? {
        let resp = match serde_json::from_str::<Request>(&line) {
            Err(e) => Response { ok: false, error: Some(format!("bad request: {e}")), status: None },
            Ok(Request::Status) => {
                Response { ok: true, error: None, status: Some(live_status(&engine, &status).await) }
            }
            Ok(_) if !may_change => Response {
                ok: false,
                error: Some("not allowed: run as root, or as the operator (`toolnet up --operator <user>`)".into()),
                status: None,
            },
            Ok(req) => {
                let (tx, rx) = oneshot::channel();
                commands.send((req, tx)).await.context("daemon stopping")?;
                match rx.await.context("daemon stopping")? {
                    Ok(()) => {
                        // Let the main loop apply it before reporting
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        Response { ok: true, error: None, status: Some(live_status(&engine, &status).await) }
                    }
                    Err(e) => Response { ok: false, error: Some(e), status: None },
                }
            }
        };
        let mut out = serde_json::to_vec(&resp)?;
        out.push(b'\n');
        w.write_all(&out).await?;
    }
    Ok(())
}

async fn live_status(engine: &Engine, status: &watch::Receiver<DaemonStatus>) -> DaemonStatus {
    let mut s = status.borrow().clone();
    let paths = engine.paths().await;
    for p in &mut s.peers {
        if let Some((direct, rtt)) = paths.get(&p.name) {
            p.path = Some(if *direct { "direct" } else { "relay" }.into());
            p.rtt_ms = Some(rtt.as_millis() as u64);
            p.online = true;
        }
    }
    s
}

/// Sends one request to the daemon
pub async fn request(path: &Path, req: &Request) -> Result<Response> {
    let stream = match UnixStream::connect(path).await {
        Ok(s) => s,
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {
            bail!("toolnet isn't running (no control socket at {})", path.display())
        }
        Err(e) => return Err(e).with_context(|| format!("connecting to {}", path.display())),
    };
    let (r, mut w) = stream.into_split();
    let mut line = serde_json::to_vec(req)?;
    line.push(b'\n');
    w.write_all(&line).await?;
    let resp = BufReader::new(r).lines().next_line().await?.context("no answer from the daemon")?;
    Ok(serde_json::from_str(&resp)?)
}

/// The user's choices, kept across restarts (state dir, prefs.json)
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Prefs {
    pub connected: bool,
    pub exit_node: Option<String>,
}

impl Prefs {
    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    pub fn save(&self, path: &Path) {
        if let Err(e) = std::fs::write(path, serde_json::to_vec_pretty(self).unwrap()) {
            warn!("saving {}: {e}", path.display());
        }
    }
}
