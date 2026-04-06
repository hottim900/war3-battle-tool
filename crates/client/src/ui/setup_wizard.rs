use eframe::egui;
use war3_protocol::war3::War3Version;

enum WizardStep {
    Info,
    Setup,
}

/// 首次執行設定精靈：說明頁 → 輸入暱稱 + 選擇 War3 版本
pub struct SetupWizard {
    pub nickname: String,
    pub war3_version: War3Version,
    pub done: bool,
    step: WizardStep,
}

impl SetupWizard {
    pub fn new() -> Self {
        Self {
            nickname: String::new(),
            war3_version: War3Version::V127,
            done: false,
            step: WizardStep::Info,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| match self.step {
            WizardStep::Info => self.show_info(ui),
            WizardStep::Setup => self.show_setup(ui),
        });
    }

    fn show_info(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.heading("War3 Battle Tool");
                ui.add_space(8.0);
                ui.small("1 / 2");
                ui.add_space(20.0);
                ui.heading("開始之前");
                ui.add_space(16.0);
            });

            let bg = ui.visuals().faint_bg_color;

            info_card(
                ui,
                bg,
                "Windows 防火牆",
                "\
程式啟動後，Windows 防火牆可能詢問是否允許網路存取。\
建議選「允許」以啟用 P2P 直連（延遲更低）。\
如果選「封鎖」，程式仍可透過雲端中轉正常使用。\
每次更新版本可能會再次詢問。",
            );

            ui.add_space(12.0);

            info_card(
                ui,
                bg,
                "自動 Port Mapping (UPnP)",
                "\
程式會嘗試透過 UPnP 自動在路由器開啟連接埠，用於 P2P 直連。\
這是自動的，不需要手動設定路由器。\
如果路由器不支援 UPnP，程式會自動改用雲端中轉。",
            );

            ui.add_space(12.0);

            info_card(
                ui,
                bg,
                "P2P 直連 (QUIC)",
                "\
對戰時，程式優先嘗試 P2P 直連以降低延遲。\
如果網路環境不支援（例如行動熱點），會自動降級為雲端中轉。\
兩種方式都能正常對戰。",
            );

            ui.add_space(12.0);

            // NOTE: SmartScreen 是事後確認（使用者已通過才看到此頁）。
            // 如果未來加了程式碼簽章，更新或移除此段。
            info_card(
                ui,
                bg,
                "Windows SmartScreen 警告",
                "\
如果你剛才看到「Windows 已保護您的電腦」，\
這是因為程式沒有商業簽章（開源軟體的常態）。\
點「其他資訊 → 仍要執行」即可。",
            );

            ui.add_space(20.0);

            ui.vertical_centered(|ui| {
                if ui.button("繼續").clicked() {
                    self.step = WizardStep::Setup;
                }
            });

            ui.add_space(20.0);
        });
    }

    fn show_setup(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.heading("War3 Battle Tool");
                ui.add_space(8.0);
                ui.small("2 / 2");
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
                        crate::ui::war3_version_combo(ui, "version_select", &mut self.war3_version);
                        ui.end_row();
                    });

                ui.add_space(20.0);

                let can_proceed = !self.nickname.trim().is_empty();
                ui.horizontal(|ui| {
                    if ui.button("上一步").clicked() {
                        self.step = WizardStep::Info;
                    }
                    if ui
                        .add_enabled(can_proceed, egui::Button::new("開始"))
                        .clicked()
                    {
                        self.done = true;
                    }
                });
            });
        });
    }
}

/// 帶淺色背景的說明卡片
fn info_card(ui: &mut egui::Ui, bg: egui::Color32, title: &str, body: &str) {
    egui::Frame::NONE
        .fill(bg)
        .inner_margin(12.0)
        .corner_radius(4.0)
        .show(ui, |ui| {
            ui.strong(title);
            ui.add_space(6.0);
            ui.label(body);
        });
}
