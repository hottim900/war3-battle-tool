use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, RwLock};
use war3_protocol::messages::{PlayerInfo, RoomInfo, ServerMessage};
use war3_protocol::war3::War3Version;

/// 每個連線的玩家
pub struct ConnectedPlayer {
    pub player_id: String,
    pub nickname: String,
    pub war3_version: War3Version,
    pub addr: SocketAddr,
    pub last_heartbeat: Instant,
    pub tx: mpsc::UnboundedSender<ServerMessage>,
    pub hosting_room: Option<String>,
}

/// 房間狀態
pub struct Room {
    pub room_id: String,
    pub host_player_id: String,
    pub host_nickname: String,
    pub host_addr: SocketAddr,
    pub room_name: String,
    pub map_name: String,
    pub max_players: u8,
    pub current_players: u8,
    pub war3_version: War3Version,
}

/// 伺服器全域狀態
pub struct AppState {
    pub players: RwLock<HashMap<String, ConnectedPlayer>>,
    pub rooms: RwLock<HashMap<String, Room>>,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            players: RwLock::new(HashMap::new()),
            rooms: RwLock::new(HashMap::new()),
        })
    }

    /// 產生玩家列表快照（不含 IP）
    pub async fn player_snapshot(&self) -> Vec<PlayerInfo> {
        let players = self.players.read().await;
        players
            .values()
            .map(|p| PlayerInfo {
                player_id: p.player_id.clone(),
                nickname: p.nickname.clone(),
                war3_version: p.war3_version,
                is_hosting: p.hosting_room.is_some(),
            })
            .collect()
    }

    /// 產生房間列表快照
    pub async fn room_snapshot(&self) -> Vec<RoomInfo> {
        let rooms = self.rooms.read().await;
        rooms
            .values()
            .map(|r| RoomInfo {
                room_id: r.room_id.clone(),
                host_nickname: r.host_nickname.clone(),
                room_name: r.room_name.clone(),
                map_name: r.map_name.clone(),
                max_players: r.max_players,
                current_players: r.current_players,
                war3_version: r.war3_version,
            })
            .collect()
    }

    /// 廣播訊息給所有玩家
    pub async fn broadcast(&self, msg: &ServerMessage) {
        let players = self.players.read().await;
        for player in players.values() {
            let _ = player.tx.send(msg.clone());
        }
    }

    /// 廣播玩家和房間列表更新
    pub async fn broadcast_state(&self) {
        let player_update = ServerMessage::PlayerUpdate {
            players: self.player_snapshot().await,
        };
        let room_update = ServerMessage::RoomUpdate {
            rooms: self.room_snapshot().await,
        };
        self.broadcast(&player_update).await;
        self.broadcast(&room_update).await;
    }

    /// 移除玩家，清理其房間
    pub async fn remove_player(&self, player_id: &str) {
        let hosting_room = {
            let mut players = self.players.write().await;
            let player = players.remove(player_id);
            player.and_then(|p| p.hosting_room)
        };

        if let Some(room_id) = hosting_room {
            self.rooms.write().await.remove(&room_id);
        }

        self.broadcast_state().await;
    }
}
