use eframe::egui;
use tokio::sync::mpsc;
use war3_protocol::messages::{PlayerInfo, RoomInfo, ServerMessage};

use crate::config::AppConfig;
use crate::net::discovery::{NetEvent, UiCommand};
use crate::ui::lobby::LobbyPanel;
use crate::ui::log_panel::LogPanel;
use crate::ui::setup_wizard::SetupWizard;

/// 主要頁籤
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tab {
    Lobby,
    Settings,
    Log,
}

/// 連線狀態
#[derive(Debug, Clone)]
enum ConnectionState {
    Disconnected,
    Connected,
    Reconnecting { attempt: u32 },
}

pub struct War3App {
    config: AppConfig,
    config_changed: bool,

    // 網路通道
    cmd_tx: mpsc::UnboundedSender<UiCommand>,
    event_rx: mpsc::UnboundedReceiver<NetEvent>,

    // 連線狀態
    connection_state: ConnectionState,
    my_player_id: Option<String>,
    registered: bool,

    // 遊戲資料
    players: Vec<PlayerInfo>,
    rooms: Vec<RoomInfo>,
    is_hosting: bool,

    // UI 元件
    current_tab: Tab,
    wizard: Option<SetupWizard>,
    lobby: LobbyPanel,
    log_panel: LogPanel,
}

impl War3App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: AppConfig,
        cmd_tx: mpsc::UnboundedSender<UiCommand>,
        event_rx: mpsc::UnboundedReceiver<NetEvent>,
    ) -> Self {
        setup_cjk_fonts(&cc.egui_ctx);

        let needs_wizard = !config.is_configured();

        let mut app = Self {
            config,
            config_changed: false,
            cmd_tx,
            event_rx,
            connection_state: ConnectionState::Disconnected,
            my_player_id: None,
            registered: false,
            players: Vec::new(),
            rooms: Vec::new(),
            is_hosting: false,
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

    fn process_network_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                NetEvent::Connected => {
                    self.connection_state = ConnectionState::Connected;
                    self.log_panel.info("已連線到發現伺服器");

                    // 自動註冊
                    if !self.registered && self.config.is_configured() {
                        let _ = self.cmd_tx.send(UiCommand::Register {
                            nickname: self.config.nickname.clone(),
                            war3_version: self.config.war3_version,
                        });
                    }
                }
                NetEvent::Disconnected => {
                    self.connection_state = ConnectionState::Disconnected;
                    self.registered = false;
                    self.my_player_id = None;
                    self.is_hosting = false;
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
                self.registered = true;
                self.log_panel.info("註冊成功");
            }
            ServerMessage::PlayerUpdate { players } => {
                self.players = players;
            }
            ServerMessage::RoomUpdate { rooms } => {
                // 檢查自己是否還在 hosting
                self.is_hosting = self
                    .my_player_id
                    .as_ref()
                    .map(|my_id| {
                        self.players
                            .iter()
                            .any(|p| p.player_id == *my_id && p.is_hosting)
                    })
                    .unwrap_or(false);
                self.rooms = rooms;
            }
            ServerMessage::JoinResult { success, host_ip } => {
                if success {
                    if let Some(ip) = host_ip {
                        self.log_panel
                            .info(format!("加入成功！房主 IP: {ip}（請切換到 War3 區域網路畫面）"));
                    }
                } else {
                    self.log_panel
                        .error("加入失敗，房間可能已關閉。");
                }
            }
            ServerMessage::PlayerJoined {
                nickname,
                player_ip,
            } => {
                self.log_panel
                    .info(format!("玩家 {nickname} 加入你的房間 (IP: {player_ip})"));
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
                    ui.weak(format!("ID: {}…", &id[..8]));
                });
            }
        });
    }
}

impl eframe::App for War3App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 處理網路事件
        self.process_network_events();

        // 首次設定精靈
        if let Some(wizard) = &mut self.wizard {
            wizard.show(ctx);
            if wizard.done {
                self.config.nickname = wizard.nickname.clone();
                self.config.war3_version = wizard.war3_version;
                let _ = self.config.save();
                self.wizard = None;

                // 觸發註冊
                if matches!(self.connection_state, ConnectionState::Connected) {
                    let _ = self.cmd_tx.send(UiCommand::Register {
                        nickname: self.config.nickname.clone(),
                        war3_version: self.config.war3_version,
                    });
                }
            }
            // 持續 repaint 以接收網路事件
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

        egui::CentralPanel::default().show(ctx, |ui| match self.current_tab {
            Tab::Lobby => {
                self.lobby.show(
                    ui,
                    &self.rooms,
                    &self.players,
                    self.my_player_id.as_deref(),
                    self.is_hosting,
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

        // 定期 repaint 以接收網路事件
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

    // 加到 Proportional 和 Monospace 的第一順位
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
