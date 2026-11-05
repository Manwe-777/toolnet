//! toolnet-tray: a tray icon for the toolnet daemon (StatusNotifierItem, so
//! it shows in KDE, Waybar, quickshell, GNOME with the AppIndicator
//! extension...). It talks to the daemon over its control socket: status
//! every few seconds, connect / disconnect and the exit node on click.

use std::{path::PathBuf, process::Stdio, time::Duration};

use ksni::{MenuItem, TrayMethods, menu::*};
use tokio::{io::AsyncWriteExt, sync::mpsc};
use toolnet::ipc::{self, DaemonStatus, Request};

/// The owner app (approve devices, routes): the copy the daemon serves on
/// this machine, else this public one (its storage, where the owner key is
/// kept, is shared with other github.io sites). `$TOOLNET_OWNER_APP` overrides both
const OWNER_APP: &str = "https://manwe-777.github.io/toolnet/";
const POLL: Duration = Duration::from_secs(3);

enum Action {
    Send(Request),
    Copy(String),
    Open(String),
}

struct Tray {
    /// None: the daemon isn't reachable (the reason)
    status: Result<DaemonStatus, String>,
    tx: mpsc::UnboundedSender<Action>,
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Connected,
    /// Connected, traffic through an exit node
    Exit,
    Waiting,
    Off,
}

impl Tray {
    fn look(&self) -> Look {
        match &self.status {
            Ok(s) if s.state == "member" && s.connected && s.exit_active => Look::Exit,
            Ok(s) if s.state == "member" && s.connected => Look::Connected,
            Ok(s) if s.connected => Look::Waiting,
            _ => Look::Off,
        }
    }

    fn summary(&self) -> String {
        match &self.status {
            Err(e) => e.clone(),
            Ok(s) if !s.connected => "Disconnected".into(),
            Ok(s) => match (s.state.as_str(), &s.name, s.ip) {
                ("member", Some(n), Some(ip)) => format!("Connected as {n} · {ip}"),
                ("member", ..) => "Connected".into(),
                ("pending", ..) => "Waiting for the owner's approval".into(),
                _ => format!("Network \"{}\" not found", s.net),
            },
        }
    }

    fn send(&self, a: Action) {
        let _ = self.tx.send(a);
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "toolnet".into()
    }

    fn title(&self) -> String {
        "toolnet".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [22, 32, 48].into_iter().map(|size| icon(size, self.look())).collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let mut text = self.summary();
        if let Ok(s) = &self.status {
            if s.exit_active {
                text += &format!("\nExit node: {}", s.exit_node.as_deref().unwrap_or("?"));
            }
            let online = s.peers.iter().filter(|p| p.online).count();
            if s.state == "member" {
                text += &format!("\n{online} of {} devices online", s.peers.len());
            }
        }
        ksni::ToolTip { title: "toolnet".into(), description: text, ..Default::default() }
    }

    /// Left click: toggle the connection
    fn activate(&mut self, _x: i32, _y: i32) {
        if let Ok(s) = &self.status {
            self.send(Action::Send(if s.connected { Request::Disconnect } else { Request::Connect }));
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = vec![label(self.summary())];
        let Ok(s) = &self.status else {
            items.push(MenuItem::Separator);
            items.push(self.open_owner_app());
            items.push(quit());
            return items;
        };
        items.push(label(format!("Network {}{}", s.net, if s.domain.is_empty() { String::new() } else { format!(" · {}", s.domain) })));
        items.push(MenuItem::Separator);
        items.push(
            CheckmarkItem {
                label: "Connected".into(),
                checked: s.connected,
                activate: Box::new(|this: &mut Self| {
                    let connected = this.status.as_ref().is_ok_and(|s| s.connected);
                    this.send(Action::Send(if connected { Request::Disconnect } else { Request::Connect }));
                }),
                ..Default::default()
            }
            .into(),
        );
        if let Some(ip) = s.ip {
            items.push(copy_item(format!("Copy my IP ({ip})"), ip.to_string()));
        }

        if s.state == "member" {
            // Exit node: None + every usable one (plus the chosen one if it's
            // gone, so the choice stays visible)
            let mut names = s.exit_nodes.clone();
            if let Some(chosen) = &s.exit_node {
                if !names.contains(chosen) {
                    names.push(chosen.clone());
                }
            }
            let selected = s.exit_node.as_ref().and_then(|c| names.iter().position(|n| n == c)).map_or(0, |i| i + 1);
            let mut options = vec![RadioItem { label: "None".into(), ..Default::default() }];
            options.extend(names.iter().map(|n| RadioItem {
                label: if s.exit_nodes.contains(n) { n.clone() } else { format!("{n} (unavailable)") },
                enabled: s.connected,
                ..Default::default()
            }));
            let choices = names.clone();
            items.push(
                SubMenu {
                    label: match (&s.exit_node, s.exit_active) {
                        (Some(n), true) => format!("Exit node: {n}"),
                        (Some(n), false) => format!("Exit node: {n} (unavailable)"),
                        (None, _) => "Exit node: none".into(),
                    },
                    enabled: !names.is_empty() || s.exit_node.is_some(),
                    submenu: vec![
                        RadioGroup {
                            selected,
                            select: Box::new(move |this: &mut Self, i| {
                                let name = if i == 0 { None } else { choices.get(i - 1).cloned() };
                                this.send(Action::Send(Request::ExitNode { name }));
                            }),
                            options,
                            ..Default::default()
                        }
                        .into(),
                    ],
                    ..Default::default()
                }
                .into(),
            );

            items.push(MenuItem::Separator);
            let online = s.peers.iter().filter(|p| p.online).count();
            items.push(label(format!("Devices · {online} of {} online", s.peers.len())));
            for p in &s.peers {
                let state = match (&p.path, p.rtt_ms) {
                    (Some(path), Some(rtt)) => format!("{path}, {rtt} ms"),
                    _ if p.online => "online".into(),
                    _ => "offline".into(),
                };
                let mut sub = vec![
                    label(format!("{} · {state}", p.dns)),
                    copy_item(format!("Copy IP ({})", p.ip), p.ip.to_string()),
                    copy_item(format!("Copy name ({})", p.dns), p.dns.clone()),
                ];
                if !p.routes.is_empty() {
                    sub.push(label(format!("Routes {}", p.routes.join(", "))));
                }
                let mut name = format!("{} {}  {}", if p.online { "●" } else { "○" }, p.name, p.ip);
                if p.exit_node {
                    name += "  · exit node";
                }
                items.push(SubMenu { label: name, submenu: sub, ..Default::default() }.into());
            }
        }
        items.push(MenuItem::Separator);
        items.push(self.open_owner_app());
        items.push(quit());
        items
    }
}

fn label<T>(text: String) -> MenuItem<T> {
    StandardItem { label: text, enabled: false, ..Default::default() }.into()
}

fn copy_item(text: String, value: String) -> MenuItem<Tray> {
    StandardItem { label: text, activate: Box::new(move |this: &mut Tray| this.send(Action::Copy(value.clone()))), ..Default::default() }
        .into()
}

impl Tray {
    fn open_owner_app(&self) -> MenuItem<Tray> {
        let (label, url) = match (std::env::var("TOOLNET_OWNER_APP"), &self.status) {
            (Ok(url), _) => ("Open the owner app".into(), url),
            (_, Ok(DaemonStatus { owner_app: Some(port), .. })) => {
                ("Open the owner app".into(), format!("http://127.0.0.1:{port}/"))
            }
            _ => ("Open the owner app (online copy, not this machine's)".into(), OWNER_APP.into()),
        };
        StandardItem { label, activate: Box::new(move |this: &mut Tray| this.send(Action::Open(url.clone()))), ..Default::default() }
            .into()
    }
}

fn quit() -> MenuItem<Tray> {
    StandardItem { label: "Quit tray".into(), activate: Box::new(|_| std::process::exit(0)), ..Default::default() }.into()
}

/// The icon: three linked nodes on a disc, coloured by state. Drawn here so
/// there are no files to install (ARGB32, antialiased by supersampling)
fn icon(size: i32, look: Look) -> ksni::Icon {
    let (disc, fg): ([u8; 3], [u8; 3]) = match look {
        Look::Connected => ([0x2e, 0xa0, 0x43], [0xff, 0xff, 0xff]),
        Look::Exit => ([0x1f, 0x6f, 0xeb], [0xff, 0xff, 0xff]),
        Look::Waiting => ([0xd2, 0x99, 0x22], [0xff, 0xff, 0xff]),
        Look::Off => ([0x6e, 0x76, 0x81], [0xd0, 0xd7, 0xde]),
    };
    let n = size as f32;
    // Node positions in a unit square: a triangle
    let nodes = [(0.5, 0.27), (0.27, 0.68), (0.73, 0.68)];
    let edges = [(0, 1), (1, 2), (2, 0)];
    let seg = |px: f32, py: f32, (ax, ay): (f32, f32), (bx, by): (f32, f32)| {
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
    };
    const SS: usize = 4;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (mut a_disc, mut a_fg) = (0.0f32, 0.0f32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = (x as f32 + (sx as f32 + 0.5) / SS as f32) / n;
                    let py = (y as f32 + (sy as f32 + 0.5) / SS as f32) / n;
                    let in_disc = ((px - 0.5).powi(2) + (py - 0.5).powi(2)).sqrt() <= 0.48;
                    let on_node = nodes.iter().any(|&(nx, ny)| ((px - nx).powi(2) + (py - ny).powi(2)).sqrt() <= 0.11);
                    let on_edge = edges.iter().any(|&(a, b)| seg(px, py, nodes[a], nodes[b]) <= 0.035);
                    if in_disc {
                        a_disc += 1.0;
                        if on_node || on_edge {
                            a_fg += 1.0;
                        }
                    }
                }
            }
            let total = (SS * SS) as f32;
            let (a_disc, a_fg) = (a_disc / total, a_fg / total);
            let mix = if a_disc > 0.0 { a_fg / a_disc } else { 0.0 };
            let c = |i: usize| (disc[i] as f32 * (1.0 - mix) + fg[i] as f32 * mix) as u8;
            data.extend_from_slice(&[(a_disc * 255.0) as u8, c(0), c(1), c(2)]);
        }
    }
    ksni::Icon { width: size, height: size, data }
}

async fn fetch(socket: &PathBuf) -> Result<DaemonStatus, String> {
    match ipc::request(socket, &Request::Status).await {
        Ok(ipc::Response { status: Some(s), .. }) => Ok(s),
        Ok(r) => Err(r.error.unwrap_or_else(|| "no status".into())),
        Err(_) => Err("toolnet isn't running".into()),
    }
}

async fn copy(text: &str) {
    let candidates: &[(&str, &[&str])] = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        &[("wl-copy", &[])]
    } else {
        &[("xclip", &["-selection", "clipboard"]), ("xsel", &["--clipboard", "--input"])]
    };
    for (cmd, args) in candidates {
        let Ok(mut child) = tokio::process::Command::new(cmd).args(*args).stdin(Stdio::piped()).spawn() else { continue };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes()).await;
        }
        let _ = child.wait().await;
        return;
    }
    notify(&format!("No clipboard tool (wl-copy, xclip): {text}"));
}

fn notify(text: &str) {
    let _ = std::process::Command::new("notify-send").args(["-a", "toolnet", "toolnet", text]).spawn();
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let socket = ipc::socket_path();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let tray = Tray { status: fetch(&socket).await, tx };
    let handle = tray.spawn().await.map_err(|e| anyhow::anyhow!("no tray host on this desktop? {e}"))?;
    let mut poll = tokio::time::interval(POLL);
    loop {
        tokio::select! {
            _ = poll.tick() => {}
            Some(action) = rx.recv() => match action {
                Action::Copy(text) => {
                    copy(&text).await;
                    continue;
                }
                Action::Open(url) => {
                    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
                    continue;
                }
                Action::Send(req) => match ipc::request(&socket, &req).await {
                    Ok(r) if !r.ok => notify(r.error.as_deref().unwrap_or("request failed")),
                    Err(e) => notify(&format!("{e:#}")),
                    Ok(_) => {}
                },
            },
        }
        let status = fetch(&socket).await;
        if handle.update(|t: &mut Tray| t.status = status).await.is_none() {
            break; // the tray service stopped
        }
    }
    Ok(())
}
