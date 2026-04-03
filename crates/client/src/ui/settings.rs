use eframe::egui;

use crate::config::AppConfig;

/// 設定面板
pub fn show(ui: &mut egui::Ui, config: &mut AppConfig, config_changed: &mut bool) {
    ui.heading("設定");
    ui.add_space(10.0);

    egui::Grid::new("settings_grid")
        .num_columns(2)
        .spacing([10.0, 10.0])
        .show(ui, |ui| {
            ui.label("暱稱：");
            if ui.text_edit_singleline(&mut config.nickname).changed() {
                *config_changed = true;
            }
            ui.end_row();

            ui.label("War3 版本：");
            let prev = config.war3_version;
            crate::ui::war3_version_combo(ui, "settings_version", &mut config.war3_version);
            if config.war3_version != prev {
                *config_changed = true;
            }
            ui.end_row();

            ui.label("伺服器位址：");
            if ui.text_edit_singleline(&mut config.server_url).changed() {
                *config_changed = true;
            }
            ui.end_row();

            ui.label("本地 IP：");
            if ui
                .text_edit_singleline(&mut config.local_ip)
                .on_hover_text("封包注入的目標 IP（通常為 127.0.0.1 或真實網卡 IP）")
                .changed()
            {
                *config_changed = true;
            }
            ui.end_row();
        });

    ui.add_space(10.0);
    ui.separator();
    ui.add_space(4.0);
    ui.label("網路診斷");
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("測試 Port 6112").clicked() {
            let addr = format!("{}:{}", config.local_ip, 6112);
            match std::net::TcpListener::bind(&addr) {
                Ok(_) => {
                    tracing::info!("Port 6112 可用");
                }
                Err(e) => {
                    tracing::warn!("Port 6112 無法綁定: {e}");
                }
            }
        }
        if ui.button("測試 UPnP").clicked() {
            match crate::net::packet::try_upnp_port_forward(war3_protocol::war3::WAR3_PORT) {
                Ok(()) => tracing::info!("UPnP port 映射成功"),
                Err(e) => tracing::warn!("UPnP 失敗: {e}"),
            }
        }
    });

    ui.add_space(10.0);
    if *config_changed {
        ui.colored_label(
            egui::Color32::from_rgb(255, 200, 100),
            "設定已修改，重新連線後生效。",
        );
        if ui.button("儲存設定").clicked() {
            if let Err(e) = config.save() {
                tracing::error!("儲存設定失敗: {e}");
            } else {
                *config_changed = false;
            }
        }
    }
}
