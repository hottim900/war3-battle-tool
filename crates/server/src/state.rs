use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{RwLock, mpsc};
use war3_protocol::messages::{PlayerInfo, RoomInfo, ServerMessage};
use war3_protocol::war3::War3Version;

const MAX_CONNECTIONS_PER_IP: u32 = 3;

pub struct ConnectedPlayer {
    pub player_id: String,
    pub nickname: String,
    pub war3_version: War3Version,
    pub client_ip: IpAddr,
    pub last_heartbeat: Instant,
    pub tx: mpsc::Sender<ServerMessage>,
    pub hosting_room: Option<String>,
    pub joined_room: Option<String>,
    pub disconnected_at: Option<Instant>,
}

pub struct Room {
    pub room_id: String,
    pub host_player_id: String,
    pub host_nickname: String,
    pub room_name: String,
    pub map_name: String,
    pub max_players: u8,
    pub current_players: u8,
    pub war3_version: War3Version,
    /// Host 的 GAMEINFO 封包 bytes（v0.2.0: joiner 加入時回傳）
    pub gameinfo: Vec<u8>,
}

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

    pub async fn try_acquire_connection(&self, ip: IpAddr) -> bool {
        let mut conns = self.connections_per_ip.write().await;
        let count = conns.entry(ip).or_insert(0);
        if *count >= MAX_CONNECTIONS_PER_IP {
            return false;
        }
        *count += 1;
        true
    }

    pub async fn release_connection(&self, ip: IpAddr) {
        let mut conns = self.connections_per_ip.write().await;
        if let Some(count) = conns.get_mut(&ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                conns.remove(&ip);
            }
        }
    }

    /// 廣播玩家和房間列表給所有在線玩家
    pub async fn broadcast_state(&self) {
        let players = self.players.read().await;
        let rooms = self.rooms.read().await;

        let player_update = ServerMessage::PlayerUpdate {
            players: players
                .values()
                .filter(|p| p.disconnected_at.is_none() && !p.nickname.starts_with("__web-viewer-"))
                .map(|p| PlayerInfo {
                    player_id: p.player_id.clone(),
                    nickname: p.nickname.clone(),
                    war3_version: p.war3_version,
                    is_hosting: p.hosting_room.is_some(),
                })
                .collect(),
        };
        let room_update = ServerMessage::RoomUpdate {
            rooms: rooms
                .values()
                .filter(|r| {
                    players
                        .get(&r.host_player_id)
                        .map(|p| p.disconnected_at.is_none())
                        .unwrap_or(false)
                })
                .map(|r| RoomInfo {
                    room_id: r.room_id.clone(),
                    host_nickname: r.host_nickname.clone(),
                    room_name: r.room_name.clone(),
                    map_name: r.map_name.clone(),
                    max_players: r.max_players,
                    current_players: r.current_players,
                    war3_version: r.war3_version,
                })
                .collect(),
        };

        for player in players.values() {
            if player.disconnected_at.is_none() {
                let _ = player.tx.try_send(player_update.clone());
                let _ = player.tx.try_send(room_update.clone());
            }
        }
    }

    pub async fn mark_disconnected(&self, player_id: &str) {
        let mut players = self.players.write().await;
        if let Some(player) = players.get_mut(player_id) {
            player.disconnected_at = Some(Instant::now());
        }
    }

    /// 移除玩家，清理其房間，並遞減已加入房間的人數
    pub async fn remove_player(&self, player_id: &str) {
        let (hosting_room, joined_room) = {
            let mut players = self.players.write().await;
            match players.remove(player_id) {
                Some(p) => (p.hosting_room, p.joined_room),
                None => return,
            }
        };

        let mut rooms = self.rooms.write().await;
        if let Some(room_id) = hosting_room {
            rooms.remove(&room_id);
        }
        if let Some(room_id) = joined_room
            && let Some(room) = rooms.get_mut(&room_id)
        {
            room.current_players = room.current_players.saturating_sub(1);
        }
    }

    pub async fn find_expired(
        &self,
        heartbeat_timeout: std::time::Duration,
        grace_period: std::time::Duration,
    ) -> Vec<String> {
        let players = self.players.read().await;
        players
            .iter()
            .filter(|(_, p)| {
                if let Some(dc_at) = p.disconnected_at {
                    dc_at.elapsed() > grace_period
                } else {
                    p.last_heartbeat.elapsed() > heartbeat_timeout
                }
            })
            .map(|(id, _)| id.clone())
            .collect()
    }
}
