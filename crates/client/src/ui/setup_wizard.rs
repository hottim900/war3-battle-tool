use eframe::egui;
use war3_protocol::war3::War3Version;

/// 首次執行設定精靈：輸入暱稱 + 選擇 War3 版本
pub struct SetupWizard {
    pub nickname: String,
    pub war3_version: War3Version,
    pub done: bool,
}

impl SetupWizard {
    pub fn new() -> Self {
        Self {
            nickname: String::new(),
            war3_version: War3Version::V127,
            done: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.heading("War3 Battle Tool");
                ui.add_space(20.0);
                ui.label("歡迎！請設定你的遊戲資訊：");
                ui.add_space(20.0);

                egui::Grid::new("setup_grid")
                    .num_columns(2)
                    .spacing([10.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("暱稱：");
                        ui.text_edit_singleline(&mut self.nickname);
                        ui.end_row();

                        ui.label("War3 版本：");
                        crate::ui::war3_version_combo(
                            ui,
                            "version_select",
                            &mut self.war3_version,
                        );
                        ui.end_row();
                    });

                ui.add_space(20.0);

                let can_proceed = !self.nickname.trim().is_empty();
                if ui
                    .add_enabled(can_proceed, egui::Button::new("開始"))
                    .clicked()
                {
                    self.done = true;
                }
            });
        });
    }
}
