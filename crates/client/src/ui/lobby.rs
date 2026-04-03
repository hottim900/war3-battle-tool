use eframe::egui;
use war3_protocol::messages::{PlayerInfo, RoomInfo};

use war3_protocol::messages::ClientMessage;

/// 大廳畫面：上方房間列表，下方線上玩家
pub struct LobbyPanel {
    // 建房表單
    pub create_room_name: String,
    pub create_map_name: String,
    pub create_max_players: u8,
    pub show_create_dialog: bool,
}

impl LobbyPanel {
    pub fn new() -> Self {
        Self {
            create_room_name: String::new(),
            create_map_name: String::new(),
            create_max_players: 4,
            show_create_dialog: false,
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        rooms: &[RoomInfo],
        players: &[PlayerInfo],
        my_nickname: Option<&str>,
        is_hosting: bool,
        cmd_tx: &tokio::sync::mpsc::UnboundedSender<ClientMessage>,
    ) {
        // 房間區塊
        ui.heading("房間列表");
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
                                ui.label(format!(
                                    "{}/{}",
                                    room.current_players, room.max_players
                                ));

                                let is_mine = my_nickname
                                    .map(|name| room.host_nickname == name)
                                    .unwrap_or(false);

                                if is_mine {
                                    ui.label("(你的房間)");
                                } else if ui.button("加入").clicked() {
                                    let _ = cmd_tx.send(ClientMessage::JoinRoom {
                                        room_id: room.room_id.clone(),
                                    });
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

        // 建房對話框
        if self.show_create_dialog {
            self.show_create_room_dialog(ui, cmd_tx);
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
    }

    fn show_create_room_dialog(
        &mut self,
        ui: &mut egui::Ui,
        cmd_tx: &tokio::sync::mpsc::UnboundedSender<ClientMessage>,
    ) {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.heading("建立房間");
            ui.add_space(8.0);

            egui::Grid::new("create_room_grid")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label("房間名稱：");
                    ui.text_edit_singleline(&mut self.create_room_name);
                    ui.end_row();

                    ui.label("地圖名稱：");
                    ui.text_edit_singleline(&mut self.create_map_name);
                    ui.end_row();

                    ui.label("最大玩家數：");
                    ui.add(egui::Slider::new(&mut self.create_max_players, 2..=12));
                    ui.end_row();
                });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let can_create = !self.create_room_name.trim().is_empty();
                if ui
                    .add_enabled(can_create, egui::Button::new("建立"))
                    .clicked()
                {
                    let _ = cmd_tx.send(ClientMessage::CreateRoom {
                        room_name: self.create_room_name.clone(),
                        map_name: self.create_map_name.clone(),
                        max_players: self.create_max_players,
                    });
                    self.show_create_dialog = false;
                    self.create_room_name.clear();
                    self.create_map_name.clear();
                }
                if ui.button("取消").clicked() {
                    self.show_create_dialog = false;
                }
            });
        });
    }
}
