use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use war3_protocol::war3::{WAR3_PORT, War3Version};

const GAMEINFO_FILE: &str = "gameinfo.bin";
const PROXY_IP: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match cmd {
        "capture" => capture(args.get(2).map(|s| s.as_str()))?,
        "inject" => inject()?,
        "proxy" => proxy(args.get(2).map(|s| s.as_str()))?,
        "join" => join(args.get(2).map(|s| s.as_str()))?,
        _ => print_usage(),
    }
    Ok(())
}

fn print_usage() {
    eprintln!("Raw UDP 實驗工具");
    eprintln!();
    eprintln!("用法:");
    eprintln!("  spike-raw-udp capture [IP]          捕捉 GAMEINFO（預設 127.0.0.1）");
    eprintln!("  spike-raw-udp inject                從 127.0.0.2 注入 GAMEINFO");
    eprintln!("  spike-raw-udp proxy <host_ip>       TCP+UDP proxy 127.0.0.2:6112 → host_ip:6112");
    eprintln!("  spike-raw-udp join <host_ip>        一鍵測試：capture + proxy + inject");
}

/// 送 SEARCHGAME 到目標 IP:6112，捕捉 GAMEINFO 回覆
fn capture(target_ip: Option<&str>) -> Result<()> {
    let ip: Ipv4Addr = match target_ip {
        Some(s) => s.parse().context("無效的 IP 位址")?,
        None => Ipv4Addr::LOCALHOST,
    };

    println!("[capture] 目標：{ip}:{WAR3_PORT}");

    let sock = UdpSocket::bind("0.0.0.0:0").context("無法綁定 UDP socket")?;
    sock.set_read_timeout(Some(Duration::from_secs(2)))?;

    let broadcast = War3Version::V127.broadcast_packet();
    let target = SocketAddr::new(IpAddr::V4(ip), WAR3_PORT);

    for i in 0..3 {
        sock.send_to(broadcast, target)?;
        if i < 2 {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    let mut buf = [0u8; 2048];
    match sock.recv_from(&mut buf) {
        Ok((len, addr)) => {
            println!("[capture] 收到 {len} bytes 從 {addr}");
            hex_dump(&buf[..len]);
            fs::write(GAMEINFO_FILE, &buf[..len]).context("寫入 gameinfo.bin 失敗")?;
            println!("[capture] 已儲存到 {GAMEINFO_FILE}");
        }
        Err(e) => {
            bail!("沒有收到回覆: {e}\n確認目標 {ip} 有開房且防火牆沒擋 UDP {WAR3_PORT}");
        }
    }
    Ok(())
}

/// 從 127.0.0.2 發送 GAMEINFO 到 127.0.0.1:6112
fn inject() -> Result<()> {
    let gameinfo = fs::read(GAMEINFO_FILE)
        .with_context(|| format!("讀取 {GAMEINFO_FILE} 失敗，請先執行 capture"))?;

    println!(
        "[inject] 從 {PROXY_IP} 送出 GAMEINFO ({} bytes) 到 127.0.0.1:{WAR3_PORT}",
        gameinfo.len()
    );

    // bind 到 127.0.0.2 讓 War3 看到 source IP = 127.0.0.2
    let sock = UdpSocket::bind(SocketAddr::new(IpAddr::V4(PROXY_IP), 0))
        .context("無法 bind 到 127.0.0.2（Windows 應支援整個 127.0.0.0/8）")?;

    let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), WAR3_PORT);

    for _ in 0..5 {
        sock.send_to(&gameinfo, target)?;
        thread::sleep(Duration::from_millis(100));
    }

    println!("[inject] 已送出 5 次。檢查 War3 LAN Games 畫面。");
    Ok(())
}

/// TCP + UDP proxy: 127.0.0.2:6112 → host_ip:6112
fn proxy(host_ip: Option<&str>) -> Result<()> {
    let host: Ipv4Addr = host_ip
        .ok_or_else(|| anyhow::anyhow!("用法: spike-raw-udp proxy <host_ip>"))?
        .parse()
        .context("無效的 IP 位址")?;

    let proxy_addr = SocketAddr::new(IpAddr::V4(PROXY_IP), WAR3_PORT);
    let host_addr = SocketAddr::new(IpAddr::V4(host), WAR3_PORT);

    // 啟動 UDP proxy (背景)
    let udp_host = host;
    thread::spawn(move || {
        if let Err(e) = run_udp_proxy(udp_host) {
            eprintln!("[udp-proxy] 錯誤: {e}");
        }
    });

    // TCP proxy (前景)
    let listener =
        TcpListener::bind(proxy_addr).with_context(|| format!("無法 bind TCP {proxy_addr}"))?;

    println!("[proxy] TCP+UDP proxy 啟動: {proxy_addr} → {host_addr}");
    println!("[proxy] 等待 War3 連線...");

    for stream in listener.incoming() {
        let stream = stream.context("accept 失敗")?;
        let peer = stream.peer_addr().ok();
        println!("[proxy] TCP 連線來自 {peer:?}，轉發到 {host_addr}");

        let host_addr_clone = host_addr;
        thread::spawn(move || {
            if let Err(e) = relay_tcp(stream, host_addr_clone) {
                eprintln!("[proxy] TCP relay 結束: {e}");
            }
        });
    }

    Ok(())
}

/// 一鍵測試：capture → proxy(背景) → inject
fn join(host_ip: Option<&str>) -> Result<()> {
    let host: Ipv4Addr = host_ip
        .ok_or_else(|| anyhow::anyhow!("用法: spike-raw-udp join <host_ip>"))?
        .parse()
        .context("無效的 IP 位址")?;

    // 1. 捕捉 GAMEINFO
    println!("=== Step 1: 從 {host} 捕捉 GAMEINFO ===");
    capture(Some(&host.to_string()))?;

    // 2. 啟動 proxy (背景)
    println!();
    println!("=== Step 2: 啟動 TCP+UDP proxy ({PROXY_IP}:6112 → {host}:6112) ===");
    let proxy_host = host;
    thread::spawn(move || {
        if let Err(e) = proxy(Some(&proxy_host.to_string())) {
            eprintln!("[proxy] 錯誤: {e}");
        }
    });

    // 等 proxy 啟動
    thread::sleep(Duration::from_millis(200));

    // 3. 注入
    println!();
    println!("=== Step 3: 注入 GAMEINFO ===");
    inject()?;

    println!();
    println!("=== 準備完成 ===");
    println!("War3 應該會顯示一個房間。點 Join 試試看。");
    println!("Proxy 持續運行中... 按 Ctrl+C 結束。");

    // 持續注入（War3 的 LAN 列表會定期清除過期房間）
    loop {
        thread::sleep(Duration::from_secs(3));
        if let Ok(gameinfo) = fs::read(GAMEINFO_FILE) {
            let sock = UdpSocket::bind(SocketAddr::new(IpAddr::V4(PROXY_IP), 0)).ok();
            if let Some(sock) = sock {
                let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), WAR3_PORT);
                let _ = sock.send_to(&gameinfo, target);
            }
        }
    }
}

/// UDP proxy: 轉發 127.0.0.2:6112 ↔ host:6112
fn run_udp_proxy(host: Ipv4Addr) -> Result<()> {
    let proxy_addr = SocketAddr::new(IpAddr::V4(PROXY_IP), WAR3_PORT);
    let sock =
        UdpSocket::bind(proxy_addr).with_context(|| format!("無法 bind UDP {proxy_addr}"))?;
    sock.set_read_timeout(Some(Duration::from_secs(30)))?;

    let host_addr = SocketAddr::new(IpAddr::V4(host), WAR3_PORT);
    println!("[udp-proxy] 監聽 {proxy_addr}，轉發到 {host_addr}");

    let mut buf = [0u8; 4096];
    loop {
        match sock.recv_from(&mut buf) {
            Ok((len, src)) => {
                println!("[udp-proxy] 收到 {len} bytes 從 {src}");
                if src.ip() != IpAddr::V4(host) {
                    // 來自本機 War3 → 轉發到 host
                    let _ = sock.send_to(&buf[..len], host_addr);
                    println!("[udp-proxy] → 轉發到 {host_addr}");
                } else {
                    // 來自 host → 轉發到本機 War3
                    let _ = sock.send_to(&buf[..len], src);
                    println!("[udp-proxy] ← 轉發到 {src}");
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                continue;
            }
            Err(e) => {
                bail!("UDP recv 錯誤: {e}");
            }
        }
    }
}

/// TCP relay: 雙向轉發
fn relay_tcp(mut client: TcpStream, host_addr: SocketAddr) -> Result<()> {
    let mut server = TcpStream::connect_timeout(&host_addr, Duration::from_secs(5))
        .with_context(|| format!("無法連線到 {host_addr}"))?;

    client.set_nodelay(true)?;
    server.set_nodelay(true)?;

    let mut client_clone = client.try_clone().context("clone client 失敗")?;
    let mut server_clone = server.try_clone().context("clone server 失敗")?;

    // client → server
    let c2s = thread::spawn(move || -> Result<()> {
        let mut buf = [0u8; 8192];
        loop {
            let n = client.read(&mut buf)?;
            if n == 0 {
                break;
            }
            server.write_all(&buf[..n])?;
        }
        let _ = server.shutdown(std::net::Shutdown::Write);
        Ok(())
    });

    // server → client
    let s2c = thread::spawn(move || -> Result<()> {
        let mut buf = [0u8; 8192];
        loop {
            let n = server_clone.read(&mut buf)?;
            if n == 0 {
                break;
            }
            client_clone.write_all(&buf[..n])?;
        }
        let _ = client_clone.shutdown(std::net::Shutdown::Write);
        Ok(())
    });

    let _ = c2s.join();
    let _ = s2c.join();
    println!("[proxy] TCP relay 結束");
    Ok(())
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
