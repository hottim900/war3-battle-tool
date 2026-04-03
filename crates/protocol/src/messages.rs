use serde::{Deserialize, Serialize};

use crate::war3::War3Version;

/// 字串欄位長度限制
pub const MAX_NICKNAME_LEN: usize = 32;
pub const MAX_ROOM_NAME_LEN: usize = 64;
pub const MAX_MAP_NAME_LEN: usize = 128;

// ── Client → Server ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// 連線時發送，註冊玩家
    Register {
        nickname: String,
        war3_version: War3Version,
    },
    /// 定期心跳（每 10 秒）
    Heartbeat,
    /// 房主建房
    CreateRoom {
        room_name: String,
        map_name: String,
        max_players: u8,
    },
    /// 房主關房或離開
    CloseRoom,
    /// 玩家請求加入房間
    JoinRoom { room_id: String },
}

// ── Server → Client ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// 連線成功，回傳分配的 player_id
    Welcome { player_id: String },
    /// 線上玩家列表更新（全量快照）
    PlayerUpdate { players: Vec<PlayerInfo> },
    /// 房間列表更新（全量快照）
    RoomUpdate { rooms: Vec<RoomInfo> },
    /// 加入房間結果：包含房主的公開 IP（僅在加入時交換）
    JoinResult {
        success: bool,
        host_ip: Option<String>,
    },
    /// 通知房主：有人加入，包含玩家的公開 IP
    PlayerJoined { nickname: String, player_ip: String },
    /// 錯誤
    Error { message: String },
}

/// 玩家資訊（不含 IP，IP 僅在 JoinRoom 時交換）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub player_id: String,
    pub nickname: String,
    pub war3_version: War3Version,
    pub is_hosting: bool,
}

/// 房間資訊
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInfo {
    pub room_id: String,
    pub host_nickname: String,
    pub room_name: String,
    pub map_name: String,
    pub max_players: u8,
    pub current_players: u8,
    pub war3_version: War3Version,
}

impl ClientMessage {
    /// 驗證訊息欄位長度
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            ClientMessage::Register { nickname, .. } => {
                if nickname.len() > MAX_NICKNAME_LEN {
                    return Err("暱稱超過長度限制");
                }
                if nickname.trim().is_empty() {
                    return Err("暱稱不能為空");
                }
            }
            ClientMessage::CreateRoom {
                room_name,
                map_name,
                max_players,
            } => {
                if room_name.len() > MAX_ROOM_NAME_LEN {
                    return Err("房間名稱超過長度限制");
                }
                if room_name.trim().is_empty() {
                    return Err("房間名稱不能為空");
                }
                if map_name.len() > MAX_MAP_NAME_LEN {
                    return Err("地圖名稱超過長度限制");
                }
                if *max_players < 2 || *max_players > 12 {
                    return Err("玩家數量必須在 2-12 之間");
                }
            }
            _ => {}
        }
        Ok(())
    }
}
