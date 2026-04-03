use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;
use war3_protocol::messages::{ClientMessage, ServerMessage};

use crate::state::{AppState, ConnectedPlayer, Room};

const MAX_MESSAGES_PER_SECOND: u32 = 10;
const MAX_MESSAGE_SIZE: usize = 4096;
const MAX_TOTAL_PLAYERS: usize = 500;
const MAX_TOTAL_ROOMS: usize = 200;
const JOIN_COOLDOWN: Duration = Duration::from_secs(5);

/// 簡單的 token bucket 速率限制器
struct RateLimiter {
    tokens: u32,
    last_refill: Instant,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            tokens: MAX_MESSAGES_PER_SECOND,
            last_refill: Instant::now(),
        }
    }

    fn allow(&mut self) -> bool {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            self.tokens = MAX_MESSAGES_PER_SECOND;
            self.last_refill = Instant::now();
        }
        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }
}

/// 處理單個 WebSocket 連線
pub async fn handle_socket(socket: WebSocket, addr: SocketAddr, state: Arc<AppState>) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(64);

    // 背景任務：從 channel 讀取訊息送到 WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg)
                && ws_sender.send(Message::Text(json.into())).await.is_err()
            {
                break;
            }
        }
    });

    let player_id = Uuid::new_v4().to_string();
    let mut registered = false;
    let mut rate_limiter = RateLimiter::new();
    let mut last_join_at: Option<Instant> = None;

    // 主迴圈：用 select! 確保 send_task 死掉時也能退出
    let receive_loop = async {
        while let Some(Ok(msg)) = ws_receiver.next().await {
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Close(_) => break,
                _ => continue,
            };

            // 訊息大小限制
            if text.len() > MAX_MESSAGE_SIZE {
                let _ = tx.try_send(ServerMessage::Error {
                    message: "訊息過大".into(),
                });
                continue;
            }

            // 速率限制
            if !rate_limiter.allow() {
                let _ = tx.try_send(ServerMessage::Error {
                    message: "訊息發送太頻繁，請稍後再試".into(),
                });
                continue;
            }

            let client_msg: ClientMessage = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(_) => {
                    let _ = tx.try_send(ServerMessage::Error {
                        message: "無法解析訊息".into(),
                    });
                    continue;
                }
            };

            // 驗證輸入
            if let Err(e) = client_msg.validate() {
                let _ = tx.try_send(ServerMessage::Error {
                    message: e.to_string(),
                });
                continue;
            }

            match client_msg {
                ClientMessage::Register {
                    nickname,
                    war3_version,
                } => {
                    if registered {
                        let _ = tx.try_send(ServerMessage::Error {
                            message: "已經註冊過了".into(),
                        });
                        continue;
                    }

                    // C3: 全域玩家上限
                    if state.players.read().await.len() >= MAX_TOTAL_PLAYERS {
                        let _ = tx.try_send(ServerMessage::Error {
                            message: "伺服器已滿".into(),
                        });
                        continue;
                    }

                    info!(%addr, %nickname, %war3_version, "玩家上線");

                    let player = ConnectedPlayer {
                        player_id: player_id.clone(),
                        nickname,
                        war3_version,
                        addr,
                        last_heartbeat: Instant::now(),
                        tx: tx.clone(),
                        hosting_room: None,
                        disconnected_at: None,
                    };

                    state
                        .players
                        .write()
                        .await
                        .insert(player_id.clone(), player);
                    registered = true;

                    let _ = tx.try_send(ServerMessage::Welcome {
                        player_id: player_id.clone(),
                    });

                    // 發送當前狀態
                    let _ = tx.try_send(ServerMessage::PlayerUpdate {
                        players: state.player_snapshot().await,
                    });
                    let _ = tx.try_send(ServerMessage::RoomUpdate {
                        rooms: state.room_snapshot().await,
                    });

                    state.broadcast_state().await;
                }

                ClientMessage::Heartbeat => {
                    if let Some(player) = state.players.write().await.get_mut(&player_id) {
                        player.last_heartbeat = Instant::now();
                    }
                }

                ClientMessage::CreateRoom {
                    room_name,
                    map_name,
                    max_players,
                } => {
                    if !registered {
                        continue;
                    }

                    // C2: 一人一房
                    let already_hosting = state
                        .players
                        .read()
                        .await
                        .get(&player_id)
                        .map(|p| p.hosting_room.is_some())
                        .unwrap_or(false);
                    if already_hosting {
                        let _ = tx.try_send(ServerMessage::Error {
                            message: "已有房間，請先關閉".into(),
                        });
                        continue;
                    }

                    // C2: 全域房間上限
                    if state.rooms.read().await.len() >= MAX_TOTAL_ROOMS {
                        let _ = tx.try_send(ServerMessage::Error {
                            message: "伺服器房間已滿".into(),
                        });
                        continue;
                    }

                    let room_id = Uuid::new_v4().to_string();
                    let players = state.players.read().await;
                    let player = match players.get(&player_id) {
                        Some(p) => p,
                        None => continue,
                    };

                    let room = Room {
                        room_id: room_id.clone(),
                        host_player_id: player_id.clone(),
                        host_nickname: player.nickname.clone(),
                        host_addr: player.addr,
                        room_name,
                        map_name,
                        max_players,
                        current_players: 1,
                        war3_version: player.war3_version,
                    };

                    drop(players);

                    info!(%room_id, "房間建立");
                    state.rooms.write().await.insert(room_id.clone(), room);

                    if let Some(player) = state.players.write().await.get_mut(&player_id) {
                        player.hosting_room = Some(room_id);
                    }

                    state.broadcast_state().await;
                }

                ClientMessage::CloseRoom => {
                    if !registered {
                        continue;
                    }

                    let room_id = {
                        let mut players = state.players.write().await;
                        if let Some(player) = players.get_mut(&player_id) {
                            player.hosting_room.take()
                        } else {
                            None
                        }
                    };

                    if let Some(room_id) = room_id {
                        info!(%room_id, "房間關閉");
                        state.rooms.write().await.remove(&room_id);
                        state.broadcast_state().await;
                    }
                }

                ClientMessage::JoinRoom { room_id } => {
                    if !registered {
                        continue;
                    }

                    // C1: Join 冷卻（防 IP 列舉）
                    if let Some(last) = last_join_at
                        && last.elapsed() < JOIN_COOLDOWN
                    {
                        let _ = tx.try_send(ServerMessage::Error {
                            message: "加入太頻繁，請稍後再試".into(),
                        });
                        continue;
                    }
                    last_join_at = Some(Instant::now());

                    let rooms = state.rooms.read().await;
                    let room = match rooms.get(&room_id) {
                        Some(r) => r,
                        None => {
                            let _ = tx.try_send(ServerMessage::JoinResult {
                                success: false,
                                host_ip: None,
                            });
                            continue;
                        }
                    };

                    // H4/C1: 版本檢查
                    let joiner_version = state
                        .players
                        .read()
                        .await
                        .get(&player_id)
                        .map(|p| p.war3_version);
                    if joiner_version != Some(room.war3_version) {
                        let _ = tx.try_send(ServerMessage::JoinResult {
                            success: false,
                            host_ip: None,
                        });
                        drop(rooms);
                        continue;
                    }

                    // C1: 房間人數檢查
                    if room.current_players >= room.max_players {
                        let _ = tx.try_send(ServerMessage::JoinResult {
                            success: false,
                            host_ip: None,
                        });
                        drop(rooms);
                        continue;
                    }

                    // M4: 檢查房主是否仍在線
                    let host_online = state
                        .players
                        .read()
                        .await
                        .get(&room.host_player_id)
                        .map(|p| p.disconnected_at.is_none())
                        .unwrap_or(false);
                    if !host_online {
                        let _ = tx.try_send(ServerMessage::JoinResult {
                            success: false,
                            host_ip: None,
                        });
                        drop(rooms);
                        continue;
                    }

                    let host_ip = room.host_addr.ip().to_string();
                    let host_player_id = room.host_player_id.clone();
                    drop(rooms);

                    let _ = tx.try_send(ServerMessage::JoinResult {
                        success: true,
                        host_ip: Some(host_ip),
                    });

                    let players = state.players.read().await;
                    let joiner_nickname = players
                        .get(&player_id)
                        .map(|p| p.nickname.clone())
                        .unwrap_or_default();
                    let joiner_ip = addr.ip().to_string();

                    if let Some(host) = players.get(&host_player_id) {
                        let _ = host.tx.try_send(ServerMessage::PlayerJoined {
                            nickname: joiner_nickname,
                            player_ip: joiner_ip,
                        });
                    }

                    info!(%room_id, %player_id, "玩家加入房間");
                }
            }
        }
    };

    // H2: select! 確保 send_task 或 receive_loop 任一結束都退出
    tokio::select! {
        _ = send_task => {
            warn!(%player_id, "send task 結束");
        }
        _ = receive_loop => {}
    }

    // 斷線：標記 grace period 而非立刻移除
    if registered {
        info!(%player_id, "玩家斷線，進入 grace period");
        state.mark_disconnected(&player_id).await;
        state.broadcast_state().await;
    }
}
