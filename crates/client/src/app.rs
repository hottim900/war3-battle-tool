use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use eframe::egui;
use tokio::sync::{mpsc, oneshot};
use war3_protocol::messages::{ClientMessage, PlayerInfo, RoomInfo, ServerMessage};

use crate::logging::LogEntry;
use crate::net::discovery::NetEvent;
use crate::net::packet::{RawUdpInjector, check_room};
use crate::net::quic::StrategyResult;
use crate::net::tunnel::{self, Transport, TunnelEvent};
use crate::ui::lobby::{LobbyAction, LobbyPanel};
use crate::ui::log_panel::LogPanel;
use crate::ui::setup_wizard::SetupWizard;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tab {
    Lobby,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum LogTab {
    Log,
    Timeline,
}

#[derive(Debug, Clone)]
enum ConnectionState {
    Disconnected,
    Connected,
    Reconnecting { attempt: u32 },
}

/// Tracks an in-flight user action awaiting server confirmation.
#[derive(Debug, Clone)]
enum PendingAction {
    /// User clicked "join" on a room; waiting for JoinResult.
    Joining { room_name: String },
    /// JoinResult succeeded; tunnel is connecting.
    JoinSuccess,
    /// JoinResult failed; show error.
    JoinFailed { reason: String },
    /// User clicked "create room"; waiting for RoomUpdate to confirm.
    CreatingRoom { room_name: String },
}

pub struct War3App {
    config: crate::config::AppConfig,
    config_changed: bool,

    cmd_tx: mpsc::UnboundedSender<ClientMessage>,
    event_rx: mpsc::UnboundedReceiver<NetEvent>,

    /// Tokio runtime handle for spawning tunnel tasks
    rt_handle: tokio::runtime::Handle,
    /// Server base URL (e.g. wss://war3.kalthor.cc/ws)
    server_url: String,

    /// Channel for receiving tunnel events from background tasks
    tunnel_event_tx: mpsc::UnboundedSender<TunnelEvent>,
    tunnel_event_rx: mpsc::UnboundedReceiver<TunnelEvent>,

    connection_state: ConnectionState,
    ever_connected: bool,
    my_player_id: Option<String>,

    players: Vec<PlayerInfo>,
    rooms: Vec<RoomInfo>,

    current_tab: Tab,
    wizard: Option<SetupWizard>,
    lobby: LobbyPanel,
    log_panel: LogPanel,
    log_tab: LogTab,

    pending_action: Option<PendingAction>,

    /// Pending GAMEINFO for injection (set when JoinResult received)
    pending_gameinfo: Option<Vec<u8>>,
    /// Handle to the GAMEINFO injection task (for cancellation)
    injection_handle: Option<tokio::task::JoinHandle<()>>,
    /// Handle to the tunnel task (for cancellation on re-join or cleanup)
    tunnel_handle: Option<tokio::task::JoinHandle<()>>,

    /// Lobby RTT 測量（ms），由 discovery 更新
    latency_ms: Arc<AtomicU64>,
    /// Tunnel RTT 測量（ms），遊戲中由 tunnel bridge 更新，0 表示無 tunnel
    tunnel_latency_ms: Arc<AtomicU64>,

    /// Server 觀測到的我方 IP（CGNAT 偵測用）
    my_observed_ip: Option<std::net::IpAddr>,
    /// P2P 直連：對方 IP（從 StunInfo 接收）
    peer_addr: Option<std::net::IpAddr>,
    /// 目前遊戲傳輸路徑（relay 或 direct）
    transport: Option<Transport>,

    /// Joiner 的 UPnP addr oneshot sender（一次只有一個 active joiner）
    upnp_addr_sender: Option<oneshot::Sender<SocketAddr>>,
    /// Host 的 UPnP mapped 通知 channel（background_quic_host 送出，app 層轉發給 server）
    upnp_mapped_tx: mpsc::UnboundedSender<(String, SocketAddr)>,
    upnp_mapped_rx: mpsc::UnboundedReceiver<(String, SocketAddr)>,
    /// 連線策略診斷結果
    connection_diagnostics: Vec<StrategyResult>,

    /// 從 UiLogLayer 接收 log entries 的 channel
    log_rx: mpsc::UnboundedReceiver<LogEntry>,
}

impl War3App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: crate::config::AppConfig,
        cmd_tx: mpsc::UnboundedSender<ClientMessage>,
        event_rx: mpsc::UnboundedReceiver<NetEvent>,
        rt_handle: tokio::runtime::Handle,
        server_url: String,
        latency_ms: Arc<AtomicU64>,
        log_rx: mpsc::UnboundedReceiver<LogEntry>,
    ) -> Self {
        setup_cjk_fonts(&cc.egui_ctx);

        // 暗色主題 — 配色與 web viewer 一致
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = egui::Color32::from_rgb(0x1a, 0x1a, 0x2e);
        visuals.window_fill = egui::Color32::from_rgb(0x16, 0x21, 0x3e);
        visuals.extreme_bg_color = egui::Color32::from_rgb(0x0f, 0x0f, 0x23);
        visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(0x16, 0x21, 0x3e);
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(0x1e, 0x29, 0x3b);
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x33, 0x41, 0x55);
        visuals.widgets.active.bg_fill = egui::Color32::from_rgb(0x3b, 0x82, 0xf6);
        visuals.selection.bg_fill = egui::Color32::from_rgb(0x3b, 0x82, 0xf6);
        visuals.widgets.noninteractive.fg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(0xcc, 0xd6, 0xf6));
        cc.egui_ctx.set_visuals(visuals);

        // 全域間距與字型大小
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        if let Some(body) = style.text_styles.get_mut(&egui::TextStyle::Body) {
            body.size = 15.0;
        }
        cc.egui_ctx.set_style(style);

        let needs_wizard = !config.is_configured();
        let log_buffer_size = config.log_buffer_size;
        let (tunnel_event_tx, tunnel_event_rx) = mpsc::unbounded_channel();
        let (upnp_mapped_tx, upnp_mapped_rx) = mpsc::unbounded_channel();

        let app = Self {
            config,
            config_changed: false,
            cmd_tx,
            event_rx,
            rt_handle,
            server_url,
            tunnel_event_tx,
            tunnel_event_rx,
            connection_state: ConnectionState::Disconnected,
            ever_connected: false,
            my_player_id: None,
            players: Vec::new(),
            rooms: Vec::new(),
            current_tab: Tab::Lobby,
            wizard: if needs_wizard {
                Some(SetupWizard::new())
            } else {
                None
            },
            lobby: LobbyPanel::new(),
            log_panel: LogPanel::new(log_buffer_size),
            log_tab: LogTab::Log,
            pending_action: None,
            pending_gameinfo: None,
            injection_handle: None,
            tunnel_handle: None,
            latency_ms,
            tunnel_latency_ms: Arc::new(AtomicU64::new(0)),
            my_observed_ip: None,
            peer_addr: None,
            transport: None,
            upnp_addr_sender: None,
            upnp_mapped_tx,
            upnp_mapped_rx,
            connection_diagnostics: Vec::new(),
            log_rx,
        };

        tracing::info!(verbosity = "concise", "War3 Battle Tool 啟動");
        app
    }

    fn is_registered(&self) -> bool {
        self.my_player_id.is_some()
    }

    fn is_hosting(&self) -> bool {
        self.my_player_id
            .as_ref()
            .map(|my_id| {
                self.players
                    .iter()
                    .any(|p| p.player_id == *my_id && p.is_hosting)
            })
            .unwrap_or(false)
    }

    fn send_register(&self) {
        let _ = self.cmd_tx.send(ClientMessage::Register {
            nickname: self.config.nickname.clone(),
            war3_version: self.config.war3_version,
            client_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        });
    }

    /// 取消舊的 tunnel task（避免 ghost socket）
    fn abort_tunnel(&mut self) {
        if let Some(h) = self.tunnel_handle.take() {
            h.abort();
        }
    }

    /// 啟動 joiner 端 tunnel 和 GAMEINFO 注入
    fn start_joiner_tunnel(&mut self, tunnel_token: String, gameinfo: Vec<u8>) {
        self.abort_tunnel();
        self.connection_diagnostics.clear();

        let server_url = self.server_url.clone();
        let event_tx = self.tunnel_event_tx.clone();
        let peer_addr = self.peer_addr.take();

        // UPnP addr oneshot：server 送 PeerUPnPAddr 時透過這個 channel 傳給 tunnel task
        let (upnp_tx, upnp_rx) = oneshot::channel::<SocketAddr>();
        self.upnp_addr_sender = Some(upnp_tx);

        self.tunnel_latency_ms.store(0, Ordering::Relaxed);
        let tunnel_lat = self.tunnel_latency_ms.clone();
        let handle = self.rt_handle.spawn(async move {
            tunnel::run_joiner_tunnel(
                server_url,
                tunnel_token,
                peer_addr,
                upnp_rx,
                event_tx,
                tunnel_lat,
            )
            .await;
        });
        self.tunnel_handle = Some(handle);

        // 存 GAMEINFO，等 ProxyReady 後開始注入
        self.pending_gameinfo = Some(gameinfo);
    }

    /// 啟動 host 端 tunnel
    fn start_host_tunnel(&mut self, tunnel_token: String) {
        self.abort_tunnel();
        self.connection_diagnostics.clear();

        let server_url = self.server_url.clone();
        let event_tx = self.tunnel_event_tx.clone();
        let peer_addr = self.peer_addr.take();
        let mapped_tx = self.upnp_mapped_tx.clone();
        let my_observed_ip = self.my_observed_ip;

        self.tunnel_latency_ms.store(0, Ordering::Relaxed);
        let tunnel_lat = self.tunnel_latency_ms.clone();
        let handle = self.rt_handle.spawn(async move {
            tunnel::run_host_tunnel(
                server_url,
                tunnel_token,
                peer_addr,
                my_observed_ip,
                mapped_tx,
                event_tx,
                tunnel_lat,
            )
            .await;
        });
        self.tunnel_handle = Some(handle);
    }

    /// 開始 GAMEINFO 注入循環（ProxyReady 時呼叫）
    fn start_gameinfo_injection(&mut self) {
        // 取消之前的注入任務
        if let Some(h) = self.injection_handle.take() {
            h.abort();
        }

        let gameinfo = match self.pending_gameinfo.take() {
            Some(gi) if !gi.is_empty() => gi,
            _ => {
                tracing::warn!(verbosity = "concise", "沒有 GAMEINFO 可注入");
                return;
            }
        };

        let handle = self.rt_handle.spawn(async move {
            let injector = match RawUdpInjector::new() {
                Ok(i) => i,
                Err(e) => {
                    tracing::error!(%e, "RawUdpInjector 建立失敗");
                    return;
                }
            };

            // 持續注入 GAMEINFO，每 3 秒一次，共 60 次（3 分鐘）
            for _ in 0..60 {
                if let Err(e) = injector.inject(&gameinfo) {
                    tracing::warn!(%e, "GAMEINFO 注入失敗");
                    break;
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });
        self.injection_handle = Some(handle);

        tracing::info!(
            verbosity = "concise",
            "GAMEINFO 注入開始，請切換到 War3 區域網路畫面加入遊戲"
        );
    }

    fn process_network_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                NetEvent::Connected => {
                    self.connection_state = ConnectionState::Connected;
                    self.ever_connected = true;
                    tracing::info!(verbosity = "concise", "已連線到發現伺服器");

                    if !self.is_registered() && self.config.is_configured() {
                        self.send_register();
                    }
                }
                NetEvent::Disconnected => {
                    self.connection_state = ConnectionState::Disconnected;
                    self.my_player_id = None;
                    tracing::warn!(verbosity = "concise", "與伺服器的連線中斷");
                }
                NetEvent::Reconnecting { attempt } => {
                    self.connection_state = ConnectionState::Reconnecting { attempt };
                    tracing::info!(verbosity = "concise", "正在重新連線... (第 {attempt} 次)");
                }
                NetEvent::ServerMessage(msg) => self.handle_server_message(msg),
            }
        }

        // 處理 host UPnP mapped 通知（背景 task → app → server）
        while let Ok((token, addr)) = self.upnp_mapped_rx.try_recv() {
            let _ = self.cmd_tx.send(ClientMessage::UPnPMapped {
                external_addr: addr.to_string(),
                tunnel_token: token,
            });
        }

        // 處理 tunnel 事件
        while let Ok(event) = self.tunnel_event_rx.try_recv() {
            match event {
                TunnelEvent::ProxyReady => {
                    tracing::info!(verbosity = "concise", "Tunnel proxy 就緒");
                    self.start_gameinfo_injection();
                }
                TunnelEvent::TransportSelected(t) => {
                    self.transport = Some(t);
                    match t {
                        Transport::Direct => {
                            tracing::info!(verbosity = "concise", "傳輸: P2P 直連")
                        }
                        Transport::Relay => {
                            tracing::info!(verbosity = "concise", "傳輸: Relay 中繼")
                        }
                    }
                }
                TunnelEvent::TransportUpgraded => {
                    self.transport = Some(Transport::Direct);
                    tracing::info!(verbosity = "concise", "傳輸升級: Relay → P2P 直連");
                }
                TunnelEvent::Finished { error: None } => {
                    if let Some(h) = self.injection_handle.take() {
                        h.abort();
                    }
                    self.tunnel_handle = None;
                    self.transport = None;
                    self.tunnel_latency_ms.store(0, Ordering::Relaxed);
                    tracing::info!(verbosity = "concise", "Tunnel 連線結束");
                }
                TunnelEvent::Finished { error: Some(e) } => {
                    if let Some(h) = self.injection_handle.take() {
                        h.abort();
                    }
                    self.tunnel_handle = None;
                    self.transport = None;
                    self.tunnel_latency_ms.store(0, Ordering::Relaxed);
                    tracing::error!(verbosity = "concise", "Tunnel 錯誤: {e}");
                }
                TunnelEvent::StrategyResults(results) => {
                    for r in &results {
                        let status = match &r.outcome {
                            crate::net::quic::StrategyOutcome::Success => {
                                format!("成功 ({}ms)", r.duration_ms)
                            }
                            crate::net::quic::StrategyOutcome::Failed(reason) => {
                                format!("失敗: {reason}")
                            }
                            crate::net::quic::StrategyOutcome::Skipped => "跳過".to_string(),
                        };
                        tracing::info!(
                            verbosity = "detailed",
                            "策略 {}: {} ({}ms)",
                            r.method,
                            status,
                            r.duration_ms
                        );
                    }
                    self.connection_diagnostics = results;
                }
                TunnelEvent::GameinfoCaptured {
                    room_name,
                    map_name,
                    max_players,
                    gameinfo,
                } => {
                    if gameinfo.is_empty() {
                        self.pending_action = None;
                        tracing::error!(
                            verbosity = "concise",
                            "請先在 War3 中建立遊戲，再回來建立房間"
                        );
                    } else {
                        let _ = self.cmd_tx.send(ClientMessage::CreateRoom {
                            room_name,
                            map_name,
                            max_players,
                            gameinfo,
                        });
                    }
                }
            }
        }
    }

    fn handle_server_message(&mut self, msg: ServerMessage) {
        match msg {
            ServerMessage::Welcome { player_id } => {
                self.my_player_id = Some(player_id);
                tracing::info!(verbosity = "concise", "註冊成功");
            }
            ServerMessage::YourObservedAddr { ip } => match ip.parse::<std::net::IpAddr>() {
                Ok(addr) => {
                    self.my_observed_ip = Some(addr);
                }
                Err(_) => {
                    tracing::warn!(verbosity = "detailed", "觀測 IP 格式錯誤: {ip}");
                }
            },
            ServerMessage::PlayerUpdate { players } => {
                self.players = players;
            }
            ServerMessage::RoomUpdate { rooms } => {
                if matches!(
                    self.pending_action,
                    Some(PendingAction::CreatingRoom { .. })
                ) {
                    self.pending_action = None;
                }
                self.rooms = rooms;
            }
            ServerMessage::JoinResult {
                success,
                room_id: _,
                tunnel_token,
                gameinfo,
            } => {
                if success {
                    self.pending_action = Some(PendingAction::JoinSuccess);
                    if let (Some(token), Some(gi)) = (tunnel_token, gameinfo) {
                        tracing::info!(verbosity = "concise", "加入成功！正在建立 tunnel 連線...");
                        self.start_joiner_tunnel(token, gi);
                    } else {
                        tracing::warn!(verbosity = "concise", "加入成功但缺少 tunnel 資訊");
                    }
                } else {
                    self.pending_action = Some(PendingAction::JoinFailed {
                        reason: "房間可能已關閉".to_string(),
                    });
                    tracing::error!(verbosity = "concise", "加入失敗，房間可能已關閉");
                }
            }
            ServerMessage::PlayerJoined {
                nickname,
                tunnel_token,
            } => {
                tracing::info!(
                    verbosity = "concise",
                    "玩家 {nickname} 加入，建立 tunnel..."
                );
                self.start_host_tunnel(tunnel_token);
            }
            ServerMessage::TunnelReady { tunnel_token } => {
                tracing::info!(verbosity = "concise", "Tunnel 就緒，建立連線...");
                self.start_host_tunnel(tunnel_token);
            }
            ServerMessage::StunInfo { peer_addr } => {
                if let Ok(ip) = peer_addr.parse::<std::net::IpAddr>() {
                    self.peer_addr = Some(ip);
                    tracing::info!(verbosity = "detailed", "收到 P2P 直連資訊");
                }
            }
            ServerMessage::PeerUPnPAddr { external_addr } => {
                // SSRF check：parse 並拒絕 RFC1918/loopback/link-local
                match external_addr.parse::<SocketAddr>() {
                    Ok(addr) if is_safe_external_addr(addr.ip()) => {
                        tracing::info!(verbosity = "detailed", "收到 UPnP 直連位址: {addr}");
                        // 送給當前 tunnel task（只有一個 active joiner）
                        if let Some(sender) = self.upnp_addr_sender.take() {
                            if sender.send(addr).is_err() {
                                tracing::warn!(
                                    verbosity = "detailed",
                                    "UPnP 位址收到但 tunnel 已結束"
                                );
                            }
                        } else {
                            tracing::warn!(
                                verbosity = "detailed",
                                "UPnP 位址收到但無 tunnel 等待接收"
                            );
                        }
                    }
                    Ok(addr) => {
                        tracing::warn!(
                            verbosity = "detailed",
                            "UPnP 位址被拒絕（不安全位址）: {}",
                            addr.ip()
                        );
                    }
                    Err(_) => {
                        tracing::warn!(
                            verbosity = "detailed",
                            "UPnP 位址格式錯誤: {external_addr}"
                        );
                    }
                }
            }
            ServerMessage::Pong { .. } => {
                // 已在 discovery.rs 處理，不會到這裡
            }
            ServerMessage::Error { message } => {
                tracing::error!(verbosity = "concise", "伺服器錯誤: {message}");
            }
            ServerMessage::Unknown => {}
        }
    }

    fn show_connection_overlay(&mut self, ui: &mut egui::Ui) -> bool {
        match &self.connection_state {
            ConnectionState::Connected => false,
            ConnectionState::Disconnected if !self.ever_connected => {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.spinner();
                    ui.add_space(12.0);
                    ui.heading("正在連線發現伺服器...");
                });
                true
            }
            ConnectionState::Disconnected => false,
            ConnectionState::Reconnecting { attempt } if *attempt > 5 => {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.colored_label(
                        egui::Color32::from_rgb(0xf5, 0x9e, 0x0b),
                        egui::RichText::new("離線模式").size(20.0).strong(),
                    );
                    ui.add_space(8.0);
                    ui.label(format!("已嘗試重新連線 {} 次，伺服器可能離線。", attempt));
                    ui.add_space(16.0);
                    ui.label("請等待伺服器恢復後自動重連。");
                });
                true
            }
            ConnectionState::Reconnecting { attempt } => {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.spinner();
                    ui.add_space(12.0);
                    ui.heading("連線中斷，正在重新連線...");
                    ui.add_space(4.0);
                    ui.label(format!("第 {} 次嘗試", attempt));
                });
                true
            }
        }
    }

    fn show_pending_action_banner(&mut self, ui: &mut egui::Ui) -> bool {
        let action = match &self.pending_action {
            Some(a) => a.clone(),
            None => return false,
        };

        match &action {
            PendingAction::Joining { room_name } => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("正在加入「{}」...", room_name));
                });
                ui.add_space(4.0);
                true
            }
            PendingAction::JoinSuccess => {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(15, 35, 25, 200))
                    .inner_margin(8.0)
                    .corner_radius(4.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                egui::Color32::from_rgb(0x64, 0xd8, 0x9a),
                                "加入成功！請切換到 War3 區域網路畫面。",
                            );
                            if ui.button("確定").clicked() {
                                self.pending_action = None;
                            }
                        });
                    });
                ui.add_space(4.0);
                true
            }
            PendingAction::JoinFailed { reason } => {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(35, 15, 15, 200))
                    .inner_margin(8.0)
                    .corner_radius(4.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                egui::Color32::from_rgb(0xef, 0x44, 0x44),
                                format!("加入失敗：{}", reason),
                            );
                            if ui.button("確定").clicked() {
                                self.pending_action = None;
                            }
                        });
                    });
                ui.add_space(4.0);
                true
            }
            PendingAction::CreatingRoom { room_name } => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("正在建立房間「{}」...", room_name));
                });
                ui.add_space(4.0);
                true
            }
        }
    }

    fn show_status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (color, text) = match &self.connection_state {
                ConnectionState::Connected => {
                    (egui::Color32::from_rgb(0x64, 0xd8, 0x9a), "● 已連線")
                }
                ConnectionState::Disconnected => {
                    (egui::Color32::from_rgb(0xef, 0x44, 0x44), "● 已斷線")
                }
                ConnectionState::Reconnecting { attempt: _ } => {
                    (egui::Color32::from_rgb(0xf5, 0x9e, 0x0b), "● 重連中...")
                }
            };
            ui.colored_label(color, text);

            if let ConnectionState::Reconnecting { attempt } = &self.connection_state {
                ui.weak(format!("(第 {} 次)", attempt));
            }

            ui.separator();
            ui.label(format!(
                "線上: {} 人 | 房間: {} 間",
                self.players.len(),
                self.rooms.len()
            ));

            if let Some(id) = &self.my_player_id {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let short_id = id.get(..8).unwrap_or(id);
                    ui.weak(format!("ID: {short_id}…"));
                });
            }
        });
    }
}

impl eframe::App for War3App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 從 UiLogLayer channel drain log entries 到 LogPanel（每幀最多 500 筆）
        for _ in 0..500 {
            match self.log_rx.try_recv() {
                Ok(entry) => self.log_panel.push(entry),
                Err(_) => break,
            }
        }

        self.process_network_events();

        // 首次設定精靈
        if let Some(wizard) = &mut self.wizard {
            wizard.show(ctx);
            if wizard.done {
                self.config.nickname = wizard.nickname.clone();
                self.config.war3_version = wizard.war3_version;
                let _ = self.config.save();
                self.wizard = None;

                if matches!(self.connection_state, ConnectionState::Connected) {
                    self.send_register();
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }

        // 主畫面
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.current_tab, Tab::Lobby, "大廳");
                ui.selectable_value(&mut self.current_tab, Tab::Settings, "設定");
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            self.show_status_bar(ui);
        });

        // 日誌面板固定在底部（大廳和設定頁都可見）
        let has_tunnel = self.transport.is_some() || !self.connection_diagnostics.is_empty();
        egui::TopBottomPanel::bottom("log_panel")
            .resizable(true)
            .default_height(120.0)
            .min_height(60.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Tab 切換：日誌 / 連線歷程
                    ui.selectable_value(
                        &mut self.log_tab,
                        LogTab::Log,
                        egui::RichText::new("日誌").size(13.0),
                    );
                    let timeline_label = if has_tunnel {
                        egui::RichText::new("● 連線歷程")
                            .size(13.0)
                            .color(egui::Color32::from_rgb(0x64, 0xd8, 0x9a))
                    } else {
                        egui::RichText::new("連線歷程").size(13.0)
                    };
                    ui.selectable_value(&mut self.log_tab, LogTab::Timeline, timeline_label);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.log_tab == LogTab::Log && ui.small_button("清除").clicked() {
                            self.log_panel.clear();
                        }
                    });
                });

                match self.log_tab {
                    LogTab::Log => self.log_panel.show(ui),
                    LogTab::Timeline => {
                        crate::ui::timeline::TimelinePanel::show(
                            ui,
                            &self.connection_diagnostics,
                            self.transport,
                            self.tunnel_latency_ms.load(Ordering::Relaxed),
                        );
                    }
                }
            });

        let is_hosting = self.is_hosting();
        egui::CentralPanel::default().show(ctx, |ui| match self.current_tab {
            Tab::Lobby => {
                if self.show_connection_overlay(ui) {
                    return;
                }

                self.show_pending_action_banner(ui);

                let my_nickname = if self.config.is_configured() {
                    Some(self.config.nickname.as_str())
                } else {
                    None
                };
                // 遊戲中優先顯示 tunnel 延遲，否則顯示 lobby 延遲
                let tunnel_lat = self.tunnel_latency_ms.load(Ordering::Relaxed);
                let latency = if tunnel_lat > 0 {
                    tunnel_lat
                } else {
                    self.latency_ms.load(Ordering::Relaxed)
                };
                let action = self.lobby.show(
                    ui,
                    &self.rooms,
                    &self.players,
                    my_nickname,
                    is_hosting,
                    &self.cmd_tx,
                    latency,
                    self.transport,
                    &self.connection_diagnostics,
                );
                match action {
                    LobbyAction::None => {}
                    LobbyAction::JoinRoom { room_name } => {
                        self.pending_action = Some(PendingAction::Joining { room_name });
                    }
                    LobbyAction::CreateRoom { max_players } => {
                        // 在背景擷取 GAMEINFO 並自動偵測房間名/地圖名
                        let version = self.config.war3_version;
                        let event_tx = self.tunnel_event_tx.clone();
                        self.rt_handle.spawn(async move {
                            let gameinfo = tokio::task::spawn_blocking(move || {
                                check_room(std::net::Ipv4Addr::LOCALHOST, version)
                                    .unwrap_or_default()
                            })
                            .await
                            .unwrap_or_default();

                            // 從 GAMEINFO 自動偵測房間名和地圖名
                            let fields = war3_protocol::war3::parse_gameinfo(&gameinfo);
                            let room_name = fields
                                .as_ref()
                                .map(|f| f.game_name.clone())
                                .unwrap_or_default();
                            let map_name = fields.and_then(|f| f.map_path).unwrap_or_default();

                            let _ = event_tx.send(TunnelEvent::GameinfoCaptured {
                                room_name,
                                map_name,
                                max_players,
                                gameinfo,
                            });
                        });
                        self.pending_action = Some(PendingAction::CreatingRoom {
                            room_name: "偵測中...".to_string(),
                        });
                    }
                }
            }
            Tab::Settings => {
                crate::ui::settings::show(ui, &mut self.config, &mut self.config_changed);
            }
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

impl Drop for War3App {
    fn drop(&mut self) {
        if let Some(h) = self.tunnel_handle.take() {
            h.abort();
        }
        if let Some(h) = self.injection_handle.take() {
            h.abort();
        }
    }
}

/// SSRF 防護：拒絕 RFC1918、loopback、link-local 位址
fn is_safe_external_addr(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            if v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
            {
                return false;
            }
            // RFC 6598 CGNAT shared address space (100.64.0.0/10)
            let octets = v4.octets();
            !(octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        std::net::IpAddr::V6(v6) => {
            !v6.is_loopback()
                && !v6.is_unspecified()
                // ULA (fc00::/7) 和 link-local (fe80::/10)
                && !matches!(v6.segments()[0], 0xfc00..=0xfdff | 0xfe80..=0xfebf)
        }
    }
}

fn setup_cjk_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let font_data = include_bytes!("../assets/NotoSansTC-Regular.otf");
    fonts.font_data.insert(
        "NotoSansTC".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(font_data)),
    );

    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "NotoSansTC".to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "NotoSansTC".to_owned());

    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    // ── T8: SSRF check — RFC1918/loopback/link-local → rejected ──

    #[test]
    fn ssrf_rejects_ipv4_loopback() {
        assert!(!is_safe_external_addr(
            "127.0.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_safe_external_addr(
            "127.255.255.255".parse::<IpAddr>().unwrap()
        ));
    }

    #[test]
    fn ssrf_rejects_ipv4_private() {
        // 10.0.0.0/8
        assert!(!is_safe_external_addr(
            "10.0.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_safe_external_addr(
            "10.255.255.255".parse::<IpAddr>().unwrap()
        ));
        // 172.16.0.0/12
        assert!(!is_safe_external_addr(
            "172.16.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_safe_external_addr(
            "172.31.255.255".parse::<IpAddr>().unwrap()
        ));
        // 192.168.0.0/16
        assert!(!is_safe_external_addr(
            "192.168.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_safe_external_addr(
            "192.168.255.255".parse::<IpAddr>().unwrap()
        ));
    }

    #[test]
    fn ssrf_rejects_ipv4_link_local() {
        assert!(!is_safe_external_addr(
            "169.254.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_safe_external_addr(
            "169.254.255.255".parse::<IpAddr>().unwrap()
        ));
    }

    #[test]
    fn ssrf_rejects_ipv4_broadcast() {
        assert!(!is_safe_external_addr(
            "255.255.255.255".parse::<IpAddr>().unwrap()
        ));
    }

    #[test]
    fn ssrf_rejects_ipv4_unspecified() {
        assert!(!is_safe_external_addr("0.0.0.0".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn ssrf_rejects_cgnat_ipv4() {
        assert!(!is_safe_external_addr(
            "100.64.1.1".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_safe_external_addr(
            "100.127.255.255".parse::<IpAddr>().unwrap()
        ));
    }

    #[test]
    fn ssrf_accepts_public_ipv4() {
        assert!(is_safe_external_addr("8.8.8.8".parse::<IpAddr>().unwrap()));
        assert!(is_safe_external_addr("1.1.1.1".parse::<IpAddr>().unwrap()));
        assert!(is_safe_external_addr(
            "203.0.113.1".parse::<IpAddr>().unwrap()
        ));
    }

    #[test]
    fn ssrf_rejects_ipv6_loopback() {
        assert!(!is_safe_external_addr("::1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn ssrf_rejects_ipv6_unspecified() {
        assert!(!is_safe_external_addr("::".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn ssrf_rejects_ipv6_ula() {
        // fc00::/7 (fc00:: - fdff::)
        assert!(!is_safe_external_addr("fc00::1".parse::<IpAddr>().unwrap()));
        assert!(!is_safe_external_addr(
            "fd12:3456:789a::1".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_safe_external_addr("fdff::1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn ssrf_rejects_ipv6_link_local() {
        // fe80::/10
        assert!(!is_safe_external_addr("fe80::1".parse::<IpAddr>().unwrap()));
        assert!(!is_safe_external_addr("febf::1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn ssrf_accepts_public_ipv6() {
        assert!(is_safe_external_addr(
            "2001:db8::1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_safe_external_addr(
            "2607:f8b0:4004:800::200e".parse::<IpAddr>().unwrap()
        ));
    }

    // ── T3: Malformed external_addr → parse 失敗，被忽略 ──

    #[test]
    fn malformed_external_addr_parse_fails() {
        // 空字串
        assert!("".parse::<SocketAddr>().is_err());
        // 非數字
        assert!("not-an-address".parse::<SocketAddr>().is_err());
        // 只有 IP 沒有 port
        assert!("192.168.1.1".parse::<SocketAddr>().is_err());
        // IPv6 without brackets
        assert!("::1".parse::<SocketAddr>().is_err());
    }

    #[test]
    fn valid_external_addr_but_unsafe_is_rejected() {
        // 可以 parse 但不安全
        let addr: SocketAddr = "192.168.1.1:19870".parse().unwrap();
        assert!(!is_safe_external_addr(addr.ip()));
        let addr: SocketAddr = "127.0.0.1:19870".parse().unwrap();
        assert!(!is_safe_external_addr(addr.ip()));
    }

    #[test]
    fn valid_external_addr_public_is_accepted() {
        let addr: SocketAddr = "203.0.113.50:19870".parse().unwrap();
        assert!(is_safe_external_addr(addr.ip()));
    }
}
