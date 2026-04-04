use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::sync::Semaphore;
use war3_protocol::war3::{WAR3_PORT, War3Version};

/// 檢查遠端房主是否有開房（正常 UDP，不需要 npcap）
///
/// 送 W3GS_SEARCHGAME broadcast 到目標 IP，收 W3GS_GAMEINFO response。
pub fn check_room(host_ip: Ipv4Addr, version: War3Version) -> Result<Vec<u8>> {
    let sock = UdpSocket::bind("0.0.0.0:0").context("無法綁定 UDP socket")?;
    sock.set_read_timeout(Some(Duration::from_millis(500)))?;

    let broadcast_data = version.broadcast_packet();
    let target = SocketAddr::new(IpAddr::V4(host_ip), WAR3_PORT);

    // 送多次增加成功率
    for _ in 0..3 {
        sock.send_to(broadcast_data, target)?;
    }

    let mut buf = [0u8; 1024];
    match sock.recv_from(&mut buf) {
        Ok((len, _)) if len > 16 => Ok(buf[..len].to_vec()),
        Ok((len, _)) => bail!("收到的回覆太短 ({len} bytes)"),
        Err(e) => bail!("沒有收到房間回覆: {e}"),
    }
}

/// 掃描 /24 子網路，找出有開房的 War3 主機（正常 UDP，不需要 npcap）
#[allow(dead_code)]
pub async fn scan_rooms(subnet: &str, version: War3Version) -> Result<Vec<(Ipv4Addr, Vec<u8>)>> {
    let base_ip = parse_subnet_base(subnet)?;
    let octets = base_ip.octets();

    let semaphore = Arc::new(Semaphore::new(20));
    let mut handles = Vec::with_capacity(254);

    for i in 1..=254u8 {
        let ip = Ipv4Addr::new(octets[0], octets[1], octets[2], i);
        let permit = Arc::clone(&semaphore);

        handles.push(tokio::spawn(async move {
            let _permit = permit.acquire().await;
            match check_room(ip, version) {
                Ok(data) => Some((ip, data)),
                Err(_) => None,
            }
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(Some(entry)) = handle.await {
            results.push(entry);
        }
    }

    Ok(results)
}

/// 解析 "/24" 子網路字串，取得基底 IP
fn parse_subnet_base(subnet: &str) -> Result<Ipv4Addr> {
    let ip_str = subnet.split('/').next().unwrap_or(subnet);
    let ip: Ipv4Addr = ip_str
        .parse()
        .with_context(|| format!("無效的子網路位址: {subnet}"))?;
    Ok(ip)
}

/// 從 127.0.0.2 發送 GAMEINFO 到本地 War3（不需要 npcap）
///
/// War3 看到 UDP source IP = 127.0.0.2 → 點 Join 時 TCP 連到 127.0.0.2:6112
/// 我們的 TCP proxy 在那裡攔截。
pub struct RawUdpInjector {
    socket: UdpSocket,
}

impl RawUdpInjector {
    /// 建立 injector，bind 到 127.0.0.2:0
    pub fn new() -> Result<Self> {
        let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)), 0);
        let socket = UdpSocket::bind(bind_addr)
            .with_context(|| format!("無法綁定 {bind_addr}，請確認 127.0.0.2 loopback 可用"))?;
        Ok(Self { socket })
    }

    /// 注入 GAMEINFO 到本地 War3（127.0.0.1:6112）
    pub fn inject(&self, gameinfo: &[u8]) -> Result<()> {
        let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), WAR3_PORT);
        self.socket
            .send_to(gameinfo, target)
            .context("GAMEINFO 注入失敗")?;
        Ok(())
    }
}
