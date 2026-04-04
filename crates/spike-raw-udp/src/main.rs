use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use war3_protocol::war3::{WAR3_PORT, War3Version};

const GAMEINFO_FILE: &str = "gameinfo.bin";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match cmd {
        "capture" => capture()?,
        "inject" => inject(args.get(2).map(|s| s.as_str()))?,
        _ => print_usage(),
    }
    Ok(())
}

fn print_usage() {
    eprintln!("Raw UDP 實驗工具");
    eprintln!();
    eprintln!("用法:");
    eprintln!("  spike-raw-udp capture           Phase 1: 捕捉 War3 GAMEINFO（需要先在 War3 開房）");
    eprintln!("  spike-raw-udp inject             Phase 2: 注入 GAMEINFO 到 War3（需要在 LAN Games 畫面）");
    eprintln!("  spike-raw-udp inject 1.2.3.4     Phase 2: 注入時指定來源 IP（測試 IP 替換）");
}

/// Phase 1: 送 SEARCHGAME 到 127.0.0.1:6112，捕捉 GAMEINFO 回覆
fn capture() -> Result<()> {
    println!("=== Phase 1: 捕捉 GAMEINFO ===");
    println!("前提：War3 已開啟並建立一個房間（Create Game）");
    println!();

    let sock = UdpSocket::bind("0.0.0.0:0").context("無法綁定 UDP socket")?;
    sock.set_read_timeout(Some(Duration::from_secs(2)))?;

    let broadcast = War3Version::V127.broadcast_packet();
    let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), WAR3_PORT);

    println!("送出 SEARCHGAME 到 127.0.0.1:{WAR3_PORT}...");
    for i in 0..3 {
        sock.send_to(broadcast, target)?;
        if i < 2 {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    println!("等待 GAMEINFO 回覆...");
    let mut buf = [0u8; 2048];
    match sock.recv_from(&mut buf) {
        Ok((len, addr)) => {
            println!("收到 {len} bytes 從 {addr}");
            println!();
            hex_dump(&buf[..len]);
            println!();

            fs::write(GAMEINFO_FILE, &buf[..len])
                .context("寫入 gameinfo.bin 失敗")?;
            println!("已儲存到 {GAMEINFO_FILE} ({len} bytes)");
            println!();
            println!("下一步：關閉房間 → 回到 LAN Games 畫面 → 執行 `spike-raw-udp inject`");
        }
        Err(e) => {
            bail!("沒有收到回覆: {e}\n\n確認 War3 有開房且在 LAN Games 畫面");
        }
    }

    Ok(())
}

/// Phase 2: 讀取 GAMEINFO，用標準 UDP 送到 127.0.0.1:6112
fn inject(fake_ip: Option<&str>) -> Result<()> {
    println!("=== Phase 2: 注入 GAMEINFO ===");
    println!("前提：War3 在 LAN Games 畫面（不是開房狀態）");
    println!();

    let mut gameinfo = fs::read(GAMEINFO_FILE)
        .with_context(|| format!("讀取 {GAMEINFO_FILE} 失敗，請先執行 capture"))?;

    println!("讀取 {GAMEINFO_FILE} ({} bytes)", gameinfo.len());

    // 如果指定了 fake IP，嘗試替換 GAMEINFO 中內嵌的 IP
    if let Some(ip_str) = fake_ip {
        let ip: Ipv4Addr = ip_str.parse().context("無效的 IP 位址")?;
        println!("嘗試在 GAMEINFO 中尋找並替換內嵌 IP 為 {ip}...");
        replace_embedded_ip(&mut gameinfo, ip);
    }

    let sock = UdpSocket::bind("0.0.0.0:0").context("無法綁定 UDP socket")?;
    let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), WAR3_PORT);

    // 測試不同發送模式
    let patterns: &[(&str, usize, u64)] = &[
        ("單次發送", 1, 0),
        ("3 次, 間隔 50ms", 3, 50),
        ("5 次, 間隔 100ms", 5, 100),
    ];

    for (desc, count, interval_ms) in patterns {
        println!();
        println!("--- {desc} ---");
        println!("送出 GAMEINFO 到 127.0.0.1:{WAR3_PORT}...");

        for i in 0..*count {
            sock.send_to(&gameinfo, target)?;
            if i + 1 < *count && *interval_ms > 0 {
                std::thread::sleep(Duration::from_millis(*interval_ms));
            }
        }

        println!("已送出。檢查 War3 LAN Games 畫面是否出現房間。");
        println!("按 Enter 繼續下一個測試模式...");

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
    }

    println!();
    println!("=== 測試結束 ===");
    println!("結果判定：");
    println!("  A) War3 顯示房間，host IP 是 GAMEINFO 內嵌的 IP → Raw UDP 完全可行");
    println!("  B) War3 顯示房間，host IP 是 127.0.0.1 → 部分可行，需進一步測試");
    println!("  C) War3 沒顯示任何房間 → Raw UDP 不可行");

    Ok(())
}

/// 嘗試在 GAMEINFO 封包中替換內嵌的 IP
///
/// W3GS GAMEINFO 格式中 IP 位址通常以 little-endian 出現在 sockaddr_in 結構中。
/// 我們搜尋 127.0.0.1 (01 00 00 7f) 的各種表示並替換。
fn replace_embedded_ip(data: &mut [u8], new_ip: Ipv4Addr) {
    let new_octets = new_ip.octets();

    // 搜尋 127.0.0.1 的 network byte order (big-endian): 7f 00 00 01
    let localhost_be = [127, 0, 0, 1];
    let mut replaced = false;

    for i in 0..data.len().saturating_sub(3) {
        if data[i..i + 4] == localhost_be {
            println!("  找到 127.0.0.1 (big-endian) 在 offset {i}，替換為 {new_ip}");
            data[i..i + 4].copy_from_slice(&new_octets);
            replaced = true;
        }
    }

    // 也搜尋本機其他常見 IP（可能是 LAN IP）
    // 先印出所有看起來像 IP 的 4-byte pattern
    if !replaced {
        println!("  未找到 127.0.0.1，列出封包中所有可能的 IP 位址：");
        // sockaddr_in 結構: family(2) + port(2) + ip(4)
        // 在 W3GS 中通常前面有 02 00 (AF_INET) 和 port
        for i in 0..data.len().saturating_sub(7) {
            if data[i] == 0x02 && data[i + 1] == 0x00 {
                // 可能是 sockaddr_in，port 在 [i+2..i+4]，IP 在 [i+4..i+8]
                let port = u16::from_be_bytes([data[i + 2], data[i + 3]]);
                if i + 8 <= data.len() {
                    let ip = Ipv4Addr::new(data[i + 4], data[i + 5], data[i + 6], data[i + 7]);
                    if !ip.is_unspecified() {
                        println!("    offset {i}: sockaddr_in {{ port: {port}, ip: {ip} }}");
                    }
                }
            }
        }
    }
}

fn hex_dump(data: &[u8]) {
    for (i, chunk) in data.chunks(16).enumerate() {
        print!("{:04x}  ", i * 16);
        for (j, byte) in chunk.iter().enumerate() {
            print!("{:02x} ", byte);
            if j == 7 {
                print!(" ");
            }
        }
        // 補齊空格
        for j in chunk.len()..16 {
            print!("   ");
            if j == 7 {
                print!(" ");
            }
        }
        print!(" |");
        for byte in chunk {
            if byte.is_ascii_graphic() || *byte == b' ' {
                print!("{}", *byte as char);
            } else {
                print!(".");
            }
        }
        println!("|");
    }
}
