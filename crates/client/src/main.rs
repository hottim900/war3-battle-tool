mod app;
mod config;
mod logging;
mod net;
mod ui;

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use tokio::sync::mpsc;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;

use app::War3App;
use config::AppConfig;
use logging::UiLogLayer;
use net::discovery::{self, NetEvent};
use war3_protocol::messages::ClientMessage;

fn main() {
    // rustls 0.23+ 需要手動安裝 crypto provider
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("無法安裝 rustls crypto provider");

    // ── Logging 設定（channel 必須在 registry.init() 之前建立）──
    let (log_tx, log_rx) = mpsc::unbounded_channel();
    let ui_layer = UiLogLayer::new(log_tx);

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "war3_client=info".parse().unwrap());

    // File layer：寫入 {data_dir}/war3-battle-tool/logs/war3-{timestamp}.log
    // Option<L> 自動實作 Layer，None 時等於不加
    let file_layer = logging::default_log_dir()
        .and_then(|dir| logging::setup_file_writer(&dir, 30))
        .map(|writer| {
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false)
        });

    // env_filter 只套用在 terminal fmt layer，UI 和 file layer 接收所有 war3_client events
    let subscriber = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_filter(env_filter),
        )
        .with(ui_layer)
        .with(file_layer);

    tracing::subscriber::set_global_default(subscriber).expect("無法設定 tracing subscriber");

    let config = AppConfig::load();
    let server_url = config.server_url.clone();

    // 建立 UI ↔ Network 通道
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<ClientMessage>();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<NetEvent>();

    let latency_ms = Arc::new(AtomicU64::new(0));

    // 在背景執行緒啟動 tokio runtime（必須在 eframe::run_native 之前）
    let rt = tokio::runtime::Runtime::new().expect("無法建立 tokio runtime");
    let rt_handle = rt.handle().clone();
    let server_url_clone = server_url.clone();
    let latency_clone = latency_ms.clone();
    std::thread::spawn(move || {
        rt.block_on(async {
            discovery::run_connection(server_url_clone, cmd_rx, event_tx, latency_clone).await;
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
        Box::new(move |cc| {
            Ok(Box::new(War3App::new(
                cc, config, cmd_tx, event_rx, rt_handle, server_url, latency_ms, log_rx,
            )))
        }),
    )
    .expect("eframe 啟動失敗");
}
