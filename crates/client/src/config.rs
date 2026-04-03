use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use war3_protocol::war3::War3Version;

const CONFIG_FILENAME: &str = "war3-battle-tool.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub nickname: String,
    pub war3_version: War3Version,
    pub server_url: String,
    /// 本地 IP，用於封包注入目標（loopback 或真實網卡 IP）
    #[serde(default = "default_local_ip")]
    pub local_ip: String,
}

fn default_local_ip() -> String {
    "127.0.0.1".into()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            nickname: String::new(),
            war3_version: War3Version::V127,
            server_url: std::env::var("SERVER_URL")
                .unwrap_or_else(|_| "wss://war3.kalthor.cc/ws".into()),
            local_ip: default_local_ip(),
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
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
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
