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

/// Tunnel session 等待配對的資料
struct PendingTunnel {
    /// host 端的 WS sender
    host_tx: mpsc::Sender<Message>,
    /// 通知 host 端 joiner 已配對
    paired_notify: oneshot::Sender<mpsc::Sender<Message>>,
    /// Token 綁定的 host IP
    host_ip: IpAddr,
    created_at: Instant,
}

pub struct TunnelState {
    /// token → 等待配對的 tunnel（host 先到）
    pending: RwLock<HashMap<String, PendingTunnel>>,
    /// per-IP tunnel 連線數
    connections_per_ip: RwLock<HashMap<IpAddr, u32>>,
}

impl TunnelState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pending: RwLock::new(HashMap::new()),
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

    pub async fn release_tunnel_connection(&self, ip: IpAddr) {
        let mut conns = self.connections_per_ip.write().await;
        if let Some(count) = conns.get_mut(&ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                conns.remove(&ip);
            }
        }
    }

    /// 清除超時未配對的 tunnel
    pub async fn cleanup_expired(&self) {
        let mut pending = self.pending.write().await;
        let before = pending.len();
        pending.retain(|token, t| {
            let expired = t.created_at.elapsed() > PAIRING_TIMEOUT;
            if expired {
                warn!(token = &token[..8], "Tunnel 配對逾時，清理");
            }
            !expired
        });
        let removed = before - pending.len();
        if removed > 0 {
            info!(removed, "清理過期 tunnel sessions");
        }
    }
}

/// 處理 tunnel WebSocket 連線
pub async fn handle_tunnel(
    socket: WebSocket,
    client_ip: IpAddr,
    token: String,
    role: String,
    tunnel_state: Arc<TunnelState>,
) {
    let token_short = token.get(..8).unwrap_or(&token).to_string();

    match role.as_str() {
        "host" => handle_host(socket, client_ip, token, &token_short, tunnel_state).await,
        "join" => handle_joiner(socket, client_ip, token, &token_short, tunnel_state).await,
        _ => {
            warn!(%token_short, %role, "無效的 tunnel role");
        }
    }
}

async fn handle_host(
    socket: WebSocket,
    client_ip: IpAddr,
    token: String,
    token_short: &str,
    tunnel_state: Arc<TunnelState>,
) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (host_tx, mut host_rx) = mpsc::channel::<Message>(RELAY_CHANNEL_SIZE);
    let (paired_tx, paired_rx) = oneshot::channel::<mpsc::Sender<Message>>();

    // 註冊等待配對
    {
        let mut pending = tunnel_state.pending.write().await;
        pending.insert(
            token.clone(),
            PendingTunnel {
                host_tx: host_tx.clone(),
                paired_notify: paired_tx,
                host_ip: client_ip,
                created_at: Instant::now(),
            },
        );
    }

    info!(%token_short, %client_ip, "Host tunnel 已註冊，等待 joiner 配對");

    // 等待 joiner 配對或超時
    let joiner_tx = tokio::select! {
        result = paired_rx => {
            match result {
                Ok(tx) => tx,
                Err(_) => {
                    warn!(%token_short, "配對 channel 關閉");
                    return;
                }
            }
        }
        _ = tokio::time::sleep(PAIRING_TIMEOUT) => {
            warn!(%token_short, "Host 等待 joiner 配對逾時 ({}s)", PAIRING_TIMEOUT.as_secs());
            tunnel_state.pending.write().await.remove(&token);
            let _ = ws_sender.send(Message::Close(None)).await;
            return;
        }
    };

    // Drop 原始 host_tx：唯一的 sender 現在只有 joiner relay 持有的那份。
    // 這樣 joiner relay 結束時 host_rx.recv() 才會返回 None。
    drop(host_tx);

    info!(%token_short, "Tunnel 配對成功，開始 relay");

    // 雙向 relay
    relay(
        &mut ws_sender,
        &mut ws_receiver,
        &mut host_rx,
        joiner_tx,
        token_short,
        "host",
    )
    .await;
}

async fn handle_joiner(
    socket: WebSocket,
    client_ip: IpAddr,
    token: String,
    token_short: &str,
    tunnel_state: Arc<TunnelState>,
) {
    // 查找對應的 pending tunnel
    let pending = {
        let mut pending_map = tunnel_state.pending.write().await;
        pending_map.remove(&token)
    };

    let pending = match pending {
        Some(p) => p,
        None => {
            warn!(%token_short, "Tunnel token 無效或已過期");
            let (mut sender, _) = socket.split();
            let _ = sender.send(Message::Close(None)).await;
            return;
        }
    };

    // 驗證 token 是否已過期（二次檢查）
    if pending.created_at.elapsed() > PAIRING_TIMEOUT {
        warn!(%token_short, "Tunnel token 已過期");
        let (mut sender, _) = socket.split();
        let _ = sender.send(Message::Close(None)).await;
        return;
    }

    info!(%token_short, %client_ip, host_ip = %pending.host_ip, "Joiner 配對成功");

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (joiner_tx, mut joiner_rx) = mpsc::channel::<Message>(RELAY_CHANNEL_SIZE);

    // 通知 host 端配對成功，傳送 joiner 的 sender
    if pending.paired_notify.send(joiner_tx.clone()).is_err() {
        warn!(%token_short, "Host 已斷線，無法配對");
        let _ = ws_sender.send(Message::Close(None)).await;
        return;
    }

    // 雙向 relay
    relay(
        &mut ws_sender,
        &mut ws_receiver,
        &mut joiner_rx,
        pending.host_tx,
        token_short,
        "join",
    )
    .await;
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
            // 從對端 relay 來的資料 → 送出去
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
            // 從這端收到的資料 → 轉發給對端
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        let len = data.len();

                        // Rate limiting
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
        let (paired_tx, _) = oneshot::channel();

        state.pending.write().await.insert(
            "test-token".into(),
            PendingTunnel {
                host_tx: tx,
                paired_notify: paired_tx,
                host_ip: "1.2.3.4".parse().unwrap(),
                created_at: Instant::now() - PAIRING_TIMEOUT - Duration::from_secs(1),
            },
        );

        assert_eq!(state.pending.read().await.len(), 1);
        state.cleanup_expired().await;
        assert_eq!(state.pending.read().await.len(), 0);
    }
}
