mod app;
mod config;
mod net;
mod ui;

use tokio::sync::mpsc;

use app::War3App;
use config::AppConfig;
use net::discovery::{self, NetEvent};
use war3_protocol::messages::ClientMessage;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "war3_client=info".parse().unwrap()),
        )
        .init();

    let config = AppConfig::load();
    let server_url = config.server_url.clone();

    // 建立 UI ↔ Network 通道
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<ClientMessage>();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<NetEvent>();

    // 在背景執行緒啟動 tokio runtime（必須在 eframe::run_native 之前）
    let rt = tokio::runtime::Runtime::new().expect("無法建立 tokio runtime");
    std::thread::spawn(move || {
        rt.block_on(async {
            discovery::run_connection(server_url, cmd_rx, event_tx).await;
        });
    });

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([600.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "War3 Battle Tool",
        native_options,
        Box::new(move |cc| Ok(Box::new(War3App::new(cc, config, cmd_tx, event_rx, None)))),
    )
    .expect("eframe 啟動失敗");
}
