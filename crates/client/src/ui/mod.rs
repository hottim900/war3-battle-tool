pub mod lobby;
pub mod log_panel;
pub mod settings;
pub mod setup_wizard;
pub mod timeline;

use eframe::egui;
use war3_protocol::war3::War3Version;

/// 共用的 War3 版本選擇 combo box
pub fn war3_version_combo(ui: &mut egui::Ui, id: &str, version: &mut War3Version) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(version.as_str())
        .show_ui(ui, |ui| {
            ui.selectable_value(version, War3Version::V127, "1.27");
            ui.selectable_value(version, War3Version::V129c, "1.29c");
        });
}
