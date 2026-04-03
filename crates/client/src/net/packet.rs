use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use war3_protocol::war3::{War3Version, WAR3_PORT};

/// 封包注入的抽象層
///
/// 正常 UDP 操作不需要 npcap，只有 spoofed source IP 才需要。
/// Windows 實作用 npcap，測試/mock 用 DummySender。
pub trait PacketSender: Send + Sync {
    fn send_spoofed_udp(
        &self,
        src_ip: Ipv4Addr,
        dst_ip: Ipv4Addr,
        src_port: u16,
        dst_port: u16,
        payload: &[u8],
    ) -> Result<()>;
}

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

/// 把房主的 W3GS_GAMEINFO 回覆轉發到本地 War3（需要 spoofed source IP）
///
/// 這讓本地 War3 以為房主在 LAN 上，在區域網路畫面顯示房間。
pub fn redirect_to_local(
    sender: &dyn PacketSender,
    server_ip: Ipv4Addr,
    local_ip: Ipv4Addr,
    game_info_response: &[u8],
) -> Result<()> {
    // 送多次增加成功率（War3 可能在忙）
    for _ in 0..5 {
        sender.send_spoofed_udp(
            server_ip,
            local_ip,
            WAR3_PORT,
            WAR3_PORT,
            game_info_response,
        )?;
    }
    Ok(())
}

/// 玩家加入房間的完整流程
///
/// 1. 用正常 UDP 問房主拿 game info
/// 2. 用 spoofed packet 送到本地 loopback
pub fn join_room(
    sender: &dyn PacketSender,
    host_ip: Ipv4Addr,
    local_ip: Ipv4Addr,
    version: War3Version,
) -> Result<()> {
    let response = check_room(host_ip, version)?;
    redirect_to_local(sender, host_ip, local_ip, &response)?;
    Ok(())
}

/// 房主邀請玩家（模擬遠端玩家送 broadcast 到本機）
///
/// 讓本機 War3 server 回應，遠端玩家就能在 check_room 收到 game info。
pub fn invite_player(
    sender: &dyn PacketSender,
    player_ip: Ipv4Addr,
    local_ip: Ipv4Addr,
    version: War3Version,
) -> Result<()> {
    let broadcast_data = version.broadcast_packet();
    for _ in 0..5 {
        sender.send_spoofed_udp(
            player_ip,
            local_ip,
            WAR3_PORT,
            WAR3_PORT,
            broadcast_data,
        )?;
    }
    Ok(())
}

/// 測試用的 dummy sender（不真的送封包）
#[cfg(test)]
pub struct DummySender;

#[cfg(test)]
impl PacketSender for DummySender {
    fn send_spoofed_udp(
        &self,
        _src_ip: Ipv4Addr,
        _dst_ip: Ipv4Addr,
        _src_port: u16,
        _dst_port: u16,
        _payload: &[u8],
    ) -> Result<()> {
        Ok(())
    }
}
