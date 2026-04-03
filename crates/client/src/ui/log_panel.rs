use eframe::egui;
use std::collections::VecDeque;

const MAX_LOG_ENTRIES: usize = 200;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub message: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

pub struct LogPanel {
    entries: VecDeque<LogEntry>,
}

impl LogPanel {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    pub fn add(&mut self, level: LogLevel, message: impl Into<String>) {
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        self.entries.push_back(LogEntry {
            timestamp: now,
            message: message.into(),
            level,
        });
        while self.entries.len() > MAX_LOG_ENTRIES {
            self.entries.pop_front();
        }
    }

    pub fn info(&mut self, message: impl Into<String>) {
        self.add(LogLevel::Info, message);
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.add(LogLevel::Warn, message);
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.add(LogLevel::Error, message);
    }

    pub fn show(&self, ui: &mut egui::Ui) {
        ui.heading("日誌");
        ui.separator();

        egui::ScrollArea::vertical()
            .id_salt("log_scroll")
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for entry in &self.entries {
                    let color = match entry.level {
                        LogLevel::Info => egui::Color32::LIGHT_GRAY,
                        LogLevel::Warn => egui::Color32::from_rgb(255, 200, 100),
                        LogLevel::Error => egui::Color32::from_rgb(255, 100, 100),
                    };
                    ui.horizontal(|ui| {
                        ui.weak(&entry.timestamp);
                        ui.colored_label(color, &entry.message);
                    });
                }
            });
    }
}
