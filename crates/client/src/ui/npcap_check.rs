use eframe::egui;

/// Checks whether Npcap is available on the current system.
///
/// On Windows: checks for `wpcap.dll` in System32.
/// On non-Windows (Linux/macOS): always returns true (for development).
pub fn is_npcap_available() -> bool {
    #[cfg(windows)]
    {
        if let Some(sys_dir) = std::env::var_os("SystemRoot") {
            let dll_path = std::path::Path::new(&sys_dir)
                .join("System32")
                .join("wpcap.dll");
            return dll_path.exists();
        }
        false
    }

    #[cfg(not(windows))]
    {
        true
    }
}

/// Action returned from the npcap blocking panel.
pub enum NpcapPanelAction {
    /// User has not done anything; keep showing the panel.
    None,
    /// User clicked "re-check"; caller should re-run `is_npcap_available()`.
    Recheck,
}

/// Shows a full-screen blocking panel telling the user to install Npcap.
///
/// Returns `NpcapPanelAction::Recheck` when the user wants to re-check.
pub fn show_npcap_missing_panel(ctx: &egui::Context) -> NpcapPanelAction {
    let mut action = NpcapPanelAction::None;

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);

            ui.heading(
                egui::RichText::new("需要安裝 Npcap")
                    .size(28.0)
                    .strong()
                    .color(egui::Color32::from_rgb(255, 200, 100)),
            );

            ui.add_space(20.0);

            ui.label(
                egui::RichText::new(
                    "War3 Battle Tool 需要 Npcap 來進行封包注入。\n\
                     請安裝 Npcap 後重新啟動程式。",
                )
                .size(16.0),
            );

            ui.add_space(30.0);

            if ui
                .button(egui::RichText::new("開啟 Npcap 下載頁面").size(16.0))
                .clicked()
            {
                let _ = open::that("https://npcap.com");
            }

            ui.add_space(12.0);

            if ui
                .button(egui::RichText::new("重新檢查").size(16.0))
                .clicked()
            {
                action = NpcapPanelAction::Recheck;
            }
        });
    });

    action
}
