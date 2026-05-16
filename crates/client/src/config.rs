use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use war3_protocol::war3::War3Version;

const CONFIG_FILENAME: &str = "war3-battle-tool.json";

pub(crate) const LOG_BUFFER_MIN: usize = 1000;
pub(crate) const LOG_BUFFER_MAX: usize = 5000;
pub(crate) const LOG_BUFFER_DEFAULT: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub nickname: String,
    pub war3_version: War3Version,
    pub server_url: String,
    /// 本地 IP，用於封包注入目標（loopback 或真實網卡 IP）
    #[serde(default = "default_local_ip")]
    pub local_ip: String,
    /// LogPanel ring buffer 大小（1000-5000）
    #[serde(default = "default_log_buffer_size")]
    pub log_buffer_size: usize,
}

fn default_local_ip() -> String {
    "127.0.0.1".into()
}

fn default_log_buffer_size() -> usize {
    LOG_BUFFER_DEFAULT
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            nickname: String::new(),
            war3_version: War3Version::V127,
            server_url: std::env::var("SERVER_URL")
                .unwrap_or_else(|_| "wss://war3.kalthor.cc/ws".into()),
            local_ip: default_local_ip(),
            log_buffer_size: default_log_buffer_size(),
        }
    }
}

impl AppConfig {
    /// 設定檔路徑：Windows %APPDATA%，其他 ~/.config/
    fn config_path() -> PathBuf {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join(CONFIG_FILENAME)
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        let mut cfg: Self = match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        cfg.normalize();
        cfg
    }

    /// 將手動編輯過的設定值夾到允許範圍，避免下游模組需要每次都防禦。
    fn normalize(&mut self) {
        let original = self.log_buffer_size;
        self.log_buffer_size = self.log_buffer_size.clamp(LOG_BUFFER_MIN, LOG_BUFFER_MAX);
        if self.log_buffer_size != original {
            tracing::warn!(
                verbosity = "concise",
                "log_buffer_size {original} 超出 {LOG_BUFFER_MIN}-{LOG_BUFFER_MAX}，已調整為 {}",
                self.log_buffer_size
            );
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    /// 是否已完成首次設定（有暱稱）
    pub fn is_configured(&self) -> bool {
        !self.nickname.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_clamps_oversize_log_buffer() {
        let mut cfg = AppConfig {
            log_buffer_size: 999_999,
            ..AppConfig::default()
        };
        cfg.normalize();
        assert_eq!(cfg.log_buffer_size, LOG_BUFFER_MAX);
    }

    #[test]
    fn normalize_clamps_undersize_log_buffer() {
        let mut cfg = AppConfig {
            log_buffer_size: 0,
            ..AppConfig::default()
        };
        cfg.normalize();
        assert_eq!(cfg.log_buffer_size, LOG_BUFFER_MIN);
    }

    #[test]
    fn normalize_leaves_in_range_value_untouched() {
        let mut cfg = AppConfig {
            log_buffer_size: 3000,
            ..AppConfig::default()
        };
        cfg.normalize();
        assert_eq!(cfg.log_buffer_size, 3000);
    }
}
