use eframe::egui;
use war3_protocol::messages::{ClientMessage, PlayerInfo, RoomInfo};

use crate::cmd_sender::CmdSender;
use crate::net::quic::{StrategyOutcome, StrategyResult};
use crate::net::tunnel::Transport;

/// Action returned from `LobbyPanel::show` so the app can track pending state.
pub enum LobbyAction {
    None,
    JoinRoom { room_name: String },
    CreateRoom { max_players: u8 },
}

/// 從 server_url 推導 web viewer base URL。
///
/// 自架 server 的人 ship 自家 client 給朋友時，「複製連結」按鈕產生的 URL
/// 必須對齊他們設定的 `SERVER_URL`，否則朋友收到的 link 永遠指向 production
/// `war3.kalthor.cc`（self-host bug）。
///
/// 規則：scheme 把 ws/wss 換成 http/https，剝掉 `/ws` 等 path、`?query`、
/// `#fragment`，**並剝掉 `user:pass@` userinfo 避免 credentials 進到貼給朋友的連結**
/// （若 user 把 token 塞到 SERVER_URL，舊實作會原樣帶到剪貼簿）。
/// malformed 或非 ws/wss/http/https → fallback production URL，保證舊行為不退化。
pub fn web_viewer_base_url(server_url: &str) -> String {
    let (scheme, rest) = if let Some(r) = server_url.strip_prefix("wss://") {
        ("https", r)
    } else if let Some(r) = server_url.strip_prefix("ws://") {
        ("http", r)
    } else if let Some(r) = server_url.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = server_url.strip_prefix("http://") {
        ("http", r)
    } else {
        return "https://war3.kalthor.cc".to_string();
    };
    // 順序：先 path（/）→ 再 query/fragment（?#）→ 最後 userinfo（@），
    // 因為 query/fragment 可能在 authority 後不經 path 直接出現
    // （RFC 3986 `authority [ "?" query ] [ "#" fragment ]`）。
    let host_port = rest.split('/').next().unwrap_or(rest);
    let host_port = host_port.split(['?', '#']).next().unwrap_or(host_port);
    // userinfo: `user:pass@host` → rsplit 取 `@` 之後（IPv6 brackets 內 host 不含 `@`）
    let host_port = host_port.rsplit('@').next().unwrap_or(host_port);
    if host_port.is_empty() {
        return "https://war3.kalthor.cc".to_string();
    }
    format!("{scheme}://{host_port}")
}

/// 大廳畫面：上方房間列表，下方線上玩家
pub struct LobbyPanel {
    pub create_max_players: u8,
    pub show_create_dialog: bool,
    pub show_diagnostics: bool,
    /// Derived from server_url at construction; used for "複製連結" 按鈕
    viewer_base_url: String,
}

impl LobbyPanel {
    pub fn new(viewer_base_url: String) -> Self {
        Self {
            create_max_players: 4,
            show_create_dialog: false,
            show_diagnostics: false,
            viewer_base_url,
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
        cmd_tx: &CmdSender,
        latency_ms: u64,
        transport: Option<Transport>,
        diagnostics: &[StrategyResult],
    ) -> LobbyAction {
        let mut action = LobbyAction::None;

        // 房間區塊
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("房間列表")
                    .size(13.0)
                    .color(egui::Color32::from_rgb(0x88, 0x92, 0xb0)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if latency_ms > 0 {
                    let suffix = match transport {
                        Some(Transport::Direct) => " (直連)",
                        Some(Transport::Relay) => " (中繼)",
                        None => "",
                    };
                    let color = if latency_ms < 30 {
                        egui::Color32::from_rgb(0x64, 0xd8, 0x9a)
                    } else if latency_ms < 80 {
                        egui::Color32::from_rgb(0xf5, 0x9e, 0x0b)
                    } else {
                        egui::Color32::from_rgb(0xef, 0x44, 0x44)
                    };
                    ui.colored_label(color, format!("延遲: {latency_ms}ms{suffix}"));
                }
            });
        });

        if rooms.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new("目前沒有房間")
                        .size(14.0)
                        .color(egui::Color32::from_rgb(0x88, 0x92, 0xb0)),
                );
            });
            ui.add_space(20.0);
        } else {
            egui::ScrollArea::vertical()
                .id_salt("rooms_scroll")
                .max_height(250.0)
                .show(ui, |ui| {
                    for room in rooms {
                        let is_mine = my_nickname
                            .map(|name| room.host_nickname == name)
                            .unwrap_or(false);
                        let room_full = room.current_players >= room.max_players;

                        // 自己的房間用左邊框強調
                        let border_stroke = if is_mine {
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x3b, 0x82, 0xf6))
                        } else {
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x1e, 0x29, 0x3b))
                        };

                        egui::Frame::new()
                            .fill(ui.visuals().window_fill)
                            .stroke(border_stroke)
                            .inner_margin(12.0)
                            .corner_radius(8.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&room.host_nickname)
                                                    .size(15.0)
                                                    .strong()
                                                    .color(egui::Color32::from_rgb(
                                                        0xcc, 0xd6, 0xf6,
                                                    )),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(&room.host_nickname);
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&room.map_name)
                                                    .size(13.0)
                                                    .color(egui::Color32::from_rgb(
                                                        0x88, 0x92, 0xb0,
                                                    )),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(&room.map_name);
                                    });

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if is_mine {
                                                if ui.button("複製連結").clicked() {
                                                    let link = format!(
                                                        "{}/join?room={}",
                                                        self.viewer_base_url, room.room_id
                                                    );
                                                    ui.ctx().copy_text(link);
                                                }
                                            } else if room_full {
                                                ui.add_enabled(false, egui::Button::new("已滿"));
                                            } else if ui.button("加入").clicked() {
                                                let sent = cmd_tx.send_or_warn(
                                                    ClientMessage::JoinRoom {
                                                        room_id: room.room_id.clone(),
                                                    },
                                                    "加入房間",
                                                );
                                                if sent {
                                                    action = LobbyAction::JoinRoom {
                                                        room_name: room.room_name.clone(),
                                                    };
                                                }
                                            }

                                            let cur = room.current_players as u32;
                                            let max = room.max_players as u32;
                                            let badge_color = if cur >= max {
                                                egui::Color32::from_rgb(0xef, 0x44, 0x44)
                                            } else if cur * 4 >= max * 3 {
                                                egui::Color32::from_rgb(0xf5, 0x9e, 0x0b)
                                            } else {
                                                egui::Color32::from_rgb(0x64, 0xd8, 0x9a)
                                            };

                                            egui::Frame::new()
                                                .fill(egui::Color32::from_rgb(0x1e, 0x29, 0x3b))
                                                .inner_margin(egui::Margin::symmetric(10, 3))
                                                .corner_radius(12.0)
                                                .show(ui, |ui| {
                                                    ui.label(
                                                        egui::RichText::new(format!(
                                                            "{}/{}",
                                                            room.current_players, room.max_players
                                                        ))
                                                        .size(12.0)
                                                        .color(badge_color),
                                                    );
                                                });
                                        },
                                    );
                                });
                            });
                        ui.add_space(6.0);
                    }
                });
        }

        ui.add_space(10.0);

        // 建房 / 關房按鈕
        if is_hosting {
            if ui.button("關閉房間").clicked() {
                // 失敗的話 server 端房間還在但 client 已斷線，warn 已由 send_or_warn 記錄
                let _ = cmd_tx.send_or_warn(ClientMessage::CloseRoom, "關閉房間");
            }
        } else {
            let create_btn = egui::Button::new("+ 建立房間")
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgb(0x33, 0x41, 0x55),
                ))
                .corner_radius(8.0);
            if ui
                .add_sized([ui.available_width(), 32.0], create_btn)
                .clicked()
            {
                self.show_create_dialog = true;
            }
        }

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
                                                egui::Color32::from_rgb(0x64, 0xd8, 0x9a),
                                                format!("✓ 成功 ({}ms)", r.duration_ms),
                                            );
                                        }
                                        StrategyOutcome::Failed(reason) => {
                                            ui.colored_label(
                                                egui::Color32::from_rgb(0xef, 0x44, 0x44),
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
                                    egui::Color32::from_rgb(0x64, 0xd8, 0x9a),
                                    format!("目前連線: QUIC 直連 ({latency_ms}ms)"),
                                ),
                                Transport::Relay => (
                                    egui::Color32::from_rgb(0xf5, 0x9e, 0x0b),
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
        ui.label(
            egui::RichText::new("線上玩家")
                .size(13.0)
                .color(egui::Color32::from_rgb(0x88, 0x92, 0xb0)),
        );
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
                                    egui::Color32::from_rgb(0x64, 0xd8, 0x9a),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_wss_derives_https() {
        assert_eq!(
            web_viewer_base_url("wss://war3.kalthor.cc/ws"),
            "https://war3.kalthor.cc"
        );
    }

    #[test]
    fn dev_ws_localhost_with_port() {
        assert_eq!(
            web_viewer_base_url("ws://localhost:3000/ws"),
            "http://localhost:3000"
        );
    }

    #[test]
    fn self_host_wss_no_path() {
        assert_eq!(
            web_viewer_base_url("wss://my-war3.example.com"),
            "https://my-war3.example.com"
        );
    }

    #[test]
    fn self_host_wss_with_port_and_path() {
        assert_eq!(
            web_viewer_base_url("wss://my-war3.example.com:8443/ws"),
            "https://my-war3.example.com:8443"
        );
    }

    #[test]
    fn passthrough_https() {
        assert_eq!(
            web_viewer_base_url("https://example.com"),
            "https://example.com"
        );
    }

    #[test]
    fn passthrough_http_strips_path() {
        assert_eq!(
            web_viewer_base_url("http://example.com/ws"),
            "http://example.com"
        );
    }

    #[test]
    fn ipv6_loopback_with_port() {
        assert_eq!(
            web_viewer_base_url("ws://[::1]:3000/ws"),
            "http://[::1]:3000"
        );
    }

    #[test]
    fn malformed_falls_back_to_production() {
        assert_eq!(web_viewer_base_url("not-a-url"), "https://war3.kalthor.cc");
    }

    #[test]
    fn empty_falls_back_to_production() {
        assert_eq!(web_viewer_base_url(""), "https://war3.kalthor.cc");
    }

    #[test]
    fn empty_host_falls_back_to_production() {
        // `wss://` 後沒有 host：避免產出 `https:///join?room=...`
        assert_eq!(web_viewer_base_url("wss://"), "https://war3.kalthor.cc");
        assert_eq!(web_viewer_base_url("wss:///ws"), "https://war3.kalthor.cc");
    }

    #[test]
    fn userinfo_credentials_stripped() {
        // 若 user 把 credentials 塞到 SERVER_URL（自架不應該但可能），
        // 「複製連結」絕不能把 user:pass 帶到剪貼簿
        assert_eq!(
            web_viewer_base_url("wss://user:pass@host.example.com/ws"),
            "https://host.example.com"
        );
        assert_eq!(
            web_viewer_base_url("ws://token@localhost:3000/ws"),
            "http://localhost:3000"
        );
    }

    #[test]
    fn query_string_stripped() {
        // authority 後直接 ? 不經 path（RFC 3986）：必須剝掉避免拼接出無效 URL
        assert_eq!(
            web_viewer_base_url("wss://host.example.com?token=xyz"),
            "https://host.example.com"
        );
        assert_eq!(
            web_viewer_base_url("wss://host.example.com/ws?token=xyz"),
            "https://host.example.com"
        );
    }

    #[test]
    fn fragment_stripped() {
        assert_eq!(
            web_viewer_base_url("wss://host.example.com#section"),
            "https://host.example.com"
        );
    }

    #[test]
    fn userinfo_with_ipv6_stripped() {
        // 確認 `@` 與 IPv6 brackets 互動正確：rsplit('@') 取 `@` 之後
        assert_eq!(
            web_viewer_base_url("ws://user@[::1]:3000/ws"),
            "http://[::1]:3000"
        );
    }

    #[test]
    fn combined_userinfo_query_fragment_stripped() {
        assert_eq!(
            web_viewer_base_url("wss://user:pass@host.example.com:8443/ws?token=abc#frag"),
            "https://host.example.com:8443"
        );
    }
}
