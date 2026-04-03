use serde::{Deserialize, Serialize};

/// War3 遊戲使用的 UDP port
pub const WAR3_PORT: u16 = 6112;

/// 支援的 War3 版本
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum War3Version {
    #[serde(rename = "1.27")]
    V127,
    #[serde(rename = "1.29c")]
    V129c,
}

impl War3Version {
    /// 客戶端廣播封包（宣告存在），用於 LAN 遊戲發現
    pub fn broadcast_packet(&self) -> &'static [u8] {
        match self {
            // 差異：offset 8 的版本位元組不同（0x1b vs 0x1d）
            War3Version::V127 => &[
                0xf7, 0x2f, 0x10, 0x00, 0x50, 0x58, 0x33, 0x57, 0x1b, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00,
            ],
            War3Version::V129c => &[
                0xf7, 0x2f, 0x10, 0x00, 0x50, 0x58, 0x33, 0x57, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00,
            ],
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            War3Version::V127 => "1.27",
            War3Version::V129c => "1.29c",
        }
    }
}

impl std::fmt::Display for War3Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
