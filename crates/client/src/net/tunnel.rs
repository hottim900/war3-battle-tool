use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};

use super::quic;

/// War3 TCP 遊戲連接埠
const WAR3_PORT: u16 = 6112;
/// TCP ↔ relay 的 buffer 大小
const RELAY_BUF_SIZE: usize = 8192;
/// Joiner 端的 loopback IP（讓 War3 的 TCP 連線被我們攔截）
const JOINER_BIND_IP: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);
/// Host War3 監聯的位址
const HOST_WAR3_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, WAR3_PORT);
/// WS relay drain timeout（swap 時等待殘留 WS 資料的最長時間）
const WS_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);

/// 傳輸路徑
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transport {
    Relay,
    Direct,
}

/// Tunnel 連線結果
#[derive(Debug)]
pub enum TunnelEvent {
    /// TCP proxy ready，可以開始 inject GAMEINFO
    ProxyReady,
    /// 傳輸路徑已確定
    TransportSelected(Transport),
    /// 傳輸從 relay 升級到 direct（mid-game swap 成功）
    TransportUpgraded,
    /// Tunnel 已結束（正常或錯誤）
    Finished { error: Option<String> },
    /// GAMEINFO 擷取完成，可以送 CreateRoom
    GameinfoCaptured {
        room_name: String,
        map_name: String,
        max_players: u8,
        gameinfo: Vec<u8>,
    },
}

/// QUIC swap 指令：背景 QUIC task 連線成功後傳給 bridge
struct QuicSwapReady {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}

/// QUIC swap handshake 信號
const SWAP_READY: u8 = 0x01;
const SWAP_ACK: u8 = 0x02;

/// 背景 QUIC 連線 + swap handshake（joiner 端）
async fn background_quic_joiner(
    peer_ip: IpAddr,
    tunnel_token: String,
    swap_tx: mpsc::Sender<QuicSwapReady>,
) {
    let token_short = tunnel_token.get(..8).unwrap_or(&tunnel_token);

    match quic::connect_direct(peer_ip, &tunnel_token).await {
        Ok((mut send, mut recv)) => {
            // Joiner 發起 swap handshake（整個流程含 timeout）
            let handshake = async {
                send.write_all(&[SWAP_READY]).await?;
                let mut buf = [0u8; 1];
                recv.read_exact(&mut buf).await?;
                anyhow::ensure!(buf[0] == SWAP_ACK, "unexpected swap response");
                Ok::<_, anyhow::Error>(())
            };
            match tokio::time::timeout(Duration::from_secs(3), handshake).await {
                Ok(Ok(())) => {
                    info!(%token_short, "QUIC swap handshake 完成");
                    let _ = swap_tx.send(QuicSwapReady { send, recv }).await;
                }
                _ => {
                    warn!(%token_short, "QUIC swap handshake 失敗");
                }
            }
        }
        Err(e) => {
            info!(%token_short, %e, "背景 QUIC 連線失敗，繼續 WS relay");
        }
    }
}

/// 背景 QUIC 連線 + swap handshake（host 端）
async fn background_quic_host(tunnel_token: String, swap_tx: mpsc::Sender<QuicSwapReady>) {
    let token_short = tunnel_token.get(..8).unwrap_or(&tunnel_token);

    match quic::accept_direct(&tunnel_token).await {
        Ok((mut send, mut recv)) => {
            // Host 等待 joiner 的 swap handshake（整個流程含 timeout）
            let handshake = async {
                let mut buf = [0u8; 1];
                recv.read_exact(&mut buf).await?;
                anyhow::ensure!(buf[0] == SWAP_READY, "unexpected swap signal");
                send.write_all(&[SWAP_ACK]).await?;
                Ok::<_, anyhow::Error>(())
            };
            match tokio::time::timeout(Duration::from_secs(3), handshake).await {
                Ok(Ok(())) => {
                    info!(%token_short, "QUIC swap handshake 完成");
                    let _ = swap_tx.send(QuicSwapReady { send, recv }).await;
                }
                _ => {
                    warn!(%token_short, "QUIC swap handshake 失敗");
                }
            }
        }
        Err(e) => {
            info!(%token_short, %e, "背景 QUIC 監聽失敗，繼續 WS relay");
        }
    }
}

/// 啟動 joiner 端 tunnel（WS relay + 背景 QUIC mid-game swap）
pub async fn run_joiner_tunnel(
    server_url: String,
    tunnel_token: String,
    peer_addr: Option<IpAddr>,
    event_tx: mpsc::UnboundedSender<TunnelEvent>,
) {
    let token_short = tunnel_token.get(..8).unwrap_or(&tunnel_token).to_string();

    // 1. Bind TCP listener on 127.0.0.2:6112
    let bind_addr = SocketAddr::V4(SocketAddrV4::new(JOINER_BIND_IP, WAR3_PORT));
    let tcp_listener = match tokio::net::TcpListener::bind(bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(%token_short, %e, "無法綁定 {bind_addr}");
            let _ = event_tx.send(TunnelEvent::Finished {
                error: Some(format!("無法綁定 {bind_addr}: {e}")),
            });
            return;
        }
    };

    info!(%token_short, %bind_addr, "TCP proxy 已就緒");
    let _ = event_tx.send(TunnelEvent::ProxyReady);

    // 2. 永遠先走 WS relay
    let _ = event_tx.send(TunnelEvent::TransportSelected(Transport::Relay));
    let base_url = server_url.strip_suffix("/ws").unwrap_or(&server_url);
    let ws_url = format!("{base_url}/tunnel?token={tunnel_token}&role=join");

    info!(%token_short, "Joiner tunnel: 連接 WS relay");
    let ws_stream = match tokio_tungstenite::connect_async(&ws_url).await {
        Ok((ws, _)) => ws,
        Err(e) => {
            error!(%token_short, %e, "Tunnel WS 連線失敗");
            let _ = event_tx.send(TunnelEvent::Finished {
                error: Some(format!("Tunnel 連線失敗: {e}")),
            });
            return;
        }
    };
    info!(%token_short, "Joiner tunnel: WS 已連線");

    let tcp_stream = match accept_war3_tcp(&tcp_listener, &token_short).await {
        Some(s) => s,
        None => {
            let _ = event_tx.send(TunnelEvent::Finished {
                error: Some("TCP accept 失敗".to_string()),
            });
            return;
        }
    };
    drop(tcp_listener);

    // 3. 背景 QUIC swap 嘗試
    let (swap_tx, swap_rx) = mpsc::channel::<QuicSwapReady>(1);
    if let Some(addr) = peer_addr {
        info!(%token_short, %addr, "背景嘗試 QUIC 直連");
        let token = tunnel_token.clone();
        tokio::spawn(background_quic_joiner(addr, token, swap_tx));
    }

    // 4. WS relay bridge（支援 mid-game swap 到 QUIC）
    let result =
        bridge_tcp_ws_with_swap(tcp_stream, ws_stream, swap_rx, &event_tx, &token_short).await;
    let _ = event_tx.send(TunnelEvent::Finished {
        error: result.err(),
    });
}

/// 啟動 host 端 tunnel（WS relay + 背景 QUIC mid-game swap）
pub async fn run_host_tunnel(
    server_url: String,
    tunnel_token: String,
    peer_addr: Option<IpAddr>,
    event_tx: mpsc::UnboundedSender<TunnelEvent>,
) {
    let token_short = tunnel_token.get(..8).unwrap_or(&tunnel_token).to_string();

    // 1. 永遠先走 WS relay
    let _ = event_tx.send(TunnelEvent::TransportSelected(Transport::Relay));
    let base_url = server_url.strip_suffix("/ws").unwrap_or(&server_url);
    let ws_url = format!("{base_url}/tunnel?token={tunnel_token}&role=host");

    info!(%token_short, "Host tunnel: 連接 WS relay");
    let ws_stream = match tokio_tungstenite::connect_async(&ws_url).await {
        Ok((ws, _)) => ws,
        Err(e) => {
            error!(%token_short, %e, "Tunnel WS 連線失敗");
            let _ = event_tx.send(TunnelEvent::Finished {
                error: Some(format!("Tunnel 連線失敗: {e}")),
            });
            return;
        }
    };
    info!(%token_short, "Host tunnel: WS 已連線，連接 War3 TCP");

    let tcp_stream = match connect_war3_tcp(&token_short).await {
        Some(s) => s,
        None => {
            let _ = event_tx.send(TunnelEvent::Finished {
                error: Some("無法連接 War3".to_string()),
            });
            return;
        }
    };

    info!(%token_short, "Host tunnel: TCP 連線成功，開始 relay");

    // 2. 背景 QUIC swap 嘗試
    let (swap_tx, swap_rx) = mpsc::channel::<QuicSwapReady>(1);
    if peer_addr.is_some() {
        info!(%token_short, "背景嘗試 QUIC host 監聽");
        let token = tunnel_token.clone();
        tokio::spawn(background_quic_host(token, swap_tx));
    }

    // 3. WS relay bridge（支援 mid-game swap 到 QUIC）
    let result =
        bridge_tcp_ws_with_swap(tcp_stream, ws_stream, swap_rx, &event_tx, &token_short).await;
    let _ = event_tx.send(TunnelEvent::Finished {
        error: result.err(),
    });
}

/// 等待 War3 TCP 連入（joiner 端），120 秒 timeout 避免永久佔用 port
async fn accept_war3_tcp(
    listener: &tokio::net::TcpListener,
    token_short: &str,
) -> Option<TcpStream> {
    match tokio::time::timeout(Duration::from_secs(120), listener.accept()).await {
        Ok(Ok((stream, peer))) => {
            info!(%token_short, %peer, "War3 TCP 連入");
            Some(stream)
        }
        Ok(Err(e)) => {
            error!(%token_short, %e, "TCP accept 失敗");
            None
        }
        Err(_) => {
            warn!(%token_short, "TCP accept 逾時（120s），釋放 port");
            None
        }
    }
}

/// 連接 War3 TCP（host 端）
async fn connect_war3_tcp(token_short: &str) -> Option<TcpStream> {
    match TcpStream::connect(SocketAddr::V4(HOST_WAR3_ADDR)).await {
        Ok(s) => {
            info!(%token_short, "War3 TCP 連線成功");
            Some(s)
        }
        Err(e) => {
            error!(%token_short, %e, "無法連接 War3 (127.0.0.1:6112)");
            None
        }
    }
}

/// 雙向 bridge：TCP ↔ WS relay，支援 mid-game swap 到 QUIC
///
/// 正常模式：TCP ↔ WS relay 雙向轉發
/// 收到 swap 信號：停止寫 WS → drain WS（buffer QUIC）→ 切換 QUIC bridge
async fn bridge_tcp_ws_with_swap(
    tcp_stream: TcpStream,
    ws_stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut swap_rx: mpsc::Receiver<QuicSwapReady>,
    event_tx: &mpsc::UnboundedSender<TunnelEvent>,
    token_short: &str,
) -> Result<(), String> {
    tcp_stream
        .set_nodelay(true)
        .map_err(|e| format!("set_nodelay 失敗: {e}"))?;

    let (mut tcp_read, mut tcp_write) = tokio::io::split(tcp_stream);
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    let mut total_tcp_to_ws: u64 = 0;
    let mut total_ws_to_tcp: u64 = 0;
    let mut buf = [0u8; RELAY_BUF_SIZE];

    // Phase 1: WS relay（可被 swap 中斷）
    let swap = loop {
        tokio::select! {
            // TCP → WS
            result = tcp_read.read(&mut buf) => {
                match result {
                    Ok(0) => {
                        info!(%token_short, "TCP 連線關閉（EOF）");
                        let _ = ws_sender.send(Message::Close(None)).await;
                        break None;
                    }
                    Ok(n) => {
                        total_tcp_to_ws += n as u64;
                        if ws_sender.send(Message::Binary(buf[..n].to_vec().into())).await.is_err() {
                            warn!(%token_short, "WS 送出失敗");
                            break None;
                        }
                    }
                    Err(e) => {
                        warn!(%token_short, %e, "TCP 讀取錯誤");
                        break None;
                    }
                }
            }
            // WS → TCP
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        total_ws_to_tcp += data.len() as u64;
                        if tcp_write.write_all(&data).await.is_err() {
                            warn!(%token_short, "TCP 寫入失敗");
                            break None;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!(%token_short, "WS 連線關閉");
                        break None;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = ws_sender.send(Message::Pong(data)).await;
                    }
                    Some(Err(e)) => {
                        warn!(%token_short, %e, "WS 接收錯誤");
                        break None;
                    }
                    Some(Ok(_)) => {}
                }
            }
            // Swap 信號：QUIC 已建立 + handshake 完成
            cmd = swap_rx.recv() => {
                if let Some(quic_ready) = cmd {
                    break Some(quic_ready);
                }
                // channel closed → 背景 QUIC 失敗，繼續 WS relay
            }
        }
    };

    info!(
        %token_short,
        tcp_to_ws_kb = total_tcp_to_ws / 1024,
        ws_to_tcp_kb = total_ws_to_tcp / 1024,
        "WS relay 階段結束"
    );

    // Phase 2: 如果有 swap，做 WS drain → QUIC 切換
    let Some(QuicSwapReady {
        send: mut quic_send,
        recv: mut quic_recv,
    }) = swap
    else {
        return Ok(());
    };

    info!(%token_short, "開始 mid-game swap: WS relay → QUIC direct");

    // Drain WS 殘留資料，同時 buffer QUIC 和 TCP 資料確保順序正確
    // 不送 WS Close — 讓 drain 自然結束，避免 server 提前回 Close 中斷 drain
    let mut quic_buffer: Vec<u8> = Vec::new();
    let mut tcp_outbound_buffer: Vec<u8> = Vec::new();
    let mut drain_quic_buf = [0u8; RELAY_BUF_SIZE];
    let mut drain_tcp_buf = [0u8; RELAY_BUF_SIZE];
    let drain_deadline = tokio::time::Instant::now() + WS_DRAIN_TIMEOUT;

    loop {
        tokio::select! {
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        if tcp_write.write_all(&data).await.is_err() {
                            warn!(%token_short, "TCP 寫入失敗（WS drain）");
                            return Ok(());
                        }
                    }
                    _ => break,
                }
            }
            result = quic_recv.read(&mut drain_quic_buf) => {
                match result {
                    Ok(Some(n)) => quic_buffer.extend_from_slice(&drain_quic_buf[..n]),
                    Ok(None) => {
                        warn!(%token_short, "QUIC stream 在 drain 期間關閉");
                        return Ok(());
                    }
                    Err(e) => {
                        warn!(%token_short, %e, "QUIC 讀取錯誤（drain）");
                        return Ok(());
                    }
                }
            }
            result = tcp_read.read(&mut drain_tcp_buf) => {
                match result {
                    Ok(0) => break,
                    Ok(n) => tcp_outbound_buffer.extend_from_slice(&drain_tcp_buf[..n]),
                    Err(_) => break,
                }
            }
            _ = tokio::time::sleep_until(drain_deadline) => {
                info!(%token_short, "WS drain timeout，切換到 QUIC");
                break;
            }
        }
    }

    // Flush buffered QUIC data → TCP（保持順序：先 WS drain，後 QUIC）
    if !quic_buffer.is_empty() {
        info!(%token_short, bytes = quic_buffer.len(), "寫入 QUIC buffer 資料");
        if tcp_write.write_all(&quic_buffer).await.is_err() {
            warn!(%token_short, "TCP 寫入 QUIC buffer 失敗");
            return Ok(());
        }
    }

    // Flush buffered TCP data → QUIC
    if !tcp_outbound_buffer.is_empty() && quic_send.write_all(&tcp_outbound_buffer).await.is_err() {
        warn!(%token_short, "QUIC 寫入 TCP buffer 失敗");
        return Ok(());
    }

    let _ = event_tx.send(TunnelEvent::TransportUpgraded);
    info!(%token_short, "Mid-game swap 完成，進入 QUIC direct bridge");

    // Phase 3: QUIC direct bridge（兩個 buf 因為 select! 需要獨立 borrow）
    let mut total_tcp_to_quic: u64 = 0;
    let mut total_quic_to_tcp: u64 = 0;
    let mut buf_tcp = [0u8; RELAY_BUF_SIZE];
    let mut buf_quic = [0u8; RELAY_BUF_SIZE];

    loop {
        tokio::select! {
            result = tcp_read.read(&mut buf_tcp) => {
                match result {
                    Ok(0) => {
                        info!(%token_short, "TCP 連線關閉（EOF）");
                        let _ = quic_send.finish();
                        break;
                    }
                    Ok(n) => {
                        total_tcp_to_quic += n as u64;
                        if quic_send.write_all(&buf_tcp[..n]).await.is_err() {
                            warn!(%token_short, "QUIC 送出失敗");
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(%token_short, %e, "TCP 讀取錯誤");
                        break;
                    }
                }
            }
            result = quic_recv.read(&mut buf_quic) => {
                match result {
                    Ok(Some(n)) => {
                        total_quic_to_tcp += n as u64;
                        if tcp_write.write_all(&buf_quic[..n]).await.is_err() {
                            warn!(%token_short, "TCP 寫入失敗");
                            break;
                        }
                    }
                    Ok(None) => {
                        info!(%token_short, "QUIC stream 關閉");
                        break;
                    }
                    Err(e) => {
                        warn!(%token_short, %e, "QUIC 接收錯誤");
                        break;
                    }
                }
            }
        }
    }

    info!(
        %token_short,
        tcp_to_quic_kb = total_tcp_to_quic / 1024,
        quic_to_tcp_kb = total_quic_to_tcp / 1024,
        "QUIC Bridge 結束"
    );

    Ok(())
}
