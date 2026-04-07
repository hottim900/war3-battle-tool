use eframe::egui;
use std::collections::VecDeque;

use crate::logging::{LogEntry, LogLevel, Verbosity};

const MAX_LOG_ENTRIES: usize = 2000;

pub struct LogPanel {
    entries: VecDeque<LogEntry>,
    filtered_indices: Vec<usize>,
    verbosity_filter: Verbosity,
    search_query: String,
    search_active: bool,
}

impl LogPanel {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            filtered_indices: Vec::new(),
            verbosity_filter: Verbosity::Concise,
            search_query: String::new(),
            search_active: false,
        }
    }

    /// 從 UiLogLayer channel 接收 LogEntry
    /// 從 UiLogLayer channel 接收 LogEntry
    pub fn push(&mut self, entry: LogEntry) {
        let passes = self.entry_passes_filter(&entry);
        self.entries.push_back(entry);

        // Ring buffer 溢出：pop 多餘的，最後只 rebuild 一次
        let mut evicted = false;
        while self.entries.len() > MAX_LOG_ENTRIES {
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
                        if let Some(log_dir) =
                            dirs::data_dir().map(|d| d.join("war3-battle-tool").join("logs"))
                        {
                            let _ = open::that(&log_dir);
                        }
                        ui.close_menu();
                    }
                });
            });
        });

        ui.separator();

        // Ring buffer 溢出提示
        if self.entries.len() == MAX_LOG_ENTRIES {
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
