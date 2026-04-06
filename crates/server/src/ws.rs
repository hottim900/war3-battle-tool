use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;
use war3_protocol::messages::{ClientMessage, ServerMessage};

use crate::state::{AppState, ConnectedPlayer, Room};
use crate::tunnel::TunnelState;

const MAX_MESSAGES_PER_SECOND: u32 = 10;
const MAX_MESSAGE_SIZE: usize = 4096;
const MAX_TOTAL_PLAYERS: usize = 500;
const MAX_TOTAL_ROOMS: usize = 200;
const JOIN_COOLDOWN: Duration = Duration::from_secs(5);
const WEB_VIEWER_TIMEOUT: Duration = Duration::from_secs(120);
const WEB_VIEWER_PREFIX: &str = "__web-viewer-";

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
pub async fn handle_socket(
    socket: WebSocket,
    client_ip: IpAddr,
    state: Arc<AppState>,
    tunnel_state: Arc<TunnelState>,
) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(64);

    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match serde_json::to_string(&msg) {
                Ok(json) => {
                    if ws_sender.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    warn!("ServerMessage 序列化失敗: {e}");
                }
            }
        }
    });

    let player_id = Uuid::new_v4().to_string();
    let mut registered = false;
    let mut is_web_viewer = false;
    let mut rate_limiter = RateLimiter::new();
    let mut last_join_at: Option<Instant> = None;
    let (wv_timeout_tx, wv_timeout_rx) = tokio::sync::oneshot::channel::<()>();
    let mut wv_timeout_tx = Some(wv_timeout_tx);

    let receive_loop = async {
        while let Some(Ok(msg)) = ws_receiver.next().await {
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Close(_) => break,
                _ => continue,
            };

            if text.len() > MAX_MESSAGE_SIZE {
                let _ = tx.try_send(ServerMessage::Error {
                    message: "訊息過大".into(),
                });
                continue;
            }

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
                    client_version: _,
                } => {
                    if registered {
                        let _ = tx.try_send(ServerMessage::Error {
                            message: "已經註冊過了".into(),
                        });
                        continue;
                    }

                    if state.players.read().await.len() >= MAX_TOTAL_PLAYERS {
                        let _ = tx.try_send(ServerMessage::Error {
                            message: "伺服器已滿".into(),
                        });
                        continue;
                    }

                    is_web_viewer = nickname.starts_with(WEB_VIEWER_PREFIX);

                    if is_web_viewer {
                        info!(%client_ip, %nickname, "Web viewer 連線");
                        if let Some(tx) = wv_timeout_tx.take() {
                            let _ = tx.send(());
                        }
                    } else {
                        info!(%client_ip, %nickname, %war3_version, "玩家上線");
                    }

                    let player = ConnectedPlayer {
                        player_id: player_id.clone(),
                        nickname,
                        war3_version,
                        client_ip,
                        last_heartbeat: Instant::now(),
                        tx: tx.clone(),
                        hosting_room: None,
                        joined_room: None,
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
                    if !is_web_viewer {
                        let _ = tx.try_send(ServerMessage::YourObservedAddr {
                            ip: client_ip.to_string(),
                        });
                    }

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
                    gameinfo,
                } => {
                    if !registered || is_web_viewer {
                        continue;
                    }

                    let player_data = {
                        let players = state.players.read().await;
                        match players.get(&player_id) {
                            Some(p) if p.hosting_room.is_some() => {
                                let _ = tx.try_send(ServerMessage::Error {
                                    message: "已有房間，請先關閉".into(),
                                });
                                None
                            }
                            Some(p) => Some((p.nickname.clone(), p.client_ip, p.war3_version)),
                            None => None,
                        }
                    };

                    let (nickname, _player_ip, war3_version) = match player_data {
                        Some(data) => data,
                        None => continue,
                    };

                    if state.rooms.read().await.len() >= MAX_TOTAL_ROOMS {
                        let _ = tx.try_send(ServerMessage::Error {
                            message: "伺服器房間已滿".into(),
                        });
                        continue;
                    }

                    let room_id = Uuid::new_v4().to_string();
                    let room = Room {
                        room_id: room_id.clone(),
                        host_player_id: player_id.clone(),
                        host_nickname: nickname,
                        room_name,
                        map_name,
                        max_players,
                        current_players: 1,
                        war3_version,
                        gameinfo,
                    };

                    info!(%room_id, "房間建立");
                    state.rooms.write().await.insert(room_id.clone(), room);

                    if let Some(player) = state.players.write().await.get_mut(&player_id) {
                        player.hosting_room = Some(room_id);
                    }

                    state.broadcast_state().await;
                }

                ClientMessage::CloseRoom => {
                    if !registered || is_web_viewer {
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

                ClientMessage::Ping { ts } => {
                    let _ = tx.try_send(ServerMessage::Pong { ts });
                }

                ClientMessage::UPnPMapped {
                    external_addr,
                    tunnel_token,
                } => {
                    if !registered || is_web_viewer {
                        continue;
                    }

                    // SSRF 防護：驗證 external_addr 非 RFC1918/loopback/link-local
                    let parsed_addr = match external_addr.parse::<SocketAddr>() {
                        Ok(addr) => addr,
                        Err(_) => {
                            warn!(
                                %client_ip,
                                %external_addr,
                                "UPnPMapped: external_addr 格式錯誤"
                            );
                            continue;
                        }
                    };
                    if !is_safe_external_addr(parsed_addr.ip()) {
                        warn!(
                            %client_ip,
                            %external_addr,
                            "UPnPMapped: external_addr 被拒絕（不安全位址）"
                        );
                        continue;
                    }

                    // Auth check: 只有 host 可以送 UPnPMapped
                    let joiner_player_id = tunnel_state
                        .lookup_upnp_pending(&tunnel_token, client_ip)
                        .await;
                    let Some(joiner_player_id) = joiner_player_id else {
                        warn!(
                            %client_ip,
                            token = tunnel_token.get(..8).unwrap_or(&tunnel_token),
                            "UPnPMapped: token 不存在或 IP 不匹配"
                        );
                        continue;
                    };

                    // 找到 joiner 的 tx channel，unicast PeerUPnPAddr
                    let players = state.players.read().await;
                    let sent = players.values().any(|p| {
                        if p.player_id == joiner_player_id && p.disconnected_at.is_none() {
                            let _ = p.tx.try_send(ServerMessage::PeerUPnPAddr {
                                external_addr: external_addr.clone(),
                            });
                            true
                        } else {
                            false
                        }
                    });

                    if sent {
                        info!(
                            token = tunnel_token.get(..8).unwrap_or(&tunnel_token),
                            %external_addr,
                            "UPnPMapped: 已轉發 PeerUPnPAddr 給 joiner"
                        );
                    } else {
                        warn!(
                            token = tunnel_token.get(..8).unwrap_or(&tunnel_token),
                            "UPnPMapped: 找不到 joiner"
                        );
                    }
                }

                ClientMessage::JoinRoom { room_id } => {
                    if !registered || is_web_viewer {
                        continue;
                    }

                    if let Some(last) = last_join_at
                        && last.elapsed() < JOIN_COOLDOWN
                    {
                        let _ = tx.try_send(ServerMessage::Error {
                            message: "加入太頻繁，請稍後再試".into(),
                        });
                        continue;
                    }

                    let room_data = {
                        let rooms = state.rooms.read().await;
                        rooms.get(&room_id).map(|r| {
                            (
                                r.host_player_id.clone(),
                                r.war3_version,
                                r.current_players >= r.max_players,
                                r.gameinfo.clone(),
                            )
                        })
                    };

                    let (host_player_id, room_version, room_full, gameinfo) = match room_data {
                        Some(data) => data,
                        None => {
                            let _ = tx.try_send(ServerMessage::join_failure());
                            continue;
                        }
                    };

                    let players = state.players.read().await;

                    let joiner = match players.get(&player_id) {
                        Some(p) => p,
                        None => continue,
                    };

                    if joiner.war3_version != room_version {
                        let _ = tx.try_send(ServerMessage::join_failure());
                        continue;
                    }

                    if room_full {
                        let _ = tx.try_send(ServerMessage::join_failure());
                        continue;
                    }

                    let host = match players.get(&host_player_id) {
                        Some(p) if p.disconnected_at.is_none() => p,
                        _ => {
                            let _ = tx.try_send(ServerMessage::join_failure());
                            continue;
                        }
                    };
                    let host_ip = host.client_ip;

                    let tunnel_token = Uuid::new_v4().to_string();

                    // 註冊 IP-bound token（含 joiner player_id，UPnP 配對用）
                    tunnel_state
                        .register_token(tunnel_token.clone(), host_ip, client_ip, player_id.clone())
                        .await;

                    // P2P 直連：先送 StunInfo，再送 JoinResult/PlayerJoined
                    // 確保 client 啟動 tunnel 時已知道 peer_addr
                    let _ = tx.try_send(ServerMessage::StunInfo {
                        peer_addr: host_ip.to_string(),
                    });
                    let _ = host.tx.try_send(ServerMessage::StunInfo {
                        peer_addr: client_ip.to_string(),
                    });

                    let _ = tx.try_send(ServerMessage::JoinResult {
                        success: true,
                        room_id: Some(room_id.clone()),
                        tunnel_token: Some(tunnel_token.clone()),
                        gameinfo: if gameinfo.is_empty() {
                            None
                        } else {
                            Some(gameinfo)
                        },
                    });

                    let joiner_nickname = joiner.nickname.clone();

                    let _ = host.tx.try_send(ServerMessage::PlayerJoined {
                        nickname: joiner_nickname,
                        tunnel_token: tunnel_token.clone(),
                    });

                    drop(players);

                    // 先寫 joined_room（清舊房 + 設新房），再 increment room count
                    // 順序確保 remove_player 能正確 decrement
                    {
                        let mut players = state.players.write().await;
                        if let Some(player) = players.get_mut(&player_id) {
                            let old_room = player.joined_room.replace(room_id.clone());
                            // 離開舊房間：decrement count
                            if let Some(old_id) = old_room {
                                drop(players);
                                if let Some(room) = state.rooms.write().await.get_mut(&old_id) {
                                    room.current_players = room.current_players.saturating_sub(1);
                                }
                            } else {
                                drop(players);
                            }
                        }
                    }
                    if let Some(room) = state.rooms.write().await.get_mut(&room_id) {
                        room.current_players = room.current_players.saturating_add(1);
                    }

                    last_join_at = Some(Instant::now());
                    info!(%room_id, %player_id, "玩家加入房間");
                    state.broadcast_state().await;
                }
            }
        }
    };

    tokio::select! {
        _ = send_task => {
            warn!(%player_id, "send task 結束");
        }
        _ = receive_loop => {}
        _ = async {
            // 等待 web-viewer 註冊通知，然後開始 120s 倒數
            let _ = wv_timeout_rx.await;
            tokio::time::sleep(WEB_VIEWER_TIMEOUT).await;
        } => {
            info!(%player_id, "Web viewer 超時斷線 ({}s)", WEB_VIEWER_TIMEOUT.as_secs());
        }
    }

    if registered {
        if is_web_viewer {
            // Web viewer 直接移除，不走 grace period
            state.remove_player(&player_id).await;
        } else {
            info!(%player_id, "玩家斷線，進入 grace period");
            state.mark_disconnected(&player_id).await;
        }
        state.broadcast_state().await;
    }
}

/// SSRF 防護：拒絕 RFC1918、loopback、link-local 位址
fn is_safe_external_addr(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
            {
                return false;
            }
            // RFC 6598 CGNAT shared address space (100.64.0.0/10)
            let octets = v4.octets();
            !(octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        IpAddr::V6(v6) => {
            !v6.is_loopback()
                && !v6.is_unspecified()
                // ULA (fc00::/7) 和 link-local (fe80::/10)
                && !matches!(v6.segments()[0], 0xfc00..=0xfdff | 0xfe80..=0xfebf)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── T12: Server SSRF validation (IPv4) ──

    #[test]
    fn ssrf_rejects_private_ipv4() {
        let ip: IpAddr = "192.168.1.1".parse().unwrap();
        assert!(!is_safe_external_addr(ip));
    }

    #[test]
    fn ssrf_rejects_loopback_ipv4() {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(!is_safe_external_addr(ip));
    }

    #[test]
    fn ssrf_rejects_link_local_ipv4() {
        let ip: IpAddr = "169.254.1.1".parse().unwrap();
        assert!(!is_safe_external_addr(ip));
    }

    #[test]
    fn ssrf_rejects_cgnat_ipv4() {
        let ip: IpAddr = "100.64.1.1".parse().unwrap();
        assert!(!is_safe_external_addr(ip));
        let ip: IpAddr = "100.127.255.255".parse().unwrap();
        assert!(!is_safe_external_addr(ip));
    }

    #[test]
    fn ssrf_accepts_public_ipv4() {
        let ip: IpAddr = "8.8.8.8".parse().unwrap();
        assert!(is_safe_external_addr(ip));
    }

    // ── T12b: Server SSRF validation (IPv6) ──

    #[test]
    fn ssrf_rejects_ipv6_loopback() {
        let ip: IpAddr = "::1".parse().unwrap();
        assert!(!is_safe_external_addr(ip));
    }

    #[test]
    fn ssrf_rejects_ipv6_link_local() {
        let ip: IpAddr = "fe80::1".parse().unwrap();
        assert!(!is_safe_external_addr(ip));
    }

    #[test]
    fn ssrf_accepts_public_ipv6() {
        let ip: IpAddr = "2001:db8::1".parse().unwrap();
        assert!(is_safe_external_addr(ip));
    }
}
