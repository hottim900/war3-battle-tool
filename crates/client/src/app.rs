use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use eframe::egui;
use tokio::sync::mpsc;
use war3_protocol::messages::{ClientMessage, PlayerInfo, RoomInfo, ServerMessage};

use crate::net::discovery::NetEvent;
use crate::net::packet::{RawUdpInjector, check_room};
use crate::net::tunnel::{self, Transport, TunnelEvent};
use crate::ui::lobby::{LobbyAction, LobbyPanel};
use crate::ui::log_panel::LogPanel;
use crate::ui::setup_wizard::SetupWizard;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tab {
    Lobby,
    Settings,
    Log,
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

    pending_action: Option<PendingAction>,

    /// Pending GAMEINFO for injection (set when JoinResult received)
    pending_gameinfo: Option<Vec<u8>>,
    /// Handle to the GAMEINFO injection task (for cancellation)
    injection_handle: Option<tokio::task::JoinHandle<()>>,
    /// Handle to the tunnel task (for cancellation on re-join or cleanup)
    tunnel_handle: Option<tokio::task::JoinHandle<()>>,

    /// Lobby RTT 測量（ms），由 discovery 更新
    latency_ms: Arc<AtomicU64>,

    /// P2P 直連：對方 IP（從 StunInfo 接收）
    peer_addr: Option<std::net::IpAddr>,
    /// 目前遊戲傳輸路徑（relay 或 direct）
    transport: Option<Transport>,
}

impl War3App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: crate::config::AppConfig,
        cmd_tx: mpsc::UnboundedSender<ClientMessage>,
        event_rx: mpsc::UnboundedReceiver<NetEvent>,
        rt_handle: tokio::runtime::Handle,
        server_url: String,
        latency_ms: Arc<AtomicU64>,
    ) -> Self {
        setup_cjk_fonts(&cc.egui_ctx);

        let needs_wizard = !config.is_configured();
        let (tunnel_event_tx, tunnel_event_rx) = mpsc::unbounded_channel();

        let mut app = Self {
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
            log_panel: LogPanel::new(),
            pending_action: None,
            pending_gameinfo: None,
            injection_handle: None,
            tunnel_handle: None,
            latency_ms,
            peer_addr: None,
            transport: None,
        };

        app.log_panel.info("War3 Battle Tool 啟動");
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

        let server_url = self.server_url.clone();
        let event_tx = self.tunnel_event_tx.clone();
        let peer_addr = self.peer_addr.take();

        let handle = self.rt_handle.spawn(async move {
            tunnel::run_joiner_tunnel(server_url, tunnel_token, peer_addr, event_tx).await;
        });
        self.tunnel_handle = Some(handle);

        // 存 GAMEINFO，等 ProxyReady 後開始注入
        self.pending_gameinfo = Some(gameinfo);
    }

    /// 啟動 host 端 tunnel
    fn start_host_tunnel(&mut self, tunnel_token: String) {
        self.abort_tunnel();

        let server_url = self.server_url.clone();
        let event_tx = self.tunnel_event_tx.clone();
        let peer_addr = self.peer_addr.take();

        let handle = self.rt_handle.spawn(async move {
            tunnel::run_host_tunnel(server_url, tunnel_token, peer_addr, event_tx).await;
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
                self.log_panel.warn("沒有 GAMEINFO 可注入");
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

        self.log_panel
            .info("GAMEINFO 注入開始，請切換到 War3 區域網路畫面加入遊戲");
    }

    fn process_network_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                NetEvent::Connected => {
                    self.connection_state = ConnectionState::Connected;
                    self.ever_connected = true;
                    self.log_panel.info("已連線到發現伺服器");

                    if !self.is_registered() && self.config.is_configured() {
                        self.send_register();
                    }
                }
                NetEvent::Disconnected => {
                    self.connection_state = ConnectionState::Disconnected;
                    self.my_player_id = None;
                    self.log_panel.warn("與伺服器的連線中斷");
                }
                NetEvent::Reconnecting { attempt } => {
                    self.connection_state = ConnectionState::Reconnecting { attempt };
                    self.log_panel
                        .info(format!("正在重新連線... (第 {attempt} 次)"));
                }
                NetEvent::ServerMessage(msg) => self.handle_server_message(msg),
            }
        }

        // 處理 tunnel 事件
        while let Ok(event) = self.tunnel_event_rx.try_recv() {
            match event {
                TunnelEvent::ProxyReady => {
                    self.log_panel.info("Tunnel proxy 就緒");
                    self.start_gameinfo_injection();
                }
                TunnelEvent::TransportSelected(t) => {
                    self.transport = Some(t);
                    match t {
                        Transport::Direct => self.log_panel.info("傳輸: P2P 直連"),
                        Transport::Relay => self.log_panel.info("傳輸: Relay 中繼"),
                    }
                }
                TunnelEvent::Finished { error: None } => {
                    if let Some(h) = self.injection_handle.take() {
                        h.abort();
                    }
                    self.tunnel_handle = None;
                    self.transport = None;
                    self.log_panel.info("Tunnel 連線結束");
                }
                TunnelEvent::Finished { error: Some(e) } => {
                    if let Some(h) = self.injection_handle.take() {
                        h.abort();
                    }
                    self.tunnel_handle = None;
                    self.transport = None;
                    self.log_panel.error(format!("Tunnel 錯誤: {e}"));
                }
                TunnelEvent::GameinfoCaptured {
                    room_name,
                    map_name,
                    max_players,
                    gameinfo,
                } => {
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

    fn handle_server_message(&mut self, msg: ServerMessage) {
        match msg {
            ServerMessage::Welcome { player_id } => {
                self.my_player_id = Some(player_id);
                self.log_panel.info("註冊成功");
            }
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
                        self.log_panel.info("加入成功！正在建立 tunnel 連線...");
                        self.start_joiner_tunnel(token, gi);
                    } else {
                        self.log_panel.warn("加入成功但缺少 tunnel 資訊");
                    }
                } else {
                    self.pending_action = Some(PendingAction::JoinFailed {
                        reason: "房間可能已關閉".to_string(),
                    });
                    self.log_panel.error("加入失敗，房間可能已關閉。");
                }
            }
            ServerMessage::PlayerJoined {
                nickname,
                tunnel_token,
            } => {
                self.log_panel
                    .info(format!("玩家 {nickname} 加入，建立 tunnel..."));
                self.start_host_tunnel(tunnel_token);
            }
            ServerMessage::TunnelReady { tunnel_token } => {
                self.log_panel.info("Tunnel 就緒，建立連線...");
                self.start_host_tunnel(tunnel_token);
            }
            ServerMessage::StunInfo { peer_addr } => {
                if let Ok(ip) = peer_addr.parse::<std::net::IpAddr>() {
                    self.peer_addr = Some(ip);
                    self.log_panel.info("收到 P2P 直連資訊");
                }
            }
            ServerMessage::Pong { .. } => {
                // 已在 discovery.rs 處理，不會到這裡
            }
            ServerMessage::Error { message } => {
                self.log_panel.error(format!("伺服器錯誤: {message}"));
            }
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
                        egui::Color32::from_rgb(255, 200, 100),
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
                    .fill(egui::Color32::from_rgba_premultiplied(40, 100, 40, 200))
                    .inner_margin(8.0)
                    .corner_radius(4.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                egui::Color32::from_rgb(100, 255, 100),
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
                    .fill(egui::Color32::from_rgba_premultiplied(100, 40, 40, 200))
                    .inner_margin(8.0)
                    .corner_radius(4.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                egui::Color32::from_rgb(255, 100, 100),
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
                ConnectionState::Connected => (egui::Color32::from_rgb(100, 200, 100), "● 已連線"),
                ConnectionState::Disconnected => {
                    (egui::Color32::from_rgb(200, 100, 100), "● 已斷線")
                }
                ConnectionState::Reconnecting { attempt: _ } => {
                    (egui::Color32::from_rgb(255, 200, 100), "● 重連中...")
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
                ui.selectable_value(&mut self.current_tab, Tab::Log, "日誌");
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            self.show_status_bar(ui);
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
                let latency = self.latency_ms.load(Ordering::Relaxed);
                let action = self.lobby.show(
                    ui,
                    &self.rooms,
                    &self.players,
                    my_nickname,
                    is_hosting,
                    &self.cmd_tx,
                    latency,
                    self.transport,
                );
                match action {
                    LobbyAction::None => {}
                    LobbyAction::JoinRoom { room_name } => {
                        self.pending_action = Some(PendingAction::Joining { room_name });
                    }
                    LobbyAction::CreateRoom {
                        room_name,
                        map_name,
                        max_players,
                    } => {
                        // 在背景擷取 GAMEINFO（blocking UDP call），避免凍結 UI
                        let version = self.config.war3_version;
                        let event_tx = self.tunnel_event_tx.clone();
                        let rn = room_name.clone();
                        self.rt_handle.spawn(async move {
                            let gameinfo = tokio::task::spawn_blocking(move || {
                                check_room(std::net::Ipv4Addr::LOCALHOST, version)
                                    .unwrap_or_default()
                            })
                            .await
                            .unwrap_or_default();

                            let _ = event_tx.send(TunnelEvent::GameinfoCaptured {
                                room_name: rn,
                                map_name,
                                max_players,
                                gameinfo,
                            });
                        });
                        self.pending_action = Some(PendingAction::CreatingRoom { room_name });
                    }
                }
            }
            Tab::Settings => {
                crate::ui::settings::show(ui, &mut self.config, &mut self.config_changed);
            }
            Tab::Log => {
                self.log_panel.show(ui);
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
