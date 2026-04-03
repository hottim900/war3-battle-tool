/// NpcapSender — Windows-only raw packet injection via npcap/pcap crate
///
/// This module is only compiled on Windows. On other platforms,
/// packet_sender in app.rs is None, and the UI shows a warning.
#[cfg(windows)]
mod platform {
    use std::net::Ipv4Addr;

    use anyhow::{bail, Context, Result};
    use pcap::{Capture, Device};

    use super::super::packet::PacketSender;

    pub struct NpcapSender {
        /// 一般網卡（用於送到外部 IP，未來擴充用）
        #[allow(dead_code)]
        device_name: String,
        /// Npcap loopback adapter（用於送到 127.x.x.x）
        loopback_name: Option<String>,
    }

    impl NpcapSender {
        /// 建立 NpcapSender，自動偵測 loopback adapter
        pub fn new(device_name: Option<&str>) -> Result<Self> {
            let devices = Device::list().context("無法列舉網路介面")?;

            let name = match device_name {
                Some(n) => n.to_string(),
                None => {
                    let dev = Device::lookup()
                        .context("無法查詢網路介面")?
                        .context("找不到可用的網路介面")?;
                    dev.name
                }
            };

            // 尋找 loopback adapter
            let loopback_name = devices.iter().find_map(|dev| {
                if dev.name.contains("Loopback") || dev.name.contains("loopback") {
                    return Some(dev.name.clone());
                }
                if let Some(desc) = &dev.desc {
                    if desc.contains("Loopback") || desc.contains("loopback") {
                        return Some(dev.name.clone());
                    }
                }
                None
            });

            if loopback_name.is_some() {
                tracing::info!("NpcapSender: 找到 loopback adapter");
            } else {
                tracing::warn!("NpcapSender: 找不到 loopback adapter，localhost 注入可能失敗");
            }

            Ok(Self {
                device_name: name,
                loopback_name,
            })
        }

        /// 構建 loopback 封包：DLT_NULL(4) + IPv4(20) + UDP(8) + payload
        fn build_loopback_packet(
            src_ip: Ipv4Addr,
            dst_ip: Ipv4Addr,
            src_port: u16,
            dst_port: u16,
            payload: &[u8],
        ) -> Vec<u8> {
            let udp_len = 8 + payload.len();
            let ip_total_len = 20 + udp_len;

            let mut pkt = Vec::with_capacity(4 + ip_total_len);

            // DLT_NULL header: AF_INET = 2, little-endian
            pkt.extend_from_slice(&2u32.to_le_bytes());

            // IP header (20 bytes)
            let ip_start = pkt.len();
            pkt.push(0x45); // version=4, IHL=5
            pkt.push(0x00); // DSCP/ECN
            pkt.extend_from_slice(&(ip_total_len as u16).to_be_bytes());
            pkt.extend_from_slice(&[0x00, 0x00]); // identification
            pkt.extend_from_slice(&[0x40, 0x00]); // flags=DF
            pkt.push(128); // TTL
            pkt.push(17); // protocol = UDP
            pkt.extend_from_slice(&[0x00, 0x00]); // checksum placeholder
            pkt.extend_from_slice(&src_ip.octets());
            pkt.extend_from_slice(&dst_ip.octets());

            let checksum = ip_checksum(&pkt[ip_start..ip_start + 20]);
            pkt[ip_start + 10] = (checksum >> 8) as u8;
            pkt[ip_start + 11] = (checksum & 0xFF) as u8;

            // UDP header (8 bytes)
            pkt.extend_from_slice(&src_port.to_be_bytes());
            pkt.extend_from_slice(&dst_port.to_be_bytes());
            pkt.extend_from_slice(&(udp_len as u16).to_be_bytes());
            pkt.extend_from_slice(&[0x00, 0x00]); // checksum optional

            // Payload
            pkt.extend_from_slice(payload);
            pkt
        }
    }

    fn ip_checksum(header: &[u8]) -> u16 {
        let mut sum: u32 = 0;
        for i in (0..header.len()).step_by(2) {
            let word = if i + 1 < header.len() {
                ((header[i] as u32) << 8) | (header[i + 1] as u32)
            } else {
                (header[i] as u32) << 8
            };
            sum += word;
        }
        while sum >> 16 != 0 {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }
        !sum as u16
    }

    impl PacketSender for NpcapSender {
        fn send_spoofed_udp(
            &self,
            src_ip: Ipv4Addr,
            dst_ip: Ipv4Addr,
            src_port: u16,
            dst_port: u16,
            payload: &[u8],
        ) -> Result<()> {
            // localhost 目標用 loopback adapter + DLT_NULL header
            let is_loopback = dst_ip.is_loopback();

            let device_name = if is_loopback {
                self.loopback_name
                    .as_deref()
                    .context("找不到 npcap loopback adapter，請確認安裝時有勾選 loopback 支援")?
            } else {
                bail!("目前只支援 loopback 注入（dst={dst_ip}）");
            };

            let raw_packet =
                Self::build_loopback_packet(src_ip, dst_ip, src_port, dst_port, payload);

            let mut cap = Capture::from_device(device_name)
                .context("無法開啟 loopback 介面")?
                .immediate_mode(true)
                .open()
                .context("無法啟動擷取")?;

            cap.sendpacket(raw_packet)
                .context("封包發送失敗")?;

            Ok(())
        }
    }
}

#[cfg(windows)]
pub use platform::NpcapSender;
