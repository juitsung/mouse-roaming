use crate::clipboard_sync::ClipboardSync;
use crate::config::{AppConfig, AppRole};
use crate::router::EdgeRouter;
use anyhow::Result;
use mouseroaming_net::{
    connect_tls, format_short_fingerprint, load_or_generate_identity, DiscoveredPeer,
    DiscoveryEvent, MdnsService, MessageConnection, NodeIdentity, TlsServerListener, TrustPolicy,
};
use mouseroaming_os::{CaptureControl, InputEvent, InputInjector, OsBackend, PermissionStatus};
use mouseroaming_proto::{
    Edge, KeyCode, Message, ModifierMapper, MouseButton, OsKind, ScreenRect, PROTOCOL_VERSION,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

const MAX_LOG_LINES: usize = 200;

#[derive(Debug, Clone)]
pub struct ConnectedPeerInfo {
    pub peer_id: String,
    pub hostname: String,
    pub os: OsKind,
    pub addr: String,
    pub edge: Option<Edge>,
    pub screen: ScreenRect,
}

impl ConnectedPeerInfo {
    /// Returns the clean IP address portion of `addr` (without ephemeral/TCP port).
    pub fn ip_address(&self) -> String {
        extract_ip_from_addr(&self.addr)
    }
}

pub fn extract_ip_from_addr(addr: &str) -> String {
    let trimmed = addr.trim();
    if let Ok(sa) = trimmed.parse::<SocketAddr>() {
        return sa.ip().to_string();
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return ip.to_string();
    }
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return rest[..end].to_string();
        }
    }
    if let Some((host, _port)) = trimmed.rsplit_once(':') {
        return host.to_string();
    }
    trimmed.to_string()
}

pub fn detect_local_ip() -> String {
    for target in [
        "8.8.8.8:80",
        "1.1.1.1:80",
        "224.0.0.251:5353",
        "192.168.1.1:80",
        "10.0.0.1:80",
    ] {
        if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
            if sock.connect(target).is_ok() {
                if let Ok(local) = sock.local_addr() {
                    let ip = local.ip();
                    if !ip.is_unspecified() && !ip.is_loopback() {
                        return ip.to_string();
                    }
                }
            }
        }
    }
    "127.0.0.1".to_string()
}

#[derive(Debug, Clone)]
pub struct EngineSnapshot {
    pub running: bool,
    pub paused: bool,
    pub role: AppRole,
    pub local_peer_id: String,
    pub local_hostname: String,
    pub local_ip: String,
    pub local_os: OsKind,
    pub local_fingerprint_short: String,
    pub local_fingerprint_full: String,
    pub local_screen: ScreenRect,
    pub listen_addr: Option<String>,
    pub active_screen_label: String,
    pub connected_peers: Vec<ConnectedPeerInfo>,
    pub discovered_peers: Vec<DiscoveredPeer>,
    pub permission: PermissionStatus,
    pub logs: VecDeque<String>,
}

#[derive(Debug, Clone)]
pub enum EngineCommand {
    Start,
    Stop,
    SetPaused(bool),
    UpdateConfig(AppConfig),
    BindPeerToEdge {
        peer_id: String,
        edge: Edge,
    },
    UnbindEdge(Edge),
    ConnectToPeer {
        addr: String,
        expected_fingerprint: Option<String>,
    },
    ManualSwitch(Option<Edge>),
    TrustFingerprint {
        key: String,
        fingerprint: String,
    },
    Shutdown,
}

pub struct EngineOptions {
    pub config_dir: PathBuf,
    pub enable_mdns: bool,
    pub bind_ip: IpAddr,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            config_dir: AppConfig::default_config_dir(),
            enable_mdns: true,
            bind_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        }
    }
}

#[derive(Clone)]
pub struct EngineHandle {
    cmd_tx: mpsc::UnboundedSender<EngineCommand>,
    snapshot: Arc<RwLock<EngineSnapshot>>,
    config: Arc<RwLock<AppConfig>>,
}

impl EngineHandle {
    pub fn send(&self, cmd: EngineCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        self.snapshot.read().unwrap().clone()
    }

    pub fn config(&self) -> AppConfig {
        self.config.read().unwrap().clone()
    }

    pub fn shutdown(&self) {
        let _ = self.cmd_tx.send(EngineCommand::Shutdown);
    }
}

pub struct MouseRoamingEngine;

impl MouseRoamingEngine {
    pub fn spawn(
        rt: &tokio::runtime::Handle,
        backend: Arc<dyn OsBackend>,
        config: AppConfig,
        options: EngineOptions,
    ) -> Result<EngineHandle> {
        let identity = load_or_generate_identity(&options.config_dir, &config.hostname)?;
        let local_screen = backend
            .injector()
            .virtual_screen()
            .unwrap_or_else(|_| ScreenRect::new(0, 0, 1920, 1080));
        let permission = backend.check_permissions();

        let local_ip = if !options.bind_ip.is_unspecified() {
            options.bind_ip.to_string()
        } else {
            detect_local_ip()
        };

        let initial_snapshot = EngineSnapshot {
            running: false,
            paused: false,
            role: config.role,
            local_peer_id: config.peer_id.clone(),
            local_hostname: config.hostname.clone(),
            local_ip,
            local_os: backend.os_kind(),
            local_fingerprint_short: identity.short_fingerprint(),
            local_fingerprint_full: identity.fingerprint.clone(),
            local_screen,
            listen_addr: None,
            active_screen_label: "本機 (Local)".into(),
            connected_peers: Vec::new(),
            discovered_peers: Vec::new(),
            permission,
            logs: VecDeque::new(),
        };

        let snapshot = Arc::new(RwLock::new(initial_snapshot));
        let shared_config = Arc::new(RwLock::new(config.clone()));
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

        let snap_clone = snapshot.clone();
        let cfg_clone = shared_config.clone();
        let handle = EngineHandle {
            cmd_tx: cmd_tx.clone(),
            snapshot,
            config: shared_config,
        };

        rt.spawn(async move {
            supervisor_loop(backend, identity, cfg_clone, snap_clone, cmd_rx, options).await;
        });

        if config.auto_start_service {
            let _ = cmd_tx.send(EngineCommand::Start);
        }

        Ok(handle)
    }
}

fn push_log(snapshot: &Arc<RwLock<EngineSnapshot>>, msg: impl Into<String>) {
    let text = msg.into();
    info!("{text}");
    if let Ok(mut s) = snapshot.write() {
        if s.logs.len() >= MAX_LOG_LINES {
            s.logs.pop_front();
        }
        s.logs.push_back(text);
    }
}

async fn supervisor_loop(
    backend: Arc<dyn OsBackend>,
    identity: NodeIdentity,
    shared_config: Arc<RwLock<AppConfig>>,
    snapshot: Arc<RwLock<EngineSnapshot>>,
    mut cmd_rx: mpsc::UnboundedReceiver<EngineCommand>,
    options: EngineOptions,
) {
    // Start mDNS discovery service if enabled
    let mut mdns = if options.enable_mdns {
        match MdnsService::new() {
            Ok(m) => Some(m),
            Err(e) => {
                push_log(&snapshot, format!("mDNS 初始化失敗（仍可手動 IP 連線）: {e}"));
                None
            }
        }
    } else {
        None
    };

    let mut discovery_rx = if let Some(ref m) = mdns {
        let local_id = shared_config.read().unwrap().peer_id.clone();
        match m.start_browse(local_id) {
            Ok(rx) => Some(rx),
            Err(e) => {
                push_log(&snapshot, format!("mDNS 探索啟動失敗: {e}"));
                None
            }
        }
    } else {
        None
    };

    let mut active_stop_tx: Option<watch::Sender<bool>> = None;
    let mut active_worker_cmd_tx: Option<mpsc::UnboundedSender<WorkerControl>> = None;
    let mut discovered_map: HashMap<String, DiscoveredPeer> = HashMap::new();

    push_log(
        &snapshot,
        format!(
            "MouseRoaming 已初始化 ({}, 指紋: {})",
            backend.os_kind().display_name(),
            identity.short_fingerprint()
        ),
    );

    loop {
        tokio::select! {
            Some(event) = async {
                match &mut discovery_rx {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match event {
                    DiscoveryEvent::Found(peer) => {
                        discovered_map.insert(peer.full_name.clone(), peer);
                    }
                    DiscoveryEvent::Removed(fullname) => {
                        discovered_map.remove(&fullname);
                    }
                }
                if let Ok(mut s) = snapshot.write() {
                    let mut list: Vec<_> = discovered_map.values().cloned().collect();
                    list.sort_by(|a, b| a.hostname.cmp(&b.hostname));
                    s.discovered_peers = list;
                }
            }
            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    EngineCommand::Shutdown => {
                        if let Some(tx) = active_stop_tx.take() {
                            let _ = tx.send(true);
                        }
                        if let Some(m) = mdns.take() {
                            m.shutdown();
                        }
                        break;
                    }
                    EngineCommand::Stop => {
                        if let Some(tx) = active_stop_tx.take() {
                            let _ = tx.send(true);
                        }
                        active_worker_cmd_tx = None;
                        if let Some(ref mut m) = mdns {
                            m.stop_advertising();
                        }
                        if let Ok(mut s) = snapshot.write() {
                            s.running = false;
                            s.listen_addr = None;
                            s.connected_peers.clear();
                            s.active_screen_label = "本機 (Local)".into();
                        }
                        push_log(&snapshot, "服務已停止");
                    }
                    EngineCommand::Start => {
                        if let Some(tx) = active_stop_tx.take() {
                            let _ = tx.send(true);
                        }
                        let cfg = shared_config.read().unwrap().clone();
                        let (stop_tx, stop_rx) = watch::channel(false);
                        let (w_tx, w_rx) = mpsc::unbounded_channel();
                        active_stop_tx = Some(stop_tx);
                        active_worker_cmd_tx = Some(w_tx);

                        if let Ok(mut s) = snapshot.write() {
                            s.running = true;
                            s.role = cfg.role;
                            s.local_ip = if !options.bind_ip.is_unspecified() {
                                options.bind_ip.to_string()
                            } else {
                                detect_local_ip()
                            };
                            s.connected_peers.clear();
                            s.active_screen_label = "本機 (Local)".into();
                        }

                        match cfg.role {
                            AppRole::Server => {
                                if let Some(ref mut m) = mdns {
                                    if let Err(e) = m.advertise(
                                        &cfg.peer_id,
                                        &cfg.hostname,
                                        backend.os_kind(),
                                        cfg.listen_port,
                                        &identity.fingerprint,
                                    ) {
                                        push_log(&snapshot, format!("mDNS 廣播警告: {e}"));
                                    }
                                }
                                let b = backend.clone();
                                let id = identity.clone();
                                let snap = snapshot.clone();
                                let scfg = shared_config.clone();
                                let bind_ip = options.bind_ip;
                                let cfg_dir = options.config_dir.clone();
                                tokio::spawn(async move {
                                    run_server_worker(b, id, scfg, cfg_dir, snap, bind_ip, stop_rx, w_rx).await;
                                });
                            }
                            AppRole::Client => {
                                if let Some(ref mut m) = mdns {
                                    m.stop_advertising();
                                }
                                let b = backend.clone();
                                let snap = snapshot.clone();
                                let scfg = shared_config.clone();
                                let cfg_dir = options.config_dir.clone();
                                tokio::spawn(async move {
                                    run_client_worker(b, scfg, cfg_dir, snap, stop_rx, w_rx).await;
                                });
                            }
                        }
                    }
                    EngineCommand::SetPaused(paused) => {
                        if let Ok(mut s) = snapshot.write() {
                            s.paused = paused;
                        }
                        if let Some(ref tx) = active_worker_cmd_tx {
                            let _ = tx.send(WorkerControl::SetPaused(paused));
                        }
                        push_log(&snapshot, if paused { "已暫停跨機切換" } else { "已恢復跨機切換" });
                    }
                    EngineCommand::UpdateConfig(new_cfg) => {
                        let role_changed = {
                            let mut cur = shared_config.write().unwrap();
                            let changed = cur.role != new_cfg.role
                                || cur.listen_port != new_cfg.listen_port
                                || cur.connect_address != new_cfg.connect_address;
                            *cur = new_cfg.clone();
                            changed
                        };
                        let _ = new_cfg.save(&options.config_dir);
                        if let Ok(mut s) = snapshot.write() {
                            s.role = new_cfg.role;
                            s.local_hostname = new_cfg.hostname.clone();
                        }
                        if let Some(ref tx) = active_worker_cmd_tx {
                            let _ = tx.send(WorkerControl::ReloadConfig(new_cfg));
                        }
                        if role_changed && active_stop_tx.is_some() {
                            push_log(&snapshot, "連線角色或位址已變更，正在重新啟動服務...");
                            // Trigger restart inline
                            if let Some(tx) = active_stop_tx.take() {
                                let _ = tx.send(true);
                            }
                            let cfg = shared_config.read().unwrap().clone();
                            let (stop_tx, stop_rx) = watch::channel(false);
                            let (w_tx, w_rx) = mpsc::unbounded_channel();
                            active_stop_tx = Some(stop_tx);
                            active_worker_cmd_tx = Some(w_tx);
                            match cfg.role {
                                AppRole::Server => {
                                    if let Some(ref mut m) = mdns {
                                        let _ = m.advertise(
                                            &cfg.peer_id,
                                            &cfg.hostname,
                                            backend.os_kind(),
                                            cfg.listen_port,
                                            &identity.fingerprint,
                                        );
                                    }
                                    let b = backend.clone();
                                    let id = identity.clone();
                                    let snap = snapshot.clone();
                                    let scfg = shared_config.clone();
                                    let bind_ip = options.bind_ip;
                                    let cfg_dir = options.config_dir.clone();
                                    tokio::spawn(async move {
                                        run_server_worker(b, id, scfg, cfg_dir, snap, bind_ip, stop_rx, w_rx).await;
                                    });
                                }
                                AppRole::Client => {
                                    if let Some(ref mut m) = mdns {
                                        m.stop_advertising();
                                    }
                                    let b = backend.clone();
                                    let snap = snapshot.clone();
                                    let scfg = shared_config.clone();
                                    let cfg_dir = options.config_dir.clone();
                                    tokio::spawn(async move {
                                        run_client_worker(b, scfg, cfg_dir, snap, stop_rx, w_rx).await;
                                    });
                                }
                            }
                        }
                    }
                    EngineCommand::BindPeerToEdge { peer_id, edge } => {
                        {
                            let mut cfg = shared_config.write().unwrap();
                            let old_edge = cfg
                                .edge_bindings
                                .iter()
                                .find_map(|(&e, v)| if v == &peer_id { Some(e) } else { None });
                            if old_edge != Some(edge) {
                                let displaced = cfg.edge_bindings.remove(&edge);
                                cfg.edge_bindings.retain(|_, v| v != &peer_id);
                                cfg.edge_bindings.insert(edge, peer_id.clone());
                                if let (Some(prev), Some(other)) = (old_edge, displaced) {
                                    cfg.edge_bindings.insert(prev, other);
                                }
                                let _ = cfg.save(&options.config_dir);
                            }
                        }
                        if let Some(ref tx) = active_worker_cmd_tx {
                            let _ = tx.send(WorkerControl::BindEdge { peer_id, edge });
                        }
                    }
                    EngineCommand::UnbindEdge(edge) => {
                        {
                            let mut cfg = shared_config.write().unwrap();
                            cfg.edge_bindings.remove(&edge);
                            let _ = cfg.save(&options.config_dir);
                        }
                        if let Some(ref tx) = active_worker_cmd_tx {
                            let _ = tx.send(WorkerControl::UnbindEdge(edge));
                        }
                    }
                    EngineCommand::ConnectToPeer { addr, expected_fingerprint } => {
                        {
                            let mut cfg = shared_config.write().unwrap();
                            cfg.role = AppRole::Client;
                            cfg.connect_address = addr.clone();
                            if let Some(fp) = expected_fingerprint {
                                if !fp.is_empty() {
                                    cfg.trusted_fingerprints.insert(addr.clone(), fp);
                                }
                            }
                            let _ = cfg.save(&options.config_dir);
                        }
                        if let Some(tx) = active_stop_tx.take() {
                            let _ = tx.send(true);
                        }
                        if let Some(ref mut m) = mdns {
                            m.stop_advertising();
                        }
                        let (stop_tx, stop_rx) = watch::channel(false);
                        let (w_tx, w_rx) = mpsc::unbounded_channel();
                        active_stop_tx = Some(stop_tx);
                        active_worker_cmd_tx = Some(w_tx);
                        if let Ok(mut s) = snapshot.write() {
                            s.running = true;
                            s.role = AppRole::Client;
                        }
                        let b = backend.clone();
                        let snap = snapshot.clone();
                        let scfg = shared_config.clone();
                        let cfg_dir = options.config_dir.clone();
                        tokio::spawn(async move {
                            run_client_worker(b, scfg, cfg_dir, snap, stop_rx, w_rx).await;
                        });
                    }
                    EngineCommand::ManualSwitch(edge) => {
                        if let Some(ref tx) = active_worker_cmd_tx {
                            let _ = tx.send(WorkerControl::ManualSwitch(edge));
                        }
                    }
                    EngineCommand::TrustFingerprint { key, fingerprint } => {
                        let mut cfg = shared_config.write().unwrap();
                        cfg.trusted_fingerprints.insert(key, fingerprint);
                        let _ = cfg.save(&options.config_dir);
                    }
                }
            }
            else => break,
        }
    }
}

enum WorkerControl {
    SetPaused(bool),
    ReloadConfig(AppConfig),
    BindEdge { peer_id: String, edge: Edge },
    UnbindEdge(Edge),
    ManualSwitch(Option<Edge>),
}

struct ServerPeer {
    peer_id: String,
    hostname: String,
    os: OsKind,
    addr: String,
    screen: ScreenRect,
    tx: mpsc::UnboundedSender<Message>,
}

enum ServerInbound {
    Connected {
        peer: ServerPeer,
    },
    Message {
        peer_id: String,
        msg: Message,
    },
    Disconnected {
        peer_id: String,
    },
}

fn update_server_snapshot(
    snapshot: &Arc<RwLock<EngineSnapshot>>,
    peers: &HashMap<String, ServerPeer>,
    router: &EdgeRouter,
) {
    if let Ok(mut s) = snapshot.write() {
        s.connected_peers = peers
            .values()
            .map(|p| ConnectedPeerInfo {
                peer_id: p.peer_id.clone(),
                hostname: p.hostname.clone(),
                os: p.os,
                addr: p.addr.clone(),
                edge: router.edge_for_peer(&p.peer_id),
                screen: p.screen,
            })
            .collect();
        s.connected_peers.sort_by(|a, b| a.hostname.cmp(&b.hostname));
        s.active_screen_label = match router.active_remote() {
            None => "本機 (Local)".into(),
            Some(active) => peers
                .get(&active.peer_id)
                .map(|p| format!("{} ({})", p.hostname, p.os.display_name()))
                .unwrap_or_else(|| active.peer_id.clone()),
        };
    }
}

async fn run_server_worker(
    backend: Arc<dyn OsBackend>,
    identity: NodeIdentity,
    shared_config: Arc<RwLock<AppConfig>>,
    _cfg_dir: PathBuf,
    snapshot: Arc<RwLock<EngineSnapshot>>,
    bind_ip: IpAddr,
    mut stop_rx: watch::Receiver<bool>,
    mut ctrl_rx: mpsc::UnboundedReceiver<WorkerControl>,
) {
    let mut cfg = shared_config.read().unwrap().clone();
    let bind_addr = SocketAddr::new(bind_ip, cfg.listen_port);
    let listener = match TlsServerListener::bind(bind_addr, &identity).await {
        Ok(l) => l,
        Err(e) => {
            push_log(&snapshot, format!("無法啟動 TLS 伺服器於 {bind_addr}: {e}"));
            if let Ok(mut s) = snapshot.write() {
                s.running = false;
            }
            return;
        }
    };

    let actual_addr = listener.local_addr().unwrap_or(bind_addr);
    if let Ok(mut s) = snapshot.write() {
        s.listen_addr = Some(actual_addr.to_string());
    }
    push_log(
        &snapshot,
        format!(
            "Server 已監聽 TLS 1.3 於 {actual_addr}（指紋: {}）",
            identity.short_fingerprint()
        ),
    );

    let local_screen = backend
        .injector()
        .virtual_screen()
        .unwrap_or_else(|_| ScreenRect::new(0, 0, 1920, 1080));
    let (cx, cy) = local_screen.center();
    let capture_control = Arc::new(CaptureControl::new(cx, cy));

    let (mut input_rx, _capture_guard) = match backend.start_capture(capture_control.clone()) {
        Ok(pair) => pair,
        Err(e) => {
            push_log(&snapshot, format!("無法啟動系統輸入擷取: {e}"));
            if let Ok(mut s) = snapshot.write() {
                s.running = false;
            }
            return;
        }
    };

    let mut router = EdgeRouter::new(local_screen, cfg.dead_corner_px, cfg.edge_switch_cooldown_ms);
    for (&edge, pid) in &cfg.edge_bindings {
        router.bind_edge(edge, pid.clone());
    }
    let mut mapper = ModifierMapper::new(cfg.key_mapping.clone());
    let mut clipboard_sync = ClipboardSync::new();
    clipboard_sync.seed(backend.clipboard().as_ref());

    let (inbound_tx, mut inbound_rx) = mpsc::unbounded_channel::<ServerInbound>();

    // Spawn TLS accept loop
    let accept_tx = inbound_tx.clone();
    let accept_cfg = shared_config.clone();
    let accept_snap = snapshot.clone();
    let accept_os = backend.os_kind();
    let mut accept_stop = stop_rx.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = accept_stop.changed() => {
                    if *accept_stop.borrow() {
                        break;
                    }
                }
                res = listener.accept() => {
                    match res {
                        Ok((conn, peer_addr)) => {
                            let tx = accept_tx.clone();
                            let scfg = accept_cfg.clone();
                            let snap = accept_snap.clone();
                            tokio::spawn(async move {
                                handle_server_client_conn(conn, peer_addr, scfg, accept_os, local_screen, snap, tx).await;
                            });
                        }
                        Err(e) => {
                            warn!("TLS accept error: {e}");
                        }
                    }
                }
            }
        }
    });

    let mut peers: HashMap<String, ServerPeer> = HashMap::new();
    let mut paused = false;
    let mut clip_interval = tokio::time::interval(Duration::from_millis(350));
    let mut ping_interval = tokio::time::interval(Duration::from_secs(5));
    let mut ping_seq: u64 = 0;

    loop {
        tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    break;
                }
            }
            Some(ctrl) = ctrl_rx.recv() => {
                match ctrl {
                    WorkerControl::SetPaused(p) => {
                        paused = p;
                        if paused && router.is_remote_active() {
                            return_server_to_local(backend.as_ref(), &mut router, &capture_control, &peers, &snapshot);
                        }
                    }
                    WorkerControl::ReloadConfig(new_cfg) => {
                        cfg = new_cfg;
                        router.set_dead_corner_px(cfg.dead_corner_px);
                        router.set_cooldown_ms(cfg.edge_switch_cooldown_ms);
                        mapper = ModifierMapper::new(cfg.key_mapping.clone());
                    }
                    WorkerControl::BindEdge { peer_id, edge } => {
                        router.move_or_swap_peer_to_edge(peer_id, edge);
                        update_server_snapshot(&snapshot, &peers, &router);
                    }
                    WorkerControl::UnbindEdge(edge) => {
                        router.unbind_edge(edge);
                        update_server_snapshot(&snapshot, &peers, &router);
                    }
                    WorkerControl::ManualSwitch(target_edge) => {
                        match target_edge {
                            None => {
                                return_server_to_local(backend.as_ref(), &mut router, &capture_control, &peers, &snapshot);
                            }
                            Some(edge) => {
                                if let Some(pid) = router.peer_on_edge(edge).map(str::to_string) {
                                    if let Some(peer) = peers.get(&pid) {
                                        let (ax, ay) = router.local_screen().center();
                                        capture_control.set_anchor(ax, ay);
                                        capture_control.set_grabbed(true);
                                        backend.set_cursor_visible(false);
                                        router.enter_remote(pid.clone(), edge, peer.os, Instant::now());
                                        let _ = peer.tx.send(Message::Enter {
                                            edge: edge.opposite(),
                                            ratio: 0.5,
                                        });
                                        update_server_snapshot(&snapshot, &peers, &router);
                                        push_log(&snapshot, format!("手動切換至 {}", peer.hostname));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Some(inbound) = inbound_rx.recv() => {
                match inbound {
                    ServerInbound::Connected { peer } => {
                        let pid = peer.peer_id.clone();
                        let hostname = peer.hostname.clone();
                        let os_name = peer.os.display_name();
                        // Ensure peer is bound to an edge so it works immediately
                        if router.edge_for_peer(&pid).is_none() {
                            let preferred = cfg.default_client_edge;
                            let chosen = if router.peer_on_edge(preferred).is_none() {
                                preferred
                            } else {
                                Edge::all()
                                    .into_iter()
                                    .find(|e| router.peer_on_edge(*e).is_none())
                                    .unwrap_or(preferred)
                            };
                            router.bind_edge(chosen, pid.clone());
                            push_log(
                                &snapshot,
                                format!("裝置 {hostname} ({os_name}) 已連線並自動指派至螢幕「{}」側", chosen.label()),
                            );
                        } else {
                            push_log(&snapshot, format!("裝置 {hostname} ({os_name}) 已連線"));
                        }
                        peers.insert(pid, peer);
                        update_server_snapshot(&snapshot, &peers, &router);
                    }
                    ServerInbound::Disconnected { peer_id } => {
                        if let Some(p) = peers.remove(&peer_id) {
                            push_log(&snapshot, format!("裝置 {} 已斷線", p.hostname));
                        }
                        if router.active_remote().map(|a| a.peer_id.as_str()) == Some(peer_id.as_str()) {
                            return_server_to_local(backend.as_ref(), &mut router, &capture_control, &peers, &snapshot);
                            push_log(&snapshot, "遠端裝置斷線，游標已自動返回本機");
                        }
                        update_server_snapshot(&snapshot, &peers, &router);
                    }
                    ServerInbound::Message { peer_id, msg } => {
                        match msg {
                            Message::Leave { edge, ratio } => {
                                if router.active_remote().map(|a| a.peer_id.as_str()) == Some(peer_id.as_str()) {
                                    let target_edge = edge.opposite();
                                    return_server_to_local(backend.as_ref(), &mut router, &capture_control, &peers, &snapshot);
                                    let (lx, ly) = router.local_screen().entry_point(target_edge, ratio);
                                    let _ = backend.injector().move_cursor_abs(lx, ly);
                                    debug!("Cursor returned to local screen at ({lx}, {ly})");
                                }
                            }
                            Message::ClipboardText { text, .. } => {
                                if cfg.clipboard_sync_enabled {
                                    if let Ok(true) = clipboard_sync.apply_remote(backend.clipboard().as_ref(), &text) {
                                        debug!("Applied remote clipboard text ({} bytes)", text.len());
                                        // Rebroadcast to other connected clients
                                        for (other_id, other_peer) in &peers {
                                            if other_id != &peer_id {
                                                let _ = other_peer.tx.send(Message::ClipboardText {
                                                    seq: 0,
                                                    text: text.clone(),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                            Message::ScreenChanged { screen } => {
                                if let Some(p) = peers.get_mut(&peer_id) {
                                    p.screen = screen;
                                }
                                update_server_snapshot(&snapshot, &peers, &router);
                            }
                            Message::Ping { seq } => {
                                if let Some(p) = peers.get(&peer_id) {
                                    let _ = p.tx.send(Message::Pong { seq });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some(ev) = input_rx.recv() => {
                if paused {
                    continue;
                }
                match ev {
                    InputEvent::EmergencyRelease => {
                        return_server_to_local(backend.as_ref(), &mut router, &capture_control, &peers, &snapshot);
                        push_log(&snapshot, "觸發緊急逃生快捷鍵 (Ctrl+Alt+Esc)，游標已強制返回本機");
                    }
                    InputEvent::MouseMove { x, y, dx, dy } => {
                        if let Some(active) = router.active_remote() {
                            if dx != 0 || dy != 0 {
                                if let Some(peer) = peers.get(&active.peer_id) {
                                    let _ = peer.tx.send(Message::MouseMoveRel { dx, dy });
                                }
                            }
                        } else if let Some((edge, peer_id, ratio)) =
                            router.check_local_edge_crossing(x, y, dx, dy, Instant::now())
                        {
                            if let Some(peer) = peers.get(&peer_id) {
                                // Refresh screen geometry in case displays changed
                                if let Ok(scr) = backend.injector().virtual_screen() {
                                    router.update_screen(scr);
                                }
                                let (ax, ay) = router.local_screen().center();
                                capture_control.set_anchor(ax, ay);
                                capture_control.set_grabbed(true);
                                backend.set_cursor_visible(false);
                                router.enter_remote(peer_id.clone(), edge, peer.os, Instant::now());
                                let _ = peer.tx.send(Message::Enter {
                                    edge: edge.opposite(),
                                    ratio,
                                });
                                update_server_snapshot(&snapshot, &peers, &router);
                                debug!("Switched cursor across {:?} edge to {}", edge, peer.hostname);
                            }
                        }
                    }
                    InputEvent::MouseButton { button, pressed } => {
                        if let Some(active) = router.active_remote().cloned() {
                            router.track_remote_button(button, pressed);
                            if let Some(peer) = peers.get(&active.peer_id) {
                                let _ = peer.tx.send(Message::MouseButton { button, pressed });
                            }
                        }
                    }
                    InputEvent::MouseScroll { dx, dy } => {
                        if let Some(active) = router.active_remote().cloned() {
                            let (mdx, mdy) = mapper.map_scroll(dx, dy, backend.os_kind(), active.os);
                            if let Some(peer) = peers.get(&active.peer_id) {
                                let _ = peer.tx.send(Message::MouseScroll { dx: mdx, dy: mdy });
                            }
                        }
                    }
                    InputEvent::Key { key, pressed } => {
                        if let Some(active) = router.active_remote().cloned() {
                            let mapped = mapper.map_key(key, backend.os_kind(), active.os);
                            router.track_remote_key(mapped, pressed);
                            if let Some(peer) = peers.get(&active.peer_id) {
                                let _ = peer.tx.send(Message::KeyEvent { key: mapped, pressed });
                            }
                        }
                    }
                }
            }
            _ = clip_interval.tick() => {
                if cfg.clipboard_sync_enabled && !peers.is_empty() {
                    if let Some(msg) = clipboard_sync.poll_local_change(backend.clipboard().as_ref()) {
                        for peer in peers.values() {
                            let _ = peer.tx.send(msg.clone());
                        }
                    }
                }
            }
            _ = ping_interval.tick() => {
                ping_seq = ping_seq.wrapping_add(1);
                for peer in peers.values() {
                    let _ = peer.tx.send(Message::Ping { seq: ping_seq });
                }
            }
        }
    }

    // Cleanup on stop
    return_server_to_local(backend.as_ref(), &mut router, &capture_control, &peers, &snapshot);
}

fn return_server_to_local(
    backend: &dyn OsBackend,
    router: &mut EdgeRouter,
    capture_control: &Arc<CaptureControl>,
    peers: &HashMap<String, ServerPeer>,
    snapshot: &Arc<RwLock<EngineSnapshot>>,
) {
    capture_control.set_grabbed(false);
    backend.set_cursor_visible(true);
    let (prev, cleanup_msgs) = router.return_local(Instant::now());
    if let Some(active) = prev {
        if let Some(peer) = peers.get(&active.peer_id) {
            for msg in cleanup_msgs {
                let _ = peer.tx.send(msg);
            }
        }
    }
    update_server_snapshot(snapshot, peers, router);
}

async fn handle_server_client_conn(
    mut conn: MessageConnection,
    peer_addr: SocketAddr,
    shared_config: Arc<RwLock<AppConfig>>,
    local_os: OsKind,
    local_screen: ScreenRect,
    snapshot: Arc<RwLock<EngineSnapshot>>,
    inbound_tx: mpsc::UnboundedSender<ServerInbound>,
) {
    let hello = match tokio::time::timeout(Duration::from_secs(5), conn.recv()).await {
        Ok(Ok(Message::Hello {
            protocol_version,
            peer_id,
            hostname,
            os,
            screen,
            pairing_token,
        })) => (protocol_version, peer_id, hostname, os, screen, pairing_token),
        Ok(Ok(_)) => {
            warn!("Unexpected initial message from {peer_addr}");
            return;
        }
        Ok(Err(e)) => {
            warn!("Handshake read error from {peer_addr}: {e}");
            return;
        }
        Err(_) => {
            warn!("Handshake timeout from {peer_addr}");
            return;
        }
    };

    let (proto_ver, peer_id, hostname, os, screen, token) = hello;
    let cfg = shared_config.read().unwrap().clone();

    let (accepted, reason) = if proto_ver != PROTOCOL_VERSION {
        (
            false,
            Some(format!(
                "Protocol mismatch: server v{PROTOCOL_VERSION}, client v{proto_ver}"
            )),
        )
    } else if !cfg.pairing_token.is_empty()
        && token.as_deref() != Some(cfg.pairing_token.as_str())
    {
        (false, Some("配對碼 (Pairing Token) 不符".into()))
    } else {
        (true, None)
    };

    let welcome = Message::Welcome {
        protocol_version: PROTOCOL_VERSION,
        peer_id: cfg.peer_id,
        hostname: cfg.hostname,
        os: local_os,
        screen: local_screen,
        accepted,
        reason: reason.clone(),
    };

    if let Err(e) = conn.send(&welcome).await {
        warn!("Failed to send Welcome to {peer_addr}: {e}");
        return;
    }

    if !accepted {
        push_log(
            &snapshot,
            format!(
                "拒絕來自 {hostname} ({peer_addr}) 的連線: {}",
                reason.unwrap_or_default()
            ),
        );
        return;
    }

    let (mut reader, mut writer) = conn.split();
    let (peer_tx, mut peer_rx) = mpsc::unbounded_channel::<Message>();

    let writer_task = tokio::spawn(async move {
        while let Some(msg) = peer_rx.recv().await {
            if writer.send(&msg).await.is_err() {
                break;
            }
        }
    });

    let peer = ServerPeer {
        peer_id: peer_id.clone(),
        hostname,
        os,
        addr: peer_addr.to_string(),
        screen,
        tx: peer_tx,
    };

    if inbound_tx.send(ServerInbound::Connected { peer }).is_err() {
        writer_task.abort();
        return;
    }

    loop {
        match reader.recv().await {
            Ok(msg) => {
                if inbound_tx
                    .send(ServerInbound::Message {
                        peer_id: peer_id.clone(),
                        msg,
                    })
                    .is_err()
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    writer_task.abort();
    let _ = inbound_tx.send(ServerInbound::Disconnected { peer_id });
}

fn release_client_inputs(
    injector: &dyn InputInjector,
    pressed_keys: &mut HashSet<KeyCode>,
    pressed_buttons: &mut HashSet<MouseButton>,
) {
    for key in pressed_keys.drain() {
        let _ = injector.key_event(key, false);
    }
    for button in pressed_buttons.drain() {
        let _ = injector.mouse_button(button, false);
    }
}

async fn run_client_worker(
    backend: Arc<dyn OsBackend>,
    shared_config: Arc<RwLock<AppConfig>>,
    cfg_dir: PathBuf,
    snapshot: Arc<RwLock<EngineSnapshot>>,
    mut stop_rx: watch::Receiver<bool>,
    mut ctrl_rx: mpsc::UnboundedReceiver<WorkerControl>,
) {
    let mut paused = false;

    loop {
        if *stop_rx.borrow() {
            break;
        }

        let cfg = shared_config.read().unwrap().clone();
        let addr = cfg.connect_address.clone();
        let expected_fp = cfg.trusted_fingerprints.get(&addr).cloned();

        push_log(&snapshot, format!("正在透過 TLS 1.3 連線至 {addr}..."));

        let connect_result = tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    break;
                }
                continue;
            }
            res = connect_tls(&addr, TrustPolicy::TrustOnFirstUse { expected_fingerprint: expected_fp.clone() }) => res,
        };

        let (mut conn, peer_fp) = match connect_result {
            Ok(pair) => pair,
            Err(e) => {
                push_log(&snapshot, format!("連線失敗 ({addr}): {e}，2 秒後重試..."));
                tokio::select! {
                    _ = stop_rx.changed() => break,
                    _ = tokio::time::sleep(Duration::from_secs(2)) => continue,
                }
            }
        };

        // TOFU: store newly seen fingerprint
        if expected_fp.is_none() && !peer_fp.is_empty() {
            let short = format_short_fingerprint(&peer_fp);
            push_log(
                &snapshot,
                format!("首次連線已記錄並信任主機指紋 (TOFU): {short}"),
            );
            let mut wcfg = shared_config.write().unwrap();
            wcfg.trusted_fingerprints.insert(addr.clone(), peer_fp.clone());
            let _ = wcfg.save(&cfg_dir);
        }

        let local_screen = backend
            .injector()
            .virtual_screen()
            .unwrap_or_else(|_| ScreenRect::new(0, 0, 1920, 1080));

        let hello = Message::Hello {
            protocol_version: PROTOCOL_VERSION,
            peer_id: cfg.peer_id.clone(),
            hostname: cfg.hostname.clone(),
            os: backend.os_kind(),
            screen: local_screen,
            pairing_token: if cfg.pairing_token.is_empty() {
                None
            } else {
                Some(cfg.pairing_token.clone())
            },
        };

        if let Err(e) = conn.send(&hello).await {
            push_log(&snapshot, format!("送出 Hello 失敗: {e}"));
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        let server_info = match tokio::time::timeout(Duration::from_secs(5), conn.recv()).await {
            Ok(Ok(Message::Welcome {
                peer_id,
                hostname,
                os,
                screen,
                accepted,
                reason,
                ..
            })) => {
                if !accepted {
                    push_log(
                        &snapshot,
                        format!(
                            "主機拒絕連線: {}",
                            reason.unwrap_or_else(|| "未知原因".into())
                        ),
                    );
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    continue;
                }
                ConnectedPeerInfo {
                    peer_id,
                    hostname,
                    os,
                    addr: addr.clone(),
                    edge: None,
                    screen,
                }
            }
            Ok(Ok(_)) => {
                push_log(&snapshot, "收到非預期的握手回應");
                continue;
            }
            Ok(Err(e)) => {
                push_log(&snapshot, format!("握手失敗: {e}"));
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            Err(_) => {
                push_log(&snapshot, "握手逾時");
                continue;
            }
        };

        push_log(
            &snapshot,
            format!(
                "已連線至主控端 {} ({})",
                server_info.hostname,
                server_info.os.display_name()
            ),
        );
        if let Ok(mut s) = snapshot.write() {
            s.connected_peers = vec![server_info.clone()];
            s.active_screen_label = format!("待命 (連線至 {})", server_info.hostname);
        }

        let (mut reader, mut writer) = conn.split();
        let injector = backend.injector();
        let mut clipboard_sync = ClipboardSync::new();
        clipboard_sync.seed(backend.clipboard().as_ref());

        let mut return_edge: Option<Edge> = None;
        let mut pressed_keys: HashSet<KeyCode> = HashSet::new();
        let mut pressed_buttons: HashSet<MouseButton> = HashSet::new();
        let mut clip_interval = tokio::time::interval(Duration::from_millis(350));
        let mut cur_cfg = cfg;

        loop {
            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        release_client_inputs(injector.as_ref(), &mut pressed_keys, &mut pressed_buttons);
                        return;
                    }
                }
                Some(ctrl) = ctrl_rx.recv() => {
                    match ctrl {
                        WorkerControl::SetPaused(p) => {
                            paused = p;
                            if paused {
                                release_client_inputs(injector.as_ref(), &mut pressed_keys, &mut pressed_buttons);
                            }
                        }
                        WorkerControl::ReloadConfig(new_cfg) => {
                            cur_cfg = new_cfg;
                        }
                        _ => {}
                    }
                }
                res = reader.recv() => {
                    let msg = match res {
                        Ok(m) => m,
                        Err(e) => {
                            push_log(&snapshot, format!("與主控端連線中斷: {e}"));
                            break;
                        }
                    };
                    if paused {
                        continue;
                    }
                    match msg {
                        Message::Enter { edge, ratio } => {
                            let scr = injector.virtual_screen().unwrap_or(local_screen);
                            let (x, y) = scr.entry_point(edge, ratio);
                            let _ = injector.move_cursor_abs(x, y);
                            return_edge = Some(edge);
                            if let Ok(mut s) = snapshot.write() {
                                s.active_screen_label = "受控中 (游標在本機)".into();
                            }
                        }
                        Message::Leave { .. } | Message::ReleaseAll => {
                            release_client_inputs(injector.as_ref(), &mut pressed_keys, &mut pressed_buttons);
                            return_edge = None;
                            if let Ok(mut s) = snapshot.write() {
                                s.active_screen_label = format!("待命 (連線至 {})", server_info.hostname);
                            }
                        }
                        Message::MouseMoveRel { dx, dy } => {
                            let scr = injector.virtual_screen().unwrap_or(local_screen);
                            if let (Some(ret_edge), Ok((cx, cy))) = (return_edge, injector.cursor_position()) {
                                if let Some(ratio) = EdgeRouter::check_client_return_edge(
                                    scr,
                                    ret_edge,
                                    cx,
                                    cy,
                                    dx,
                                    dy,
                                    cur_cfg.dead_corner_px,
                                ) {
                                    release_client_inputs(injector.as_ref(), &mut pressed_keys, &mut pressed_buttons);
                                    return_edge = None;
                                    if let Ok(mut s) = snapshot.write() {
                                        s.active_screen_label = format!("待命 (連線至 {})", server_info.hostname);
                                    }
                                    if writer.send(&Message::Leave { edge: ret_edge, ratio }).await.is_err() {
                                        break;
                                    }
                                    continue;
                                }
                            }
                            let _ = injector.move_cursor_rel(dx, dy);
                        }
                        Message::MouseMoveAbs { x_ratio, y_ratio } => {
                            let scr = injector.virtual_screen().unwrap_or(local_screen);
                            let x = scr.x + (x_ratio.clamp(0.0, 1.0) * (scr.width - 1) as f32).round() as i32;
                            let y = scr.y + (y_ratio.clamp(0.0, 1.0) * (scr.height - 1) as f32).round() as i32;
                            let _ = injector.move_cursor_abs(x, y);
                        }
                        Message::MouseButton { button, pressed } => {
                            if pressed {
                                pressed_buttons.insert(button);
                            } else {
                                pressed_buttons.remove(&button);
                            }
                            let _ = injector.mouse_button(button, pressed);
                        }
                        Message::MouseScroll { dx, dy } => {
                            let _ = injector.mouse_scroll(dx, dy);
                        }
                        Message::KeyEvent { key, pressed } => {
                            if pressed {
                                pressed_keys.insert(key);
                            } else {
                                pressed_keys.remove(&key);
                            }
                            let _ = injector.key_event(key, pressed);
                        }
                        Message::ClipboardText { text, .. } => {
                            if cur_cfg.clipboard_sync_enabled {
                                let _ = clipboard_sync.apply_remote(backend.clipboard().as_ref(), &text);
                            }
                        }
                        Message::Ping { seq } => {
                            if writer.send(&Message::Pong { seq }).await.is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                _ = clip_interval.tick() => {
                    if cur_cfg.clipboard_sync_enabled {
                        if let Some(msg) = clipboard_sync.poll_local_change(backend.clipboard().as_ref()) {
                            if writer.send(&msg).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Always release stuck keys/buttons on disconnect
        release_client_inputs(injector.as_ref(), &mut pressed_keys, &mut pressed_buttons);
        if let Ok(mut s) = snapshot.write() {
            s.connected_peers.clear();
            s.active_screen_label = "未連線 (重新連線中...)".into();
        }
        error!("Client session ended; retrying in 2s...");
        tokio::select! {
            _ = stop_rx.changed() => break,
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mouseroaming_os::mock::{InjectedAction, MockOsBackend};
    use mouseroaming_os::ClipboardProvider;

    #[tokio::test]
    async fn end_to_end_windows_server_to_macos_client_roaming() {
        let base_dir = std::env::temp_dir().join(format!(
            "mouseroaming_e2e_{}",
            std::process::id()
        ));
        let srv_dir = base_dir.join("server");
        let cli_dir = base_dir.join("client");
        let _ = std::fs::remove_dir_all(&base_dir);

        // Windows Server (1920x1080) on the Left, macOS Client (1440x900) on the Right
        let win_backend = Arc::new(MockOsBackend::new(
            OsKind::Windows,
            ScreenRect::new(0, 0, 1920, 1080),
        ));
        let mac_backend = Arc::new(MockOsBackend::new(
            OsKind::MacOS,
            ScreenRect::new(0, 0, 1440, 900),
        ));

        let mut srv_cfg = AppConfig::default();
        srv_cfg.hostname = "WinHost".into();
        srv_cfg.role = AppRole::Server;
        srv_cfg.listen_port = 0; // Let OS assign free port
        srv_cfg.default_client_edge = Edge::Right;
        srv_cfg.edge_switch_cooldown_ms = 0;
        srv_cfg.pairing_token = "888888".into();

        let srv_handle = MouseRoamingEngine::spawn(
            &tokio::runtime::Handle::current(),
            win_backend.clone(),
            srv_cfg,
            EngineOptions {
                config_dir: srv_dir,
                enable_mdns: false,
                bind_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            },
        )
        .expect("spawn server");

        // Wait until server reports listen_addr
        let mut listen_addr = None;
        for _ in 0..50 {
            if let Some(a) = srv_handle.snapshot().listen_addr {
                listen_addr = Some(a);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let listen_addr = listen_addr.expect("server bound");

        let mut cli_cfg = AppConfig::default();
        cli_cfg.hostname = "MacClient".into();
        cli_cfg.role = AppRole::Client;
        cli_cfg.connect_address = listen_addr;
        cli_cfg.pairing_token = "888888".into();

        let cli_handle = MouseRoamingEngine::spawn(
            &tokio::runtime::Handle::current(),
            mac_backend.clone(),
            cli_cfg,
            EngineOptions {
                config_dir: cli_dir,
                enable_mdns: false,
                bind_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            },
        )
        .expect("spawn client");

        // Wait for TLS handshake & peer connection
        for _ in 0..100 {
            if !srv_handle.snapshot().connected_peers.is_empty()
                && !cli_handle.snapshot().connected_peers.is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(srv_handle.snapshot().connected_peers.len(), 1);
        assert_eq!(cli_handle.snapshot().connected_peers.len(), 1);

        // 1. Move Windows cursor to Right edge (x=1919, y=540 -> ratio 0.5)
        win_backend.push_input(InputEvent::MouseMove {
            x: 1919,
            y: 540,
            dx: 5,
            dy: 0,
        });
        tokio::time::sleep(Duration::from_millis(60)).await;

        assert!(
            srv_handle.snapshot().active_screen_label.contains("MacClient"),
            "active screen should switch to MacClient, got: {}",
            srv_handle.snapshot().active_screen_label
        );
        assert!(
            !win_backend.is_cursor_visible(),
            "local mouse cursor should be hidden when control is roaming on remote client"
        );

        // 2. Send relative mouse move, left click, and Windows Ctrl+C (must become macOS LeftMeta+C!)
        win_backend.push_input(InputEvent::MouseMove {
            x: 960,
            y: 540,
            dx: 20,
            dy: -10,
        });
        win_backend.push_input(InputEvent::MouseButton {
            button: MouseButton::Left,
            pressed: true,
        });
        win_backend.push_input(InputEvent::MouseButton {
            button: MouseButton::Left,
            pressed: false,
        });
        win_backend.push_input(InputEvent::Key {
            key: KeyCode::LeftCtrl,
            pressed: true,
        });
        win_backend.push_input(InputEvent::Key {
            key: KeyCode::C,
            pressed: true,
        });
        win_backend.push_input(InputEvent::Key {
            key: KeyCode::C,
            pressed: false,
        });
        win_backend.push_input(InputEvent::Key {
            key: KeyCode::LeftCtrl,
            pressed: false,
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let actions = mac_backend.mock_injector().recorded_actions();
        assert!(
            actions.contains(&InjectedAction::MoveRel(20, -10)),
            "expected MoveRel(20, -10) in {:?}",
            actions
        );
        assert!(
            actions.contains(&InjectedAction::MouseButton {
                button: MouseButton::Left,
                pressed: true
            }),
            "expected left click in {:?}",
            actions
        );
        assert!(
            actions.contains(&InjectedAction::Key {
                key: KeyCode::LeftMeta,
                pressed: true
            }),
            "expected Windows LeftCtrl to map to macOS LeftMeta (Cmd) in {:?}",
            actions
        );
        assert!(
            actions.contains(&InjectedAction::Key {
                key: KeyCode::C,
                pressed: true
            }),
            "expected C key in {:?}",
            actions
        );

        // 3. Test clipboard sync from Windows to macOS
        win_backend
            .mock_clipboard()
            .set_text("MouseRoaming 剪貼簿同步測試")
            .unwrap();
        for _ in 0..30 {
            if mac_backend.mock_clipboard().get_text().unwrap().as_deref()
                == Some("MouseRoaming 剪貼簿同步測試")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            mac_backend.mock_clipboard().get_text().unwrap().as_deref(),
            Some("MouseRoaming 剪貼簿同步測試")
        );

        // 4. Push macOS cursor back across its Left edge to return to Windows Server
        win_backend.push_input(InputEvent::MouseMove {
            x: 960,
            y: 540,
            dx: -100,
            dy: 0,
        });
        for _ in 0..30 {
            if srv_handle.snapshot().active_screen_label.contains("Local") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            srv_handle.snapshot().active_screen_label.contains("Local"),
            "cursor should return to local Windows host"
        );
        assert!(
            win_backend.is_cursor_visible(),
            "local mouse cursor should be visible again after returning to local host"
        );

        cli_handle.shutdown();
        srv_handle.shutdown();
        let _ = std::fs::remove_dir_all(&base_dir);
    }
}
