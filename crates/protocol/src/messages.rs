use serde::{Deserialize, Serialize};

use crate::war3::War3Version;

/// 字串欄位長度限制
pub const MAX_NICKNAME_LEN: usize = 32;
pub const MAX_ROOM_NAME_LEN: usize = 64;
pub const MAX_MAP_NAME_LEN: usize = 128;
/// GAMEINFO 封包大小上限（bytes）
pub const MAX_GAMEINFO_LEN: usize = 1024;
/// UPnP external_addr 長度上限
pub const MAX_EXTERNAL_ADDR_LEN: usize = 64;

// ── Client → Server ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// 連線時發送，註冊玩家
    Register {
        nickname: String,
        war3_version: War3Version,
        /// Client 版本（v0.2.0+），server 可拒絕不相容版本
        #[serde(default)]
        client_version: Option<String>,
    },
    /// 定期心跳（每 10 秒）
    Heartbeat,
    /// 房主建房（v0.2.0: 附帶 GAMEINFO 供 joiner 注入）
    CreateRoom {
        room_name: String,
        map_name: String,
        max_players: u8,
        /// War3 GAMEINFO UDP 封包 bytes（host 本地擷取）
        #[serde(default)]
        gameinfo: Vec<u8>,
    },
    /// 房主關房或離開
    CloseRoom,
    /// 玩家請求加入房間
    JoinRoom { room_id: String },
    /// 延遲測量：client 送 ts，server 原封回傳
    Ping { ts: u64 },
    /// Host UPnP port mapping 成功，通知 server 轉發給 joiner
    UPnPMapped {
        /// 完整 SocketAddr 字串（IP:port），≤ 64 bytes
        external_addr: String,
        /// 對應的 tunnel pairing token
        tunnel_token: String,
    },
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
    /// 加入房間結果（v0.2.0: tunnel_token + gameinfo 取代 host_ip）
    JoinResult {
        success: bool,
        room_id: Option<String>,
        /// 一次性 tunnel token，用於開啟 /tunnel WebSocket
        tunnel_token: Option<String>,
        /// Host 的 GAMEINFO 封包 bytes，joiner 用來注入本地 War3
        gameinfo: Option<Vec<u8>>,
    },
    /// 通知房主：有人加入，附帶 tunnel token 讓 host 開啟對應 tunnel
    PlayerJoined {
        nickname: String,
        tunnel_token: String,
    },
    /// 通知 host：tunnel 已準備好（多人遊戲時，每個新 joiner 觸發一次）
    TunnelReady { tunnel_token: String },
    /// P2P 直連資訊：僅透過 unicast player tx 發送，不經 broadcast_state
    StunInfo { peer_addr: String },
    /// 延遲測量：原封回傳 client 的 ts
    Pong { ts: u64 },
    /// Host 的 UPnP mapped address，轉發給 joiner 嘗試直連
    PeerUPnPAddr { external_addr: String },
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

impl ServerMessage {
    pub fn join_failure() -> Self {
        ServerMessage::JoinResult {
            success: false,
            room_id: None,
            tunnel_token: None,
            gameinfo: None,
        }
    }
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
                gameinfo,
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
                if gameinfo.len() > MAX_GAMEINFO_LEN {
                    return Err("GAMEINFO 超過大小限制");
                }
            }
            ClientMessage::JoinRoom { room_id } => {
                if room_id.len() > 64 {
                    return Err("room_id 超過長度限制");
                }
            }
            ClientMessage::UPnPMapped {
                external_addr,
                tunnel_token,
            } => {
                if external_addr.len() > MAX_EXTERNAL_ADDR_LEN {
                    return Err("external_addr 超過長度限制");
                }
                if external_addr.trim().is_empty() {
                    return Err("external_addr 不能為空");
                }
                if tunnel_token.trim().is_empty() {
                    return Err("tunnel_token 不能為空");
                }
            }
            _ => {}
        }
        Ok(())
    }
}
