//! Premise 5 PoC: 驗證 proxy layer 能 mid-game swap transport，零資料遺失
//!
//! 模擬架構：
//!   sender ──TCP──→ bridge ──backend──→ receiver
//!
//! 測試流程：
//! 1. sender 持續送序號資料 (0, 1, 2, ...)
//! 2. bridge 透過 backend_a 轉發
//! 3. 中途發 swap 信號，bridge drain backend_a，切換到 backend_b
//! 4. receiver 驗證收到的序號完整且有序

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

const MSG_SIZE: usize = 4;
const BUF_SIZE: usize = 1024;

struct BackendHandle {
    reader: tokio::io::ReadHalf<tokio::io::DuplexStream>,
    writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
}

struct SwapCommand {
    new_backend: BackendHandle,
    done_tx: oneshot::Sender<()>,
}

// --- Test helpers ---

fn spawn_echo(stream: tokio::io::DuplexStream) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (mut r, mut w) = tokio::io::split(stream);
        let mut buf = [0u8; BUF_SIZE];
        loop {
            match r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if w.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        }
    })
}

async fn collect_sequences(
    mut reader: tokio::io::ReadHalf<tokio::io::DuplexStream>,
    capacity: usize,
) -> Vec<u32> {
    let mut received = Vec::with_capacity(capacity);
    let mut buf = [0u8; MSG_SIZE];
    let mut pos = 0;
    let mut read_buf = [0u8; BUF_SIZE];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), reader.read(&mut read_buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(n)) => {
                for &byte in &read_buf[..n] {
                    buf[pos] = byte;
                    pos += 1;
                    if pos == MSG_SIZE {
                        received.push(u32::from_be_bytes(buf));
                        pos = 0;
                    }
                }
            }
        }
    }
    received
}

fn assert_sequences_complete(received: &[u32], total: u32) {
    if received.len() != total as usize {
        let received_set: std::collections::HashSet<u32> = received.iter().copied().collect();
        let missing: Vec<u32> = (0..total).filter(|s| !received_set.contains(s)).collect();
        let first_10: Vec<_> = missing.iter().take(10).collect();
        panic!(
            "資料遺失！sent={total}, received={}, 遺失 {} 筆, 前 10 個: {first_10:?}",
            received.len(),
            missing.len(),
        );
    }
    for (i, &seq) in received.iter().enumerate() {
        assert_eq!(
            seq, i as u32,
            "順序錯誤 @ index {i}: expected {i}, got {seq}"
        );
    }
}

// --- Bridge ---

/// Drain backend 讀端到 EOF，轉發殘留資料到 TCP
/// 必須在 shutdown backend 寫端之後呼叫，確保遠端 echo 停止後能讀到 EOF
async fn drain_backend(
    backend_read: &mut tokio::io::ReadHalf<tokio::io::DuplexStream>,
    tcp_write: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>,
) {
    let mut buf = [0u8; BUF_SIZE];
    loop {
        match backend_read.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let _ = tcp_write.write_all(&buf[..n]).await;
            }
        }
    }
}

async fn swappable_bridge(
    mut tcp_read: tokio::io::ReadHalf<tokio::io::DuplexStream>,
    mut tcp_write: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    initial_backend: BackendHandle,
    mut swap_rx: mpsc::Receiver<SwapCommand>,
) {
    let mut backend_read = initial_backend.reader;
    let mut backend_write = initial_backend.writer;

    let mut buf_tcp = [0u8; BUF_SIZE];
    let mut buf_backend = [0u8; BUF_SIZE];

    loop {
        tokio::select! {
            result = tcp_read.read(&mut buf_tcp) => {
                match result {
                    Ok(0) => {
                        let _ = backend_write.shutdown().await;
                        drain_backend(&mut backend_read, &mut tcp_write).await;
                        break;
                    }
                    Err(_) => break,
                    Ok(n) => {
                        if backend_write.write_all(&buf_tcp[..n]).await.is_err() {
                            break;
                        }
                    }
                }
            }
            result = backend_read.read(&mut buf_backend) => {
                match result {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tcp_write.write_all(&buf_backend[..n]).await.is_err() {
                            break;
                        }
                    }
                }
            }
            cmd = swap_rx.recv() => {
                match cmd {
                    Some(swap) => {
                        let _ = backend_write.shutdown().await;
                        drain_backend(&mut backend_read, &mut tcp_write).await;
                        backend_read = swap.new_backend.reader;
                        backend_write = swap.new_backend.writer;
                        let _ = swap.done_tx.send(());
                    }
                    None => break,
                }
            }
        }
    }
}

// --- Tests ---

#[tokio::test]
async fn test_mid_game_swap_zero_data_loss() {
    let (tcp_client, tcp_bridge) = tokio::io::duplex(64 * 1024);
    let (tcp_bridge_read, tcp_bridge_write) = tokio::io::split(tcp_bridge);

    let (backend_a_bridge, backend_a_remote) = tokio::io::duplex(64 * 1024);
    let (backend_a_read, backend_a_write) = tokio::io::split(backend_a_bridge);

    let (backend_b_bridge, backend_b_remote) = tokio::io::duplex(64 * 1024);
    let (backend_b_read, backend_b_write) = tokio::io::split(backend_b_bridge);

    let (swap_tx, swap_rx) = mpsc::channel::<SwapCommand>(1);

    let bridge_handle = tokio::spawn(swappable_bridge(
        tcp_bridge_read,
        tcp_bridge_write,
        BackendHandle {
            reader: backend_a_read,
            writer: backend_a_write,
        },
        swap_rx,
    ));

    let echo_a = spawn_echo(backend_a_remote);
    let echo_b = spawn_echo(backend_b_remote);

    let total_messages: u32 = 5000;
    let swap_at: u32 = 2000;

    let (tcp_read, mut tcp_write) = tokio::io::split(tcp_client);

    let sender = tokio::spawn(async move {
        let mut pending_backend = Some(BackendHandle {
            reader: backend_b_read,
            writer: backend_b_write,
        });

        for seq in 0..total_messages {
            if seq == swap_at {
                if let Some(new_backend) = pending_backend.take() {
                    let (done_tx, done_rx) = oneshot::channel();
                    swap_tx
                        .send(SwapCommand {
                            new_backend,
                            done_tx,
                        })
                        .await
                        .unwrap();
                    done_rx.await.unwrap();
                }
            }

            let bytes = seq.to_be_bytes();
            if tcp_write.write_all(&bytes).await.is_err() {
                break;
            }

            if seq % 100 == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
        // 等 echo pipeline 排空再關閉 TCP
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(tcp_write);
    });

    let receiver = tokio::spawn(collect_sequences(tcp_read, total_messages as usize));

    sender.await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(3), bridge_handle).await;
    echo_a.abort();
    echo_b.abort();

    let received = receiver.await.unwrap();
    assert_sequences_complete(&received, total_messages);
    println!("✓ Premise 5 PoC 通過：{total_messages} 筆資料，swap @ #{swap_at}，零遺失、順序正確");
}

#[tokio::test]
async fn test_swap_under_heavy_load() {
    let (tcp_client, tcp_bridge) = tokio::io::duplex(64 * 1024);
    let (tcp_bridge_read, tcp_bridge_write) = tokio::io::split(tcp_bridge);

    let (ba_bridge, ba_remote) = tokio::io::duplex(64 * 1024);
    let (bb_bridge, bb_remote) = tokio::io::duplex(64 * 1024);
    let (bc_bridge, bc_remote) = tokio::io::duplex(64 * 1024);

    let (ba_read, ba_write) = tokio::io::split(ba_bridge);
    let (bb_read, bb_write) = tokio::io::split(bb_bridge);
    let (bc_read, bc_write) = tokio::io::split(bc_bridge);

    let (swap_tx, swap_rx) = mpsc::channel::<SwapCommand>(1);

    let bridge_handle = tokio::spawn(swappable_bridge(
        tcp_bridge_read,
        tcp_bridge_write,
        BackendHandle {
            reader: ba_read,
            writer: ba_write,
        },
        swap_rx,
    ));

    for remote in [ba_remote, bb_remote, bc_remote] {
        spawn_echo(remote);
    }

    let total: u32 = 20000;
    let swap_points = [5000u32, 12000];

    let (tcp_read, mut tcp_write) = tokio::io::split(tcp_client);

    let sender = tokio::spawn(async move {
        let backends = vec![
            BackendHandle {
                reader: bb_read,
                writer: bb_write,
            },
            BackendHandle {
                reader: bc_read,
                writer: bc_write,
            },
        ];
        let mut backend_iter = backends.into_iter();

        for seq in 0..total {
            if swap_points.contains(&seq) {
                if let Some(new_backend) = backend_iter.next() {
                    let (done_tx, done_rx) = oneshot::channel();
                    swap_tx
                        .send(SwapCommand {
                            new_backend,
                            done_tx,
                        })
                        .await
                        .unwrap();
                    done_rx.await.unwrap();
                }
            }
            let bytes = seq.to_be_bytes();
            if tcp_write.write_all(&bytes).await.is_err() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(tcp_write);
    });

    let receiver = tokio::spawn(collect_sequences(tcp_read, total as usize));

    sender.await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(3), bridge_handle).await;
    let received = receiver.await.unwrap();

    assert_sequences_complete(&received, total);
    println!("✓ 高負載 swap 測試通過：{total} 筆，swap 兩次，零遺失");
}
