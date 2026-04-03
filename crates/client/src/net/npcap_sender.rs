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
        device_name: String,
    }

    impl NpcapSender {
        /// 建立 NpcapSender，選擇指定的網路介面
        /// 如果 device_name 為 None，使用預設介面
        pub fn new(device_name: Option<&str>) -> Result<Self> {
            let name = match device_name {
                Some(n) => n.to_string(),
                None => {
                    let dev = Device::lookup()
                        .context("無法查詢網路介面")?
                        .context("找不到可用的網路介面")?;
                    dev.name
                }
            };
            Ok(Self { device_name: name })
        }

        /// 構建帶有 spoofed source IP 的 UDP 封包（raw bytes）
        fn build_udp_packet(
            src_ip: Ipv4Addr,
            dst_ip: Ipv4Addr,
            src_port: u16,
            dst_port: u16,
            payload: &[u8],
        ) -> Vec<u8> {
            let udp_len = 8 + payload.len();
            let total_len = 20 + udp_len;

            let mut pkt = Vec::with_capacity(total_len);

            // IP header (20 bytes)
            pkt.push(0x45); // version=4, IHL=5
            pkt.push(0x00); // DSCP/ECN
            pkt.extend_from_slice(&(total_len as u16).to_be_bytes()); // total length
            pkt.extend_from_slice(&[0x00, 0x00]); // identification
            pkt.extend_from_slice(&[0x40, 0x00]); // flags=DF, fragment offset=0
            pkt.push(64); // TTL
            pkt.push(17); // protocol = UDP
            pkt.extend_from_slice(&[0x00, 0x00]); // checksum (0 = let OS calculate)
            pkt.extend_from_slice(&src_ip.octets());
            pkt.extend_from_slice(&dst_ip.octets());

            // UDP header (8 bytes)
            pkt.extend_from_slice(&src_port.to_be_bytes());
            pkt.extend_from_slice(&dst_port.to_be_bytes());
            pkt.extend_from_slice(&(udp_len as u16).to_be_bytes());
            pkt.extend_from_slice(&[0x00, 0x00]); // checksum (optional for UDP)

            // Payload
            pkt.extend_from_slice(payload);

            // Calculate IP header checksum
            let checksum = ip_checksum(&pkt[..20]);
            pkt[10] = (checksum >> 8) as u8;
            pkt[11] = (checksum & 0xFF) as u8;

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
            let raw_packet = Self::build_udp_packet(src_ip, dst_ip, src_port, dst_port, payload);

            let mut cap = Capture::from_device(self.device_name.as_str())
                .context("無法開啟網路介面")?
                .open()
                .context("無法啟動擷取")?;

            cap.sendpacket(&raw_packet)
                .context("封包發送失敗")?;

            Ok(())
        }
    }
}

#[cfg(windows)]
pub use platform::NpcapSender;
