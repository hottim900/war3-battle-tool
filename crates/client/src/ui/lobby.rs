use eframe::egui;
use war3_protocol::messages::{ClientMessage, PlayerInfo, RoomInfo};

use crate::net::quic::{StrategyOutcome, StrategyResult};
use crate::net::tunnel::Transport;

/// Action returned from `LobbyPanel::show` so the app can track pending state.
pub enum LobbyAction {
    None,
    JoinRoom { room_name: String },
    CreateRoom { max_players: u8 },
}

/// 大廳畫面：上方房間列表，下方線上玩家
pub struct LobbyPanel {
    pub create_max_players: u8,
    pub show_create_dialog: bool,
    pub show_diagnostics: bool,
}

impl LobbyPanel {
    pub fn new() -> Self {
        Self {
            create_max_players: 4,
            show_create_dialog: false,
            show_diagnostics: false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        rooms: &[RoomInfo],
        players: &[PlayerInfo],
        my_nickname: Option<&str>,
        is_hosting: bool,
        cmd_tx: &tokio::sync::mpsc::UnboundedSender<ClientMessage>,
        latency_ms: u64,
        transport: Option<Transport>,
        diagnostics: &[StrategyResult],
    ) -> LobbyAction {
        let mut action = LobbyAction::None;

        // 房間區塊
        ui.horizontal(|ui| {
            ui.heading("房間列表");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if latency_ms > 0 {
                    let suffix = match transport {
                        Some(Transport::Direct) => " (direct)",
                        Some(Transport::Relay) => " (relay)",
                        None => "",
                    };
                    let color = if latency_ms < 30 {
                        egui::Color32::from_rgb(100, 200, 100) // 綠
                    } else if latency_ms < 80 {
                        egui::Color32::from_rgb(255, 200, 100) // 黃
                    } else {
                        egui::Color32::from_rgb(255, 100, 100) // 紅
                    };
                    ui.colored_label(color, format!("延遲: {latency_ms}ms{suffix}"));
                }
            });
        });
        ui.separator();

        if rooms.is_empty() {
            ui.label("目前沒有房間。建立一個來開始對戰！");
        } else {
            egui::ScrollArea::vertical()
                .id_salt("rooms_scroll")
                .max_height(250.0)
                .show(ui, |ui| {
                    egui::Grid::new("room_grid")
                        .num_columns(5)
                        .striped(true)
                        .spacing([10.0, 6.0])
                        .show(ui, |ui| {
                            // 表頭
                            ui.strong("房間");
                            ui.strong("房主");
                            ui.strong("地圖");
                            ui.strong("人數");
                            ui.strong("");
                            ui.end_row();

                            for room in rooms {
                                ui.label(&room.room_name);
                                ui.label(&room.host_nickname);
                                ui.label(&room.map_name);
                                ui.label(format!("{}/{}", room.current_players, room.max_players));

                                let is_mine = my_nickname
                                    .map(|name| room.host_nickname == name)
                                    .unwrap_or(false);

                                if is_mine {
                                    if ui.button("複製連結").clicked() {
                                        let link = format!(
                                            "https://war3.kalthor.cc/join?room={}",
                                            room.room_id
                                        );
                                        ui.ctx().copy_text(link);
                                    }
                                } else if ui.button("加入").clicked() {
                                    let _ = cmd_tx.send(ClientMessage::JoinRoom {
                                        room_id: room.room_id.clone(),
                                    });
                                    action = LobbyAction::JoinRoom {
                                        room_name: room.room_name.clone(),
                                    };
                                }
                                ui.end_row();
                            }
                        });
                });
        }

        ui.add_space(10.0);

        // 建房 / 關房按鈕
        ui.horizontal(|ui| {
            if is_hosting {
                if ui.button("關閉房間").clicked() {
                    let _ = cmd_tx.send(ClientMessage::CloseRoom);
                }
            } else if ui.button("建立房間").clicked() {
                self.show_create_dialog = true;
            }
        });

        // 建房對話框（房間名和地圖名從 War3 自動偵測）
        if self.show_create_dialog
            && let Some(create_action) = self.show_create_room_dialog(ui)
        {
            action = create_action;
        }

        // 連線詳情面板（有診斷資料時顯示）
        if !diagnostics.is_empty() {
            ui.add_space(10.0);
            let label = if self.show_diagnostics {
                "▼ 連線詳情"
            } else {
                "▶ 連線詳情"
            };
            if ui.selectable_label(self.show_diagnostics, label).clicked() {
                self.show_diagnostics = !self.show_diagnostics;
            }

            if self.show_diagnostics {
                egui::Frame::new()
                    .fill(ui.style().visuals.extreme_bg_color)
                    .inner_margin(8.0)
                    .corner_radius(4.0)
                    .show(ui, |ui| {
                        egui::Grid::new("diagnostics_grid")
                            .num_columns(3)
                            .striped(true)
                            .spacing([10.0, 4.0])
                            .show(ui, |ui| {
                                ui.strong("策略");
                                ui.strong("結果");
                                ui.strong("耗時");
                                ui.end_row();

                                for r in diagnostics {
                                    ui.label(r.method.to_string());
                                    match &r.outcome {
                                        StrategyOutcome::Success => {
                                            ui.colored_label(
                                                egui::Color32::from_rgb(100, 200, 100),
                                                format!("✓ 成功 ({}ms)", r.duration_ms),
                                            );
                                        }
                                        StrategyOutcome::Failed(reason) => {
                                            ui.colored_label(
                                                egui::Color32::from_rgb(255, 100, 100),
                                                format!("✗ {reason}"),
                                            );
                                        }
                                        StrategyOutcome::Skipped => {
                                            ui.weak("— 跳過");
                                        }
                                    }
                                    if r.duration_ms > 0 {
                                        ui.label(format!("{}ms", r.duration_ms));
                                    } else {
                                        ui.weak("—");
                                    }
                                    ui.end_row();
                                }
                            });

                        // 目前連線狀態
                        if let Some(t) = transport {
                            ui.add_space(4.0);
                            let (color, text) = match t {
                                Transport::Direct => (
                                    egui::Color32::from_rgb(100, 200, 100),
                                    format!("目前連線: QUIC 直連 ({latency_ms}ms)"),
                                ),
                                Transport::Relay => (
                                    egui::Color32::from_rgb(255, 200, 100),
                                    format!("目前連線: Relay 中繼 ({latency_ms}ms)"),
                                ),
                            };
                            ui.colored_label(color, text);
                        }
                    });
            }
        }

        ui.add_space(20.0);
        ui.separator();

        // 線上玩家
        ui.heading("線上玩家");
        ui.add_space(4.0);

        if players.is_empty() {
            ui.label("目前沒有人在線上。");
        } else {
            egui::ScrollArea::vertical()
                .id_salt("players_scroll")
                .max_height(150.0)
                .show(ui, |ui| {
                    for player in players {
                        ui.horizontal(|ui| {
                            ui.label(&player.nickname);
                            ui.weak(format!("({})", player.war3_version.as_str()));
                            if player.is_hosting {
                                ui.colored_label(
                                    egui::Color32::from_rgb(100, 200, 100),
                                    "🏠 建房中",
                                );
                            }
                        });
                    }
                });
        }

        action
    }

    fn show_create_room_dialog(&mut self, ui: &mut egui::Ui) -> Option<LobbyAction> {
        let mut result: Option<LobbyAction> = None;

        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.heading("建立房間");
            ui.add_space(4.0);
            ui.label("請先在 War3 建立遊戲，再按建立。");
            ui.label("房間名稱和地圖名稱會自動偵測。");
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label("最大玩家數：");
                ui.add(egui::Slider::new(&mut self.create_max_players, 2..=12));
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("建立").clicked() {
                    result = Some(LobbyAction::CreateRoom {
                        max_players: self.create_max_players,
                    });
                    self.show_create_dialog = false;
                }
                if ui.button("取消").clicked() {
                    self.show_create_dialog = false;
                }
            });
        });

        result
    }
}
