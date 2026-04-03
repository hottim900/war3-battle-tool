use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{RwLock, mpsc};
use war3_protocol::messages::{PlayerInfo, RoomInfo, ServerMessage};
use war3_protocol::war3::War3Version;

const MAX_CONNECTIONS_PER_IP: u32 = 3;

/// 每個連線的玩家
pub struct ConnectedPlayer {
    pub player_id: String,
    pub nickname: String,
    pub war3_version: War3Version,
    pub addr: SocketAddr,
    pub last_heartbeat: Instant,
    pub tx: mpsc::UnboundedSender<ServerMessage>,
    pub hosting_room: Option<String>,
    /// 斷線時間（None = 仍在線）
    pub disconnected_at: Option<Instant>,
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
    pub connections_per_ip: RwLock<HashMap<IpAddr, u32>>,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            players: RwLock::new(HashMap::new()),
            rooms: RwLock::new(HashMap::new()),
            connections_per_ip: RwLock::new(HashMap::new()),
        })
    }

    /// 嘗試取得連線許可，超過上限回傳 false
    pub async fn try_acquire_connection(&self, ip: IpAddr) -> bool {
        let mut conns = self.connections_per_ip.write().await;
        let count = conns.entry(ip).or_insert(0);
        if *count >= MAX_CONNECTIONS_PER_IP {
            return false;
        }
        *count += 1;
        true
    }

    /// 釋放連線計數
    pub async fn release_connection(&self, ip: IpAddr) {
        let mut conns = self.connections_per_ip.write().await;
        if let Some(count) = conns.get_mut(&ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                conns.remove(&ip);
            }
        }
    }

    /// 產生玩家列表快照（不含 IP，只包含在線玩家）
    pub async fn player_snapshot(&self) -> Vec<PlayerInfo> {
        let players = self.players.read().await;
        players
            .values()
            .filter(|p| p.disconnected_at.is_none())
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

    /// 廣播訊息給所有在線玩家
    pub async fn broadcast(&self, msg: &ServerMessage) {
        let players = self.players.read().await;
        for player in players.values() {
            if player.disconnected_at.is_none() {
                let _ = player.tx.send(msg.clone());
            }
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

    /// 標記玩家斷線（開始 grace period，不立刻清房）
    pub async fn mark_disconnected(&self, player_id: &str) {
        let mut players = self.players.write().await;
        if let Some(player) = players.get_mut(player_id) {
            player.disconnected_at = Some(Instant::now());
        }
    }

    /// 移除玩家，清理其房間（grace period 到期後由 cleanup task 呼叫）
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

    /// 清理超時斷線玩家（grace period 到期）
    pub async fn cleanup_expired(
        &self,
        heartbeat_timeout: std::time::Duration,
        grace_period: std::time::Duration,
    ) -> Vec<String> {
        let expired: Vec<String> = {
            let players = self.players.read().await;
            players
                .iter()
                .filter(|(_, p)| {
                    // 已斷線且超過 grace period
                    if let Some(dc_at) = p.disconnected_at {
                        dc_at.elapsed() > grace_period
                    } else {
                        // 在線但心跳超時
                        p.last_heartbeat.elapsed() > heartbeat_timeout
                    }
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        expired
    }
}
