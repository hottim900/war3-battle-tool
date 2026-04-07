use std::fmt::Write as _;

use tokio::sync::mpsc;
use tracing::Level;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;

// ── Verbosity ──────────────────────────────────────────────

/// 日誌詳細度等級，決定哪些訊息對哪種使用者可見。
///
/// - `Concise`: 玩家直覺能懂（"已連線"、"加入成功"）
/// - `Detailed`: 進階玩家（傳輸切換、重連、延遲變化）
/// - `Full`: 開發者/除錯（QUIC handshake、UPnP probe）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verbosity {
    Concise,
    Detailed,
    Full,
}

impl Verbosity {
    pub fn label(&self) -> &'static str {
        match self {
            Verbosity::Concise => "簡潔",
            Verbosity::Detailed => "詳細",
            Verbosity::Full => "全部",
        }
    }
}

// ── LogLevel / LogEntry ────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub message: String,
    pub level: LogLevel,
    pub verbosity: Verbosity,
    #[allow(dead_code)] // 用於 timeline 視覺化
    pub module: String,
}

// ── VerbosityVisitor ───────────────────────────────────────

/// 從 tracing event 提取 message 和 verbosity field。
struct VerbosityVisitor {
    verbosity: Option<Verbosity>,
    message: String,
}

impl VerbosityVisitor {
    fn new() -> Self {
        Self {
            verbosity: None,
            message: String::new(),
        }
    }
}

impl Visit for VerbosityVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "verbosity" {
            self.verbosity = match value {
                "concise" => Some(Verbosity::Concise),
                "detailed" => Some(Verbosity::Detailed),
                "full" => Some(Verbosity::Full),
                _ => None,
            };
        }
        // tracing 的 message 通常走 record_debug，但保險起見也處理 record_str
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            // tracing::info!("...") 的 message 是 fmt::Arguments，
            // Debug impl 直接輸出 formatted string（無額外引號）。
            self.message.clear();
            let _ = write!(self.message, "{:?}", value);
        }
    }
}

// ── Verbosity 分類邏輯 ─────────────────────────────────────

/// 根據 field、module path、level 決定最終 verbosity。
///
/// 優先順序：
/// 1. Warn/Error → 強制 Concise（玩家永遠看得到）
/// 2. 明確 field 標記（`verbosity = "concise"`）
/// 3. Module fallback：`war3_client::net::*` → Full
/// 4. 其他 → Concise
fn classify_verbosity(
    field_verbosity: Option<Verbosity>,
    level: &Level,
    module_path: Option<&str>,
) -> Verbosity {
    // Warn/Error 強制 Concise
    if *level == Level::WARN || *level == Level::ERROR {
        return Verbosity::Concise;
    }

    // 明確 field 標記
    if let Some(v) = field_verbosity {
        return v;
    }

    // Module fallback
    if let Some(path) = module_path
        && path.contains("::net::")
    {
        return Verbosity::Full;
    }

    Verbosity::Concise
}

// ── UiLogLayer ─────────────────────────────────────────────

/// 自訂 tracing Layer，將 log 事件透過 mpsc channel 推送到 UI thread。
pub struct UiLogLayer {
    tx: mpsc::UnboundedSender<LogEntry>,
}

impl UiLogLayer {
    pub fn new(tx: mpsc::UnboundedSender<LogEntry>) -> Self {
        Self { tx }
    }
}

impl<S> Layer<S> for UiLogLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = VerbosityVisitor::new();
        event.record(&mut visitor);

        let metadata = event.metadata();

        let level = match *metadata.level() {
            Level::ERROR => LogLevel::Error,
            Level::WARN => LogLevel::Warn,
            _ => LogLevel::Info,
        };

        let verbosity =
            classify_verbosity(visitor.verbosity, metadata.level(), metadata.module_path());

        let module = metadata
            .module_path()
            .unwrap_or("")
            .strip_prefix("war3_client::")
            .unwrap_or(metadata.module_path().unwrap_or(""))
            .to_string();

        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();

        let entry = LogEntry {
            timestamp,
            message: visitor.message,
            level,
            verbosity,
            module,
        };

        // 送不出去就丟掉（UI 已關閉）
        let _ = self.tx.send(entry);
    }
}

// ── Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn test_visitor_extracts_formatted_message() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let layer = UiLogLayer::new(tx);
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!("Tunnel 已連線");

        let entry = rx.try_recv().expect("should receive log entry");
        assert_eq!(entry.message, "Tunnel 已連線");
        assert_eq!(entry.level, LogLevel::Info);
    }

    #[test]
    fn test_visitor_extracts_verbosity() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let layer = UiLogLayer::new(tx);
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!(verbosity = "concise", "已連線");

        let entry = rx.try_recv().expect("should receive log entry");
        assert_eq!(entry.verbosity, Verbosity::Concise);
        assert_eq!(entry.message, "已連線");
    }

    #[test]
    fn test_visitor_unknown_verbosity_defaults_none() {
        // Unknown verbosity + no net:: module → fallback to Concise
        let (tx, mut rx) = mpsc::unbounded_channel();
        let layer = UiLogLayer::new(tx);
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!(verbosity = "invalid", "test");

        let entry = rx.try_recv().expect("should receive log entry");
        // Unknown field + INFO level + non-net module → Concise fallback
        assert_eq!(entry.verbosity, Verbosity::Concise);
    }

    #[test]
    fn test_warn_error_override() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let layer = UiLogLayer::new(tx);
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::error!(verbosity = "full", "嚴重錯誤");

        let entry = rx.try_recv().expect("should receive log entry");
        // Error 強制 Concise，不管 field 怎麼標
        assert_eq!(entry.verbosity, Verbosity::Concise);
        assert_eq!(entry.level, LogLevel::Error);
    }

    #[test]
    fn test_channel_delivery() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let layer = UiLogLayer::new(tx);
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!(verbosity = "detailed", "傳輸切換");
        tracing::warn!("連線不穩");

        let e1 = rx.try_recv().expect("first entry");
        let e2 = rx.try_recv().expect("second entry");

        assert_eq!(e1.message, "傳輸切換");
        assert_eq!(e1.verbosity, Verbosity::Detailed);

        assert_eq!(e2.message, "連線不穩");
        assert_eq!(e2.level, LogLevel::Warn);
        assert_eq!(e2.verbosity, Verbosity::Concise); // Warn 強制 Concise
    }

    #[test]
    fn test_interpolated_message() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let layer = UiLogLayer::new(tx);
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let count = 42;
        tracing::info!(verbosity = "concise", "已處理 {count} 筆");

        let entry = rx.try_recv().expect("should receive log entry");
        assert_eq!(entry.message, "已處理 42 筆");
    }

    #[test]
    fn test_classify_verbosity_module_fallback() {
        // net:: module 無明確 field → Full
        assert_eq!(
            classify_verbosity(None, &Level::INFO, Some("war3_client::net::tunnel")),
            Verbosity::Full,
        );
        // 非 net:: module 無明確 field → Concise
        assert_eq!(
            classify_verbosity(None, &Level::INFO, Some("war3_client::app")),
            Verbosity::Concise,
        );
        // net:: module 但有明確 field → 用 field
        assert_eq!(
            classify_verbosity(
                Some(Verbosity::Concise),
                &Level::INFO,
                Some("war3_client::net::quic")
            ),
            Verbosity::Concise,
        );
    }
}
