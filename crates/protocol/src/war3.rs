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

/// 從 W3GS_GAMEINFO 封包解析房間名稱和地圖路徑
///
/// 封包格式（War3 1.27/1.29c）:
///   [0..4]   header: 0xF7, 0x30, size_le16
///   [4..8]   product ID
///   [8..12]  version
///   [12..16] host counter
///   [16..]   game name (null-terminated UTF-8)
///   接著是 0x00 分隔，然後 encoded game settings
///     第一個 byte 跳過後，decoded data 包含 map path（null-terminated）
pub fn parse_gameinfo(data: &[u8]) -> Option<GameinfoFields> {
    // 最小長度：header(4) + product(4) + version(4) + counter(4) + 至少 1 byte name
    if data.len() < 17 || data[0] != 0xF7 || data[1] != 0x30 {
        return None;
    }

    // Game name starts at offset 16, null-terminated
    let name_start = 16;
    let name_end = data[name_start..].iter().position(|&b| b == 0)?;
    let game_name = String::from_utf8_lossy(&data[name_start..name_start + name_end]).into_owned();

    // After game name: null terminator + separator byte, then stat string
    let stat_start = name_start + name_end + 2;

    // Stat string 以 0x00 結尾，裡面包含 encoded game settings
    // 解碼 stat string 取得 map path
    let map_path = decode_stat_string(data, stat_start);

    Some(GameinfoFields {
        game_name,
        map_path,
        // 從 encoded stat string 解析 max_players（offset 較深，需要完整解碼）
        // 暫時不解析，由 UI 讓使用者填
    })
}

/// W3GS stat string 解碼，取得 map path
///
/// Stat string 是 War3 自訂的編碼格式：
/// 每 1 mask byte + 7 data bytes 為一組（mask 的每個 bit 表示對應 byte 是否需要 -1 還原）
fn decode_stat_string(data: &[u8], start: usize) -> Option<String> {
    if start >= data.len() {
        return None;
    }

    // 找 stat string 的結尾（0x00）
    let stat_end = data[start..].iter().position(|&b| b == 0)?;
    let encoded = &data[start..start + stat_end];

    if encoded.is_empty() {
        return None;
    }

    // War3 stat string decoding: 1 mask byte + 7 data bytes per group
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < encoded.len() {
        let mask = encoded[i];
        i += 1;
        for bit in 0..7 {
            if i >= encoded.len() {
                break;
            }
            if mask & (1 << bit) != 0 {
                decoded.push(encoded[i] - 1);
            } else {
                decoded.push(encoded[i]);
            }
            i += 1;
        }
    }

    // decoded data 格式：
    //   settings_flags (4 bytes, little-endian)
    //   map_and_creator: 兩個 null-terminated strings
    //     第一個是 map path (e.g. "Maps\Download\DotA v6.83d.w3x")
    if decoded.len() < 5 {
        return None;
    }

    // Skip 4 bytes of settings flags
    let map_start = 4;
    let map_end = decoded[map_start..]
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(decoded.len() - map_start);

    let raw = &decoded[map_start..map_start + map_end];
    let map_path = String::from_utf8_lossy(raw).into_owned();

    // 取最後一個 '\' 後的檔名（去掉路徑）
    let map_name = map_path
        .rsplit_once('\\')
        .map(|(_, name)| name)
        .unwrap_or(&map_path)
        .trim_end_matches(".w3x")
        .trim_end_matches(".w3m")
        .to_string();

    Some(map_name)
}

/// GAMEINFO 解析結果
#[derive(Debug, Clone)]
pub struct GameinfoFields {
    /// 房間名稱（War3 建房時輸入的）
    pub game_name: String,
    /// 地圖名稱（從 map path 取檔名，去掉副檔名）
    pub map_path: Option<String>,
}
