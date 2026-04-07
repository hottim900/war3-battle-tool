use eframe::egui;

use crate::net::quic::{StrategyKind, StrategyOutcome, StrategyResult};
use crate::net::tunnel::Transport;

const LANE_HEIGHT: f32 = 20.0;
const LANE_SPACING: f32 = 4.0;
const BAR_MIN_WIDTH: f32 = 2.0;
const BAR_CORNER_RADIUS: u8 = 3;

/// 連線時間軸視覺化面板
pub struct TimelinePanel;

impl TimelinePanel {
    pub fn show(
        ui: &mut egui::Ui,
        diagnostics: &[StrategyResult],
        transport: Option<Transport>,
        tunnel_latency_ms: u64,
    ) {
        if diagnostics.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.colored_label(egui::Color32::from_rgb(0x64, 0x6c, 0x80), "尚未連線");
            });
            return;
        }

        // 計算時間軸範圍
        let max_duration = diagnostics
            .iter()
            .map(|r| r.duration_ms)
            .max()
            .unwrap_or(1)
            .max(1);

        egui::Frame::new()
            .fill(ui.style().visuals.extreme_bg_color)
            .inner_margin(12.0)
            .corner_radius(4.0)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;

                // 目前連線狀態
                if let Some(t) = transport {
                    let (color, text) = match t {
                        Transport::Direct => (
                            egui::Color32::from_rgb(0x64, 0xd8, 0x9a),
                            format!("QUIC 直連 ({tunnel_latency_ms}ms)"),
                        ),
                        Transport::Relay => (
                            egui::Color32::from_rgb(0xf5, 0x9e, 0x0b),
                            format!("Relay 中繼 ({tunnel_latency_ms}ms)"),
                        ),
                    };
                    ui.horizontal(|ui| {
                        ui.colored_label(color, format!("● {text}"));
                    });
                    ui.add_space(8.0);
                }

                // 策略泳道圖
                let available_width = ui.available_width();
                let label_width = 80.0;
                let bar_area_width = (available_width - label_width - 60.0).max(100.0);

                for result in diagnostics {
                    ui.horizontal(|ui| {
                        // 策略名稱
                        let label = match result.method {
                            StrategyKind::QuicDirect => "QUIC 穿透",
                            StrategyKind::UPnP => "UPnP",
                        };
                        ui.label(
                            egui::RichText::new(label)
                                .size(12.0)
                                .color(egui::Color32::from_rgb(0x88, 0x92, 0xb0)),
                        );

                        // 為 label 預留空間
                        let label_used = ui.min_rect().width();
                        if label_used < label_width {
                            ui.add_space(label_width - label_used);
                        }

                        // 泳道 bar
                        let bar_width = if result.duration_ms > 0 {
                            ((result.duration_ms as f32 / max_duration as f32) * bar_area_width)
                                .max(BAR_MIN_WIDTH)
                        } else {
                            BAR_MIN_WIDTH
                        };

                        let (bar_color, status_text) = match &result.outcome {
                            StrategyOutcome::Success => (
                                egui::Color32::from_rgb(0x64, 0xd8, 0x9a),
                                format!("✓ {}ms", result.duration_ms),
                            ),
                            StrategyOutcome::Failed(reason) => (
                                egui::Color32::from_rgb(0xef, 0x44, 0x44),
                                format!("✗ {reason}"),
                            ),
                            StrategyOutcome::Skipped => (
                                egui::Color32::from_rgb(0x4a, 0x56, 0x68),
                                "跳過".to_string(),
                            ),
                        };

                        // 繪製 bar
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(bar_width, LANE_HEIGHT),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::same(BAR_CORNER_RADIUS),
                            bar_color,
                        );

                        ui.add_space(4.0);

                        // 結果文字
                        ui.label(egui::RichText::new(status_text).size(11.0).color(bar_color));
                    });

                    ui.add_space(LANE_SPACING);
                }

                // 傳輸切換標記
                if transport == Some(Transport::Direct) && diagnostics.len() > 1 {
                    ui.add_space(4.0);
                    ui.colored_label(
                        egui::Color32::from_rgb(0x64, 0xd8, 0x9a),
                        egui::RichText::new("↑ Relay → P2P 直連 (mid-game swap)").size(11.0),
                    );
                }
            });
    }
}
