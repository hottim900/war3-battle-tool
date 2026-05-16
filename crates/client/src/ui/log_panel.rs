use eframe::egui;
use std::collections::VecDeque;

use crate::config::{LOG_BUFFER_MAX, LOG_BUFFER_MIN};
use crate::logging::{LogEntry, LogLevel, Verbosity};

pub struct LogPanel {
    entries: VecDeque<LogEntry>,
    filtered_indices: Vec<usize>,
    verbosity_filter: Verbosity,
    search_query: String,
    search_active: bool,
    max_entries: usize,
}

impl LogPanel {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            filtered_indices: Vec::new(),
            verbosity_filter: Verbosity::Concise,
            search_query: String::new(),
            search_active: false,
            max_entries: max_entries.clamp(LOG_BUFFER_MIN, LOG_BUFFER_MAX),
        }
    }

    /// 從 UiLogLayer channel 接收 LogEntry
    pub fn push(&mut self, entry: LogEntry) {
        let passes = self.entry_passes_filter(&entry);
        self.entries.push_back(entry);

        // Ring buffer 溢出：pop 多餘的，最後只 rebuild 一次
        let mut evicted = false;
        while self.entries.len() > self.max_entries {
            self.entries.pop_front();
            evicted = true;
        }

        if evicted {
            // pop_front 改變了所有 index，必須重建
            self.rebuild_filtered_indices();
        } else if passes {
            // 沒有溢出時才用增量 append（index 穩定）
            self.filtered_indices.push(self.entries.len() - 1);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.filtered_indices.clear();
    }

    fn entry_passes_filter(&self, entry: &LogEntry) -> bool {
        // Verbosity 過濾：entry.verbosity <= self.verbosity_filter 才顯示
        let passes_verbosity = entry.verbosity <= self.verbosity_filter;
        if !passes_verbosity {
            return false;
        }

        // 文字搜尋
        if !self.search_query.is_empty() {
            return entry.message.contains(&self.search_query);
        }

        true
    }

    fn rebuild_filtered_indices(&mut self) {
        self.filtered_indices.clear();
        for (i, entry) in self.entries.iter().enumerate() {
            if self.entry_passes_filter(entry) {
                self.filtered_indices.push(i);
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        // Toolbar
        ui.horizontal(|ui| {
            // Verbosity filter 按鈕
            for &v in &[Verbosity::Concise, Verbosity::Detailed, Verbosity::Full] {
                let selected = self.verbosity_filter == v;
                let btn =
                    egui::Button::new(egui::RichText::new(v.label()).size(12.0)).selected(selected);
                if ui.add(btn).clicked() && !selected {
                    self.verbosity_filter = v;
                    self.rebuild_filtered_indices();
                }
            }

            ui.separator();

            // 搜尋
            if self.search_active {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .desired_width(150.0)
                        .hint_text("搜尋...")
                        .font(egui::TextStyle::Small),
                );
                if resp.changed() {
                    self.rebuild_filtered_indices();
                }
                if ui.small_button("✕").clicked() {
                    self.search_active = false;
                    self.search_query.clear();
                    self.rebuild_filtered_indices();
                }
            } else if ui.small_button("🔍").clicked() {
                self.search_active = true;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Overflow 按鈕：複製 / 匯出
                ui.menu_button("⋯", |ui| {
                    if ui.button("複製全部").clicked() {
                        let text: String = self
                            .filtered_indices
                            .iter()
                            .filter_map(|&i| self.entries.get(i))
                            .map(|e| format!("{} {}", e.timestamp, e.message))
                            .collect::<Vec<_>>()
                            .join("\n");
                        ui.ctx().copy_text(text);
                        ui.close_menu();
                    }
                    if ui.button("開啟 Log 資料夾").clicked() {
                        match crate::logging::default_log_dir() {
                            Some(log_dir) => {
                                if let Err(e) = open::that(&log_dir) {
                                    tracing::warn!(
                                        verbosity = "concise",
                                        "無法開啟 log 資料夾 {}: {e}",
                                        log_dir.display()
                                    );
                                }
                            }
                            None => tracing::warn!(
                                verbosity = "concise",
                                "找不到使用者 data 目錄，無法定位 log 資料夾"
                            ),
                        }
                        ui.close_menu();
                    }
                });
            });
        });

        ui.separator();

        // Ring buffer 溢出提示
        if self.entries.len() == self.max_entries {
            ui.colored_label(
                egui::Color32::from_rgb(0x64, 0x6c, 0x80),
                egui::RichText::new("已省略較舊日誌").size(11.0),
            );
        }

        // 虛擬捲動
        let row_height = 18.0;
        let num_rows = self.filtered_indices.len();

        if num_rows == 0 {
            ui.centered_and_justified(|ui| {
                if self.search_active && !self.search_query.is_empty() {
                    ui.colored_label(egui::Color32::from_rgb(0x64, 0x6c, 0x80), "無符合結果");
                } else {
                    ui.colored_label(egui::Color32::from_rgb(0x64, 0x6c, 0x80), "等待日誌...");
                }
            });
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("log_scroll")
            .stick_to_bottom(true)
            .auto_shrink(false)
            .show_rows(ui, row_height, num_rows, |ui, row_range| {
                for row in row_range {
                    if let Some(&entry_idx) = self.filtered_indices.get(row)
                        && let Some(entry) = self.entries.get(entry_idx)
                    {
                        let color = match entry.level {
                            LogLevel::Info => egui::Color32::from_rgb(0x88, 0x92, 0xb0),
                            LogLevel::Warn => egui::Color32::from_rgb(0xf5, 0x9e, 0x0b),
                            LogLevel::Error => egui::Color32::from_rgb(0xef, 0x44, 0x44),
                        };
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                egui::Color32::from_rgb(0x4a, 0x56, 0x68),
                                &entry.timestamp,
                            );
                            ui.colored_label(color, &entry.message);
                        });
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LOG_BUFFER_DEFAULT;

    fn entry(message: &str, verbosity: Verbosity, level: LogLevel) -> LogEntry {
        LogEntry {
            timestamp: "00:00:00".into(),
            message: message.into(),
            level,
            verbosity,
            module: "war3_client::test".into(),
        }
    }

    #[test]
    fn test_ring_buffer_overflow() {
        let mut panel = LogPanel::new(LOG_BUFFER_DEFAULT);
        for i in 0..LOG_BUFFER_DEFAULT + 1 {
            panel.push(entry(
                &format!("msg {i}"),
                Verbosity::Concise,
                LogLevel::Info,
            ));
        }
        assert_eq!(panel.entries.len(), LOG_BUFFER_DEFAULT);
        // 最舊的被淘汰，最新的留著
        assert_eq!(panel.entries.front().unwrap().message, "msg 1");
        assert_eq!(
            panel.entries.back().unwrap().message,
            format!("msg {}", LOG_BUFFER_DEFAULT)
        );
        // 確認 evicted 路徑也有 rebuild filtered_indices（不只是 entries）
        assert_eq!(panel.filtered_indices.len(), LOG_BUFFER_DEFAULT);
    }

    #[test]
    fn test_filtered_indices_update() {
        let mut panel = LogPanel::new(LOG_BUFFER_DEFAULT);
        // 預設 filter = Concise，只收 Concise
        panel.push(entry("a", Verbosity::Concise, LogLevel::Info));
        panel.push(entry("b", Verbosity::Detailed, LogLevel::Info));
        panel.push(entry("c", Verbosity::Full, LogLevel::Info));
        assert_eq!(panel.filtered_indices.len(), 1);

        // 切到 Detailed：Concise + Detailed 都看得到
        panel.verbosity_filter = Verbosity::Detailed;
        panel.rebuild_filtered_indices();
        assert_eq!(panel.filtered_indices.len(), 2);

        // 切到 Full：全部看得到
        panel.verbosity_filter = Verbosity::Full;
        panel.rebuild_filtered_indices();
        assert_eq!(panel.filtered_indices.len(), 3);
    }

    #[test]
    fn test_search_filter() {
        let mut panel = LogPanel::new(LOG_BUFFER_DEFAULT);
        panel.verbosity_filter = Verbosity::Full;
        panel.push(entry("Tunnel 已連線", Verbosity::Concise, LogLevel::Info));
        panel.push(entry("加入房間成功", Verbosity::Concise, LogLevel::Info));
        panel.push(entry("Tunnel 中斷", Verbosity::Detailed, LogLevel::Warn));

        panel.search_query = "Tunnel".into();
        panel.rebuild_filtered_indices();
        assert_eq!(panel.filtered_indices.len(), 2);
        // 確認過濾出的真的都含關鍵字
        for idx in &panel.filtered_indices {
            assert!(panel.entries[*idx].message.contains("Tunnel"));
        }
    }

    #[test]
    fn test_buffer_size_clamped_to_range() {
        // 超出上限 → 夾到 5000
        let panel = LogPanel::new(99999);
        assert_eq!(panel.max_entries, LOG_BUFFER_MAX);
        // 低於下限 → 夾到 1000
        let panel = LogPanel::new(10);
        assert_eq!(panel.max_entries, LOG_BUFFER_MIN);
        // 範圍內 → 原值
        let panel = LogPanel::new(3000);
        assert_eq!(panel.max_entries, 3000);
    }
}
