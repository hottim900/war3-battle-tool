use std::sync::Arc;

use eframe::egui;
use tokio::sync::mpsc;
use war3_protocol::messages::{ClientMessage, PlayerInfo, RoomInfo, ServerMessage};

use crate::config::AppConfig;
use crate::net::discovery::NetEvent;
use crate::net::packet::PacketSender;
use crate::ui::lobby::LobbyPanel;
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

pub struct War3App {
    config: AppConfig,
    config_changed: bool,

    cmd_tx: mpsc::UnboundedSender<ClientMessage>,
    event_rx: mpsc::UnboundedReceiver<NetEvent>,
    packet_sender: Option<Arc<dyn PacketSender>>,

    connection_state: ConnectionState,
    my_player_id: Option<String>,

    players: Vec<PlayerInfo>,
    rooms: Vec<RoomInfo>,

    current_tab: Tab,
    wizard: Option<SetupWizard>,
    lobby: LobbyPanel,
    log_panel: LogPanel,
}

impl War3App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: AppConfig,
        cmd_tx: mpsc::UnboundedSender<ClientMessage>,
        event_rx: mpsc::UnboundedReceiver<NetEvent>,
        packet_sender: Option<Arc<dyn PacketSender>>,
    ) -> Self {
        setup_cjk_fonts(&cc.egui_ctx);

        let needs_wizard = !config.is_configured();

        let mut app = Self {
            config,
            config_changed: false,
            cmd_tx,
            event_rx,
            packet_sender,
            connection_state: ConnectionState::Disconnected,
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
        });
    }

    /// 玩家端：收到房主 IP 後，取 game info 並注入到本地 War3
    fn try_inject_join(&mut self, host_ip_str: &str) {
        let sender = match &self.packet_sender {
            Some(s) => s.clone(),
            None => {
                self.log_panel
                    .warn("封包注入未啟用（缺少 npcap），請手動加入 LAN 遊戲");
                return;
            }
        };

        let host_ip: std::net::Ipv4Addr = match host_ip_str.parse() {
            Ok(ip) => ip,
            Err(_) => {
                self.log_panel.error(format!("無效的 IP: {host_ip_str}"));
                return;
            }
        };

        let version = self.config.war3_version;
        let local_ip = std::net::Ipv4Addr::LOCALHOST;

        match crate::net::packet::join_room(&*sender, host_ip, local_ip, version) {
            Ok(()) => {
                self.log_panel
                    .info("封包注入成功！請切換到 War3 區域網路畫面");
            }
            Err(e) => {
                self.log_panel
                    .error(format!("封包注入失敗: {e}"));
            }
        }
    }

    /// 房主端：有玩家加入時，模擬遠端玩家的 broadcast 讓本機 War3 回應
    fn try_inject_invite(&mut self, player_ip_str: &str) {
        let sender = match &self.packet_sender {
            Some(s) => s.clone(),
            None => return, // 沒有 npcap，靜默跳過
        };

        let player_ip: std::net::Ipv4Addr = match player_ip_str.parse() {
            Ok(ip) => ip,
            Err(_) => return,
        };

        let version = self.config.war3_version;
        let local_ip = std::net::Ipv4Addr::LOCALHOST;

        if let Err(e) = crate::net::packet::invite_player(&*sender, player_ip, local_ip, version) {
            self.log_panel.warn(format!("邀請封包失敗: {e}"));
        }
    }

    fn process_network_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                NetEvent::Connected => {
                    self.connection_state = ConnectionState::Connected;
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
                self.rooms = rooms;
            }
            ServerMessage::JoinResult { success, host_ip } => {
                if success {
                    if let Some(ip) = host_ip {
                        self.log_panel
                            .info(format!("取得房主 IP: {ip}，正在注入封包..."));
                        self.try_inject_join(&ip);
                    }
                } else {
                    self.log_panel.error("加入失敗，房間可能已關閉。");
                }
            }
            ServerMessage::PlayerJoined {
                nickname,
                player_ip,
            } => {
                self.log_panel
                    .info(format!("玩家 {nickname} 加入，正在邀請... (IP: {player_ip})"));
                self.try_inject_invite(&player_ip);
            }
            ServerMessage::Error { message } => {
                self.log_panel.error(format!("伺服器錯誤: {message}"));
            }
        }
    }

    fn show_status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (color, text) = match &self.connection_state {
                ConnectionState::Connected => {
                    (egui::Color32::from_rgb(100, 200, 100), "● 已連線")
                }
                ConnectionState::Disconnected => {
                    (egui::Color32::from_rgb(200, 100, 100), "● 已斷線")
                }
                ConnectionState::Reconnecting { attempt: _ } => (
                    egui::Color32::from_rgb(255, 200, 100),
                    "● 重連中...",
                ),
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
                let my_nickname = if self.config.is_configured() {
                    Some(self.config.nickname.as_str())
                } else {
                    None
                };
                self.lobby.show(
                    ui,
                    &self.rooms,
                    &self.players,
                    my_nickname,
                    is_hosting,
                    &self.cmd_tx,
                );
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
