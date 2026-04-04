use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{RwLock, mpsc, oneshot};
use tracing::{info, warn};

const PAIRING_TIMEOUT: Duration = Duration::from_secs(30);
const TUNNEL_IDLE_TIMEOUT: Duration = Duration::from_secs(300); // 5 分鐘
const RELAY_CHANNEL_SIZE: usize = 64;
const RATE_LIMIT_BYTES_PER_SEC: usize = 50 * 1024; // 50 KB/s
const MAX_TUNNEL_CONNECTIONS_PER_IP: u32 = 12;

/// 等待配對的一端（不分 host/joiner，先到先等）
struct WaitingPeer {
    tx: mpsc::Sender<Message>,
    notify: oneshot::Sender<mpsc::Sender<Message>>,
    role: String,
    created_at: Instant,
}

/// Token 綁定的 IP 資訊（JoinRoom 時建立）
struct TokenBinding {
    host_ip: IpAddr,
    joiner_ip: IpAddr,
}

pub struct TunnelState {
    /// token → 先到的一端（等待配對）
    waiting: RwLock<HashMap<String, WaitingPeer>>,
    /// token → IP binding（JoinRoom 時註冊，/tunnel 連線時驗證）
    token_bindings: RwLock<HashMap<String, TokenBinding>>,
    /// per-IP tunnel 連線數
    connections_per_ip: RwLock<HashMap<IpAddr, u32>>,
}

impl TunnelState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            waiting: RwLock::new(HashMap::new()),
            token_bindings: RwLock::new(HashMap::new()),
            connections_per_ip: RwLock::new(HashMap::new()),
        })
    }

    pub async fn try_acquire_tunnel_connection(&self, ip: IpAddr) -> bool {
        let mut conns = self.connections_per_ip.write().await;
        let count = conns.entry(ip).or_insert(0);
        if *count >= MAX_TUNNEL_CONNECTIONS_PER_IP {
            return false;
        }
        *count += 1;
        true
    }

    /// 註冊 token 的 IP 綁定（ws.rs JoinRoom 時呼叫）
    pub async fn register_token(&self, token: String, host_ip: IpAddr, joiner_ip: IpAddr) {
        self.token_bindings
            .write()
            .await
            .insert(token, TokenBinding { host_ip, joiner_ip });
    }

    /// 驗證 token 的 IP 是否匹配
    fn verify_ip(binding: &TokenBinding, role: &str, client_ip: IpAddr) -> bool {
        match role {
            "host" => binding.host_ip == client_ip,
            "join" => binding.joiner_ip == client_ip,
            _ => false,
        }
    }

    pub async fn release_tunnel_connection(&self, ip: IpAddr) {
        let mut conns = self.connections_per_ip.write().await;
        if let Some(count) = conns.get_mut(&ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                conns.remove(&ip);
            }
        }
    }

    /// 清除超時未配對的 tunnel 和過期的 token bindings
    pub async fn cleanup_expired(&self) {
        let mut waiting = self.waiting.write().await;
        let expired_tokens: Vec<String> = waiting
            .iter()
            .filter(|(_, w)| w.created_at.elapsed() > PAIRING_TIMEOUT)
            .map(|(token, _)| token.clone())
            .collect();

        for token in &expired_tokens {
            warn!(
                token = &token[..std::cmp::min(8, token.len())],
                "Tunnel 配對逾時，清理"
            );
            waiting.remove(token);
        }
        drop(waiting);

        if !expired_tokens.is_empty() {
            let mut bindings = self.token_bindings.write().await;
            for token in &expired_tokens {
                bindings.remove(token);
            }
            info!(removed = expired_tokens.len(), "清理過期 tunnel sessions");
        }
    }
}

/// 處理 tunnel WebSocket 連線（host 或 joiner 都走同一個入口）
pub async fn handle_tunnel(
    socket: WebSocket,
    client_ip: IpAddr,
    token: String,
    role: String,
    tunnel_state: Arc<TunnelState>,
) {
    let token_short = token.get(..8).unwrap_or(&token).to_string();

    // Token 驗證：必須有 binding（ws.rs JoinRoom 時註冊），且 IP 匹配
    {
        let bindings = tunnel_state.token_bindings.read().await;
        match bindings.get(&token) {
            None => {
                warn!(%token_short, %role, "Tunnel token 無效（未註冊）");
                let (mut sender, _) = socket.split();
                let _ = sender.send(Message::Close(None)).await;
                return;
            }
            Some(binding) if !TunnelState::verify_ip(binding, &role, client_ip) => {
                warn!(%token_short, %client_ip, %role, "Tunnel token IP 不匹配");
                let (mut sender, _) = socket.split();
                let _ = sender.send(Message::Close(None)).await;
                return;
            }
            Some(_) => {}
        }
    }

    // 嘗試配對：如果已有人在等，配對成功；否則自己等
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (my_tx, mut my_rx) = mpsc::channel::<Message>(RELAY_CHANNEL_SIZE);

    // 檢查是否有對方在等
    let existing = {
        let mut waiting = tunnel_state.waiting.write().await;
        waiting.remove(&token)
    };

    match existing {
        Some(peer) => {
            // 對方已在等，配對成功
            info!(%token_short, %role, peer_role = %peer.role, "Tunnel 配對成功（第二個到達）");

            // 清除 token binding
            tunnel_state.token_bindings.write().await.remove(&token);

            // 通知對方，把我的 tx 傳過去
            if peer.notify.send(my_tx.clone()).is_err() {
                warn!(%token_short, "對方已斷線，無法配對");
                let _ = ws_sender.send(Message::Close(None)).await;
                return;
            }

            // 開始 relay：我用 peer.tx 送資料給對方，對方用 my_tx 送資料給我
            relay(
                &mut ws_sender,
                &mut ws_receiver,
                &mut my_rx,
                peer.tx,
                &token_short,
                &role,
            )
            .await;
        }
        None => {
            // 我是第一個到，等待對方
            let (notify_tx, notify_rx) = oneshot::channel::<mpsc::Sender<Message>>();

            {
                let mut waiting = tunnel_state.waiting.write().await;
                waiting.insert(
                    token.clone(),
                    WaitingPeer {
                        tx: my_tx.clone(),
                        notify: notify_tx,
                        role: role.clone(),
                        created_at: Instant::now(),
                    },
                );
            }

            info!(%token_short, %role, "Tunnel 已註冊，等待對方配對");

            // 等待對方配對或超時
            let peer_tx = tokio::select! {
                result = notify_rx => {
                    match result {
                        Ok(tx) => tx,
                        Err(_) => {
                            warn!(%token_short, "配對 channel 關閉");
                            return;
                        }
                    }
                }
                _ = tokio::time::sleep(PAIRING_TIMEOUT) => {
                    warn!(%token_short, %role, "等待配對逾時 ({}s)", PAIRING_TIMEOUT.as_secs());
                    tunnel_state.waiting.write().await.remove(&token);
                    let _ = ws_sender.send(Message::Close(None)).await;
                    return;
                }
            };

            // Drop my_tx：唯一的 sender 現在只有對方持有的那份。
            // 對方 relay 結束時 my_rx.recv() 才會返回 None。
            drop(my_tx);

            info!(%token_short, %role, "Tunnel 配對成功（第一個到達），開始 relay");

            relay(
                &mut ws_sender,
                &mut ws_receiver,
                &mut my_rx,
                peer_tx,
                &token_short,
                &role,
            )
            .await;
        }
    }
}

/// 雙向 binary frame relay with rate limiting
async fn relay(
    ws_sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    ws_receiver: &mut futures_util::stream::SplitStream<WebSocket>,
    rx: &mut mpsc::Receiver<Message>,
    peer_tx: mpsc::Sender<Message>,
    token_short: &str,
    role: &str,
) {
    let mut bytes_this_second: usize = 0;
    let mut rate_window_start = Instant::now();
    let mut total_bytes: u64 = 0;
    let relay_start = Instant::now();

    let idle_deadline = tokio::time::sleep(TUNNEL_IDLE_TIMEOUT);
    tokio::pin!(idle_deadline);

    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Some(m) => {
                        if ws_sender.send(m).await.is_err() {
                            info!(%token_short, %role, "WS 送出失敗，關閉 tunnel");
                            break;
                        }
                        idle_deadline.as_mut().reset(tokio::time::Instant::now() + TUNNEL_IDLE_TIMEOUT);
                    }
                    None => {
                        info!(%token_short, %role, "對端已關閉 relay channel");
                        let _ = ws_sender.send(Message::Close(None)).await;
                        break;
                    }
                }
            }
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        let len = data.len();

                        if rate_window_start.elapsed() >= Duration::from_secs(1) {
                            bytes_this_second = 0;
                            rate_window_start = Instant::now();
                        }
                        bytes_this_second += len;
                        if bytes_this_second > RATE_LIMIT_BYTES_PER_SEC {
                            warn!(%token_short, %role, bytes = bytes_this_second, "Rate limit 超過，丟棄封包");
                            continue;
                        }

                        total_bytes += len as u64;
                        idle_deadline.as_mut().reset(tokio::time::Instant::now() + TUNNEL_IDLE_TIMEOUT);

                        if peer_tx.try_send(Message::Binary(data)).is_err() {
                            warn!(%token_short, %role, "對端 buffer 已滿或已關閉，丟棄封包");
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!(%token_short, %role, "收到 Close frame");
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = ws_sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        warn!(%token_short, %role, %e, "WS 接收錯誤");
                        break;
                    }
                    None => {
                        info!(%token_short, %role, "WS 連線關閉");
                        break;
                    }
                }
            }
            _ = &mut idle_deadline => {
                warn!(%token_short, %role, "Tunnel 閒置逾時，關閉");
                let _ = ws_sender.send(Message::Close(None)).await;
                break;
            }
        }
    }

    let duration = relay_start.elapsed();
    info!(
        %token_short,
        %role,
        duration_s = duration.as_secs(),
        total_kb = total_bytes / 1024,
        "Tunnel relay 結束"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tunnel_state_per_ip_limit() {
        let state = TunnelState::new();
        let ip: IpAddr = "1.2.3.4".parse().unwrap();

        for _ in 0..MAX_TUNNEL_CONNECTIONS_PER_IP {
            assert!(state.try_acquire_tunnel_connection(ip).await);
        }
        assert!(!state.try_acquire_tunnel_connection(ip).await);

        state.release_tunnel_connection(ip).await;
        assert!(state.try_acquire_tunnel_connection(ip).await);
    }

    #[tokio::test]
    async fn tunnel_state_cleanup_expired() {
        let state = TunnelState::new();
        let (tx, _rx) = mpsc::channel(1);
        let (notify_tx, _) = oneshot::channel();

        state.waiting.write().await.insert(
            "test-token".into(),
            WaitingPeer {
                tx,
                notify: notify_tx,
                role: "host".into(),
                created_at: Instant::now() - PAIRING_TIMEOUT - Duration::from_secs(1),
            },
        );

        assert_eq!(state.waiting.read().await.len(), 1);
        state.cleanup_expired().await;
        assert_eq!(state.waiting.read().await.len(), 0);
    }
}
