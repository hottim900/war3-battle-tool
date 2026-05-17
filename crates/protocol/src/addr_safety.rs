//! 外部位址安全檢查：拒絕 RFC1918 私有 / loopback / link-local / CGNAT 等
//! 不應作為對外公開 endpoint 的位址。
//!
//! 同時被 server 端（驗證 `UPnPMapped` 內容）與 client 端（驗證
//! `PeerUPnPAddr` 來源）使用——任何一邊放鬆規則都會破壞 SSRF 防線，
//! 故規則由本 crate 提供單一 source of truth。

use std::net::IpAddr;

/// 是否為「可作為公開連線目標」的位址。
///
/// 拒絕：
/// - IPv4: loopback / RFC1918 / link-local / broadcast / unspecified / CGNAT (100.64/10)
/// - IPv6: loopback / unspecified / ULA (fc00::/7) / link-local (fe80::/10)
pub fn is_safe_external_addr(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
            {
                return false;
            }
            // RFC 6598 CGNAT shared address space (100.64.0.0/10)
            let octets = v4.octets();
            !(octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        IpAddr::V6(v6) => {
            !v6.is_loopback()
                && !v6.is_unspecified()
                // ULA (fc00::/7) 和 link-local (fe80::/10)
                && !matches!(v6.segments()[0], 0xfc00..=0xfdff | 0xfe80..=0xfebf)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── IPv4 ──

    #[test]
    fn rejects_ipv4_loopback() {
        assert!(!is_safe_external_addr("127.0.0.1".parse().unwrap()));
        assert!(!is_safe_external_addr("127.255.255.255".parse().unwrap()));
    }

    #[test]
    fn rejects_ipv4_private() {
        // 10.0.0.0/8
        assert!(!is_safe_external_addr("10.0.0.1".parse().unwrap()));
        assert!(!is_safe_external_addr("10.255.255.255".parse().unwrap()));
        // 172.16.0.0/12
        assert!(!is_safe_external_addr("172.16.0.1".parse().unwrap()));
        assert!(!is_safe_external_addr("172.31.255.255".parse().unwrap()));
        // 192.168.0.0/16
        assert!(!is_safe_external_addr("192.168.0.1".parse().unwrap()));
        assert!(!is_safe_external_addr("192.168.255.255".parse().unwrap()));
    }

    #[test]
    fn rejects_ipv4_link_local() {
        assert!(!is_safe_external_addr("169.254.0.1".parse().unwrap()));
        assert!(!is_safe_external_addr("169.254.255.255".parse().unwrap()));
    }

    #[test]
    fn rejects_ipv4_broadcast() {
        assert!(!is_safe_external_addr("255.255.255.255".parse().unwrap()));
    }

    #[test]
    fn rejects_ipv4_unspecified() {
        assert!(!is_safe_external_addr("0.0.0.0".parse().unwrap()));
    }

    #[test]
    fn rejects_ipv4_cgnat() {
        // RFC 6598: 100.64.0.0/10
        assert!(!is_safe_external_addr("100.64.0.1".parse().unwrap()));
        assert!(!is_safe_external_addr("100.64.1.1".parse().unwrap()));
        assert!(!is_safe_external_addr("100.127.255.255".parse().unwrap()));
    }

    #[test]
    fn accepts_ipv4_public() {
        assert!(is_safe_external_addr("8.8.8.8".parse().unwrap()));
        assert!(is_safe_external_addr("1.1.1.1".parse().unwrap()));
        assert!(is_safe_external_addr("203.0.113.1".parse().unwrap()));
    }

    // ── IPv6 ──

    #[test]
    fn rejects_ipv6_loopback() {
        assert!(!is_safe_external_addr("::1".parse().unwrap()));
    }

    #[test]
    fn rejects_ipv6_unspecified() {
        assert!(!is_safe_external_addr("::".parse().unwrap()));
    }

    #[test]
    fn rejects_ipv6_ula() {
        // fc00::/7 (fc00:: – fdff::)
        assert!(!is_safe_external_addr("fc00::1".parse().unwrap()));
        assert!(!is_safe_external_addr("fd12:3456:789a::1".parse().unwrap()));
        assert!(!is_safe_external_addr("fdff::1".parse().unwrap()));
    }

    #[test]
    fn rejects_ipv6_link_local() {
        // fe80::/10
        assert!(!is_safe_external_addr("fe80::1".parse().unwrap()));
        assert!(!is_safe_external_addr("febf::1".parse().unwrap()));
    }

    #[test]
    fn accepts_ipv6_public() {
        assert!(is_safe_external_addr("2001:db8::1".parse().unwrap()));
        assert!(is_safe_external_addr(
            "2607:f8b0:4004:800::200e".parse().unwrap()
        ));
    }
}
