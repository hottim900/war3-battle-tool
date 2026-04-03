//! Phase 0 Spike: 驗證透過 npcap loopback 發送 spoofed UDP 封包給 War3。
//!
//! 使用方式：
//!   cargo run -p spike-packet [SOURCE_IP]
//!
//! 預設 source IP: 192.168.1.100
//!
//! 前置條件：
//! 1. npcap 已安裝（勾選 loopback adapter 支援）
//! 2. npcap SDK 已下載，LIB 環境變數指向 SDK/Lib/x64
//! 3. 以管理員身分執行（npcap 需要提升權限）
//! 4. War3 已開啟並在 LAN 遊戲瀏覽畫面

use std::net::Ipv4Addr;
use war3_protocol::war3::{War3Version, WAR3_PORT};

fn main() -> anyhow::Result<()> {
    let src_ip: Ipv4Addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "192.168.1.100".into())
        .parse()?;

    let version = War3Version::V127;
    let payload = version.broadcast_packet();

    println!("=== War3 Packet Spike ===");
    println!("Source IP:   {src_ip}");
    println!("Dest IP:     127.0.0.1");
    println!("Port:        {WAR3_PORT}");
    println!("War3 Ver:    {version}");
    println!("Payload ({} bytes): {:02x?}", payload.len(), payload);
    println!();

    // 列出所有 npcap 裝置
    println!("可用 npcap 裝置：");
    let devices = pcap::Device::list()?;
    for dev in &devices {
        println!(
            "  {} - {}",
            dev.name,
            dev.desc.as_deref().unwrap_or("(無描述)")
        );
    }
    println!();

    // 尋找 loopback adapter
    let loopback = find_loopback(&devices)?;
    println!(
        "使用裝置: {} ({})",
        loopback.name,
        loopback.desc.as_deref().unwrap_or("")
    );

    // 建構封包
    let pkt = build_packet(src_ip, Ipv4Addr::LOCALHOST, WAR3_PORT, WAR3_PORT, payload);
    println!("封包 ({} bytes):", pkt.len());
    print_hex_dump(&pkt);
    println!();

    // 發送
    let repeat = 10;
    println!("發送 {repeat} 次...");
    let mut cap = pcap::Capture::from_device(loopback.clone())?
        .immediate_mode(true)
        .open()?;

    for i in 0..repeat {
        cap.sendpacket(pkt.as_slice())?;
        println!("  [{}/{}] 已送出", i + 1, repeat);
        if i < repeat - 1 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    println!();
    println!("完成！請檢查 War3 區域網路瀏覽畫面。");
    println!();
    println!("如果看不到遊戲房間，可能原因：");
    println!("  1. War3 未在 LAN 遊戲瀏覽畫面");
    println!("  2. broadcast_packet 格式不正確（可能需要完整的 game info）");
    println!("  3. War3 需要回應 query（需要雙向通訊）");
    println!("  4. 需要以管理員身分執行");

    Ok(())
}

fn find_loopback(devices: &[pcap::Device]) -> anyhow::Result<pcap::Device> {
    for dev in devices {
        if dev.name.contains("Loopback") || dev.name.contains("loopback") {
            return Ok(dev.clone());
        }
        if let Some(desc) = &dev.desc {
            if desc.contains("Loopback") || desc.contains("loopback") {
                return Ok(dev.clone());
            }
        }
    }
    anyhow::bail!(
        "找不到 npcap loopback adapter。\n\
         請確認 npcap 安裝時有勾選「Support loopback traffic」。"
    )
}

/// 建構 npcap loopback 原始封包：DLT_NULL(4) + IPv4(20) + UDP(8) + payload
fn build_packet(
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let udp_len = 8u16 + payload.len() as u16;
    let ip_total_len = 20u16 + udp_len;
    let mut pkt = Vec::with_capacity(4 + ip_total_len as usize);

    // DLT_NULL: AF_INET = 2, little-endian
    pkt.extend_from_slice(&2u32.to_le_bytes());

    // IPv4 header
    let ip_start = pkt.len();
    pkt.push(0x45);
    pkt.push(0x00);
    pkt.extend_from_slice(&ip_total_len.to_be_bytes());
    pkt.extend_from_slice(&[0x00, 0x00]); // identification
    pkt.extend_from_slice(&[0x40, 0x00]); // DF
    pkt.push(128); // TTL
    pkt.push(17); // UDP
    pkt.extend_from_slice(&[0x00, 0x00]); // checksum placeholder
    pkt.extend_from_slice(&src_ip.octets());
    pkt.extend_from_slice(&dst_ip.octets());

    let checksum = ip_checksum(&pkt[ip_start..ip_start + 20]);
    pkt[ip_start + 10] = (checksum >> 8) as u8;
    pkt[ip_start + 11] = (checksum & 0xff) as u8;

    // UDP header
    pkt.extend_from_slice(&src_port.to_be_bytes());
    pkt.extend_from_slice(&dst_port.to_be_bytes());
    pkt.extend_from_slice(&udp_len.to_be_bytes());
    pkt.extend_from_slice(&[0x00, 0x00]); // checksum optional

    // Payload
    pkt.extend_from_slice(payload);
    pkt
}

fn ip_checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    for chunk in data.chunks(2) {
        let word = if chunk.len() == 2 {
            u16::from_be_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_be_bytes([chunk[0], 0])
        };
        sum += word as u32;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn print_hex_dump(data: &[u8]) {
    for (i, chunk) in data.chunks(16).enumerate() {
        print!("  {:04x}: ", i * 16);
        for (j, byte) in chunk.iter().enumerate() {
            if j == 8 {
                print!(" ");
            }
            print!("{byte:02x} ");
        }
        println!();
    }
}
