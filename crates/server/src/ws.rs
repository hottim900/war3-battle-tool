use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket};
use tokio::sync::mpsc;
use tracing::info;
use uuid::Uuid;
use war3_protocol::messages::{ClientMessage, ServerMessage};

use crate::state::{AppState, ConnectedPlayer, Room};

const MAX_MESSAGES_PER_SECOND: u32 = 10;
const MAX_MESSAGE_SIZE: usize = 4096;

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
        use futures_util::SinkExt;
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

    // 主迴圈：處理收到的訊息
    use futures_util::StreamExt;
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
            Err(e) => {
                let _ = tx.try_send(ServerMessage::Error {
                    message: format!("無法解析訊息: {e}"),
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

    // 斷線：標記 grace period 而非立刻移除
    if registered {
        info!(%player_id, "玩家斷線，進入 grace period");
        state.mark_disconnected(&player_id).await;
        state.broadcast_state().await;
    }

    send_task.abort();
}
