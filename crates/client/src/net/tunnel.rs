use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};

use super::quic;

/// War3 TCP 遊戲連接埠
const WAR3_PORT: u16 = 6112;
/// TCP ↔ relay 的 buffer 大小
const RELAY_BUF_SIZE: usize = 8192;
/// Joiner 端的 loopback IP（讓 War3 的 TCP 連線被我們攔截）
const JOINER_BIND_IP: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);
/// Host War3 監聽的位址
const HOST_WAR3_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, WAR3_PORT);

/// Tunnel 連線結果
#[derive(Debug)]
pub enum TunnelEvent {
    /// TCP proxy ready，可以開始 inject GAMEINFO
    ProxyReady,
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

/// 啟動 joiner 端 tunnel（QUIC 優先，fallback WS relay）
pub async fn run_joiner_tunnel(
    server_url: String,
    tunnel_token: String,
    peer_addr: Option<SocketAddr>,
    event_tx: tokio::sync::mpsc::UnboundedSender<TunnelEvent>,
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

    // 2. 嘗試 QUIC 直連
    if let Some(addr) = peer_addr {
        info!(%token_short, %addr, "嘗試 QUIC 直連");
        match quic::connect_direct(addr, &tunnel_token).await {
            Ok((quic_send, quic_recv)) => {
                info!(%token_short, "QUIC 直連成功，等待 War3 TCP");
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
                let result = bridge_tcp_quic(tcp_stream, quic_send, quic_recv, &token_short).await;
                let _ = event_tx.send(TunnelEvent::Finished {
                    error: result.err(),
                });
                return;
            }
            Err(e) => {
                info!(%token_short, %e, "QUIC 直連失敗，fallback WS relay");
            }
        }
    }

    // 3. Fallback: WS relay
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

    let result = bridge_tcp_ws(tcp_stream, ws_stream, &token_short).await;
    let _ = event_tx.send(TunnelEvent::Finished {
        error: result.err(),
    });
}

/// 啟動 host 端 tunnel（QUIC 優先，fallback WS relay）
pub async fn run_host_tunnel(
    server_url: String,
    tunnel_token: String,
    _peer_addr: Option<SocketAddr>,
    event_tx: tokio::sync::mpsc::UnboundedSender<TunnelEvent>,
) {
    let token_short = tunnel_token.get(..8).unwrap_or(&tunnel_token).to_string();

    // 1. 嘗試 QUIC 直連（host 監聽）
    info!(%token_short, "嘗試 QUIC host 監聽");
    match quic::accept_direct(&tunnel_token).await {
        Ok((quic_send, quic_recv)) => {
            info!(%token_short, "QUIC 直連成功，連接 War3 TCP");
            let tcp_stream = match connect_war3_tcp(&token_short).await {
                Some(s) => s,
                None => {
                    let _ = event_tx.send(TunnelEvent::Finished {
                        error: Some("無法連接 War3".to_string()),
                    });
                    return;
                }
            };
            let result = bridge_tcp_quic(tcp_stream, quic_send, quic_recv, &token_short).await;
            let _ = event_tx.send(TunnelEvent::Finished {
                error: result.err(),
            });
            return;
        }
        Err(e) => {
            info!(%token_short, %e, "QUIC host 監聽失敗，fallback WS relay");
        }
    }

    // 2. Fallback: WS relay
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
    let result = bridge_tcp_ws(tcp_stream, ws_stream, &token_short).await;
    let _ = event_tx.send(TunnelEvent::Finished {
        error: result.err(),
    });
}

/// 等待 War3 TCP 連入（joiner 端）
async fn accept_war3_tcp(
    listener: &tokio::net::TcpListener,
    token_short: &str,
) -> Option<TcpStream> {
    match listener.accept().await {
        Ok((stream, peer)) => {
            info!(%token_short, %peer, "War3 TCP 連入");
            Some(stream)
        }
        Err(e) => {
            error!(%token_short, %e, "TCP accept 失敗");
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

/// 雙向 bridge：TCP stream ↔ QUIC stream
async fn bridge_tcp_quic(
    tcp_stream: TcpStream,
    mut quic_send: quinn::SendStream,
    mut quic_recv: quinn::RecvStream,
    token_short: &str,
) -> Result<(), String> {
    tcp_stream
        .set_nodelay(true)
        .map_err(|e| format!("set_nodelay 失敗: {e}"))?;

    let (mut tcp_read, mut tcp_write) = tokio::io::split(tcp_stream);

    let mut total_tcp_to_quic: u64 = 0;
    let mut total_quic_to_tcp: u64 = 0;
    let mut buf_a = [0u8; RELAY_BUF_SIZE];
    let mut buf_b = [0u8; RELAY_BUF_SIZE];

    loop {
        tokio::select! {
            result = tcp_read.read(&mut buf_a) => {
                match result {
                    Ok(0) => {
                        info!(%token_short, "TCP 連線關閉（EOF）");
                        let _ = quic_send.finish();
                        break;
                    }
                    Ok(n) => {
                        total_tcp_to_quic += n as u64;
                        if quic_send.write_all(&buf_a[..n]).await.is_err() {
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
            result = quic_recv.read(&mut buf_b) => {
                match result {
                    Ok(Some(n)) => {
                        total_quic_to_tcp += n as u64;
                        if tcp_write.write_all(&buf_b[..n]).await.is_err() {
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

/// 雙向 bridge：TCP stream ↔ WebSocket stream
async fn bridge_tcp_ws(
    tcp_stream: TcpStream,
    ws_stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
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

    loop {
        tokio::select! {
            // TCP → WS：從 War3 讀取，轉為 WS Binary 送出
            result = tcp_read.read(&mut buf) => {
                match result {
                    Ok(0) => {
                        info!(%token_short, "TCP 連線關閉（EOF）");
                        let _ = ws_sender.send(Message::Close(None)).await;
                        break;
                    }
                    Ok(n) => {
                        total_tcp_to_ws += n as u64;
                        if ws_sender.send(Message::Binary(buf[..n].to_vec().into())).await.is_err() {
                            warn!(%token_short, "WS 送出失敗");
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(%token_short, %e, "TCP 讀取錯誤");
                        break;
                    }
                }
            }
            // WS → TCP：從 server relay 收到，寫入 War3 TCP
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        total_ws_to_tcp += data.len() as u64;
                        if tcp_write.write_all(&data).await.is_err() {
                            warn!(%token_short, "TCP 寫入失敗");
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!(%token_short, "WS 連線關閉");
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = ws_sender.send(Message::Pong(data)).await;
                    }
                    Some(Err(e)) => {
                        warn!(%token_short, %e, "WS 接收錯誤");
                        break;
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }

    info!(
        %token_short,
        tcp_to_ws_kb = total_tcp_to_ws / 1024,
        ws_to_tcp_kb = total_ws_to_tcp / 1024,
        "WS Bridge 結束"
    );

    Ok(())
}
