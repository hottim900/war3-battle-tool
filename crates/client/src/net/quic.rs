//! QUIC P2P 直連模組
//!
//! Host 監聽 QUIC endpoint，Joiner 連線。
//! 雙方使用 tunnel_token 做 ALPN 驗證。
//! 失敗時靜默回傳 Err，caller fallback 到 WS relay。

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use tracing::{info, warn};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// QUIC 偏好 port（host 端，碰撞時 fallback ephemeral）
const QUIC_PREFERRED_PORT: u16 = 19870;
/// Joiner 直連預設 port（StunInfo path，不知道 host 實際 port 時使用）
pub const QUIC_DEFAULT_PORT: u16 = 19870;
/// UPnP gateway 搜尋 timeout
const UPNP_SEARCH_TIMEOUT: Duration = Duration::from_secs(2);
/// UPnP port mapping timeout
const UPNP_MAP_TIMEOUT: Duration = Duration::from_secs(1);
/// UPnP lease 時間（秒）
const UPNP_LEASE_SECS: u32 = 7200;
/// UPnP mapping 的 port（host 對外公開）
const UPNP_EXTERNAL_PORT: u16 = 19870;

// ── Strategy 型別 ──

/// 連線策略類型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StrategyKind {
    QuicDirect,
    UPnP,
}

impl fmt::Display for StrategyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StrategyKind::QuicDirect => write!(f, "QUIC 穿透"),
            StrategyKind::UPnP => write!(f, "UPnP 映射"),
        }
    }
}

/// 策略失敗原因
#[derive(Debug, Clone)]
pub enum StrategyFailReason {
    // QUIC 相關
    NoStunInfo,
    BindFailed(String),
    #[allow(dead_code)]
    Timeout,
    HandshakeFailed(String),
    // UPnP 相關
    UPnPGatewayNotFound,
    UPnPMappingFailed(String),
    UPnPNotAttempted,
    CgnatDetected,
}

impl fmt::Display for StrategyFailReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StrategyFailReason::NoStunInfo => write!(f, "無對方 IP"),
            StrategyFailReason::BindFailed(e) => write!(f, "綁定失敗: {e}"),
            StrategyFailReason::Timeout => write!(f, "逾時"),
            StrategyFailReason::HandshakeFailed(e) => write!(f, "交握失敗: {e}"),
            StrategyFailReason::UPnPGatewayNotFound => write!(f, "Gateway 找不到"),
            StrategyFailReason::UPnPMappingFailed(e) => write!(f, "映射失敗: {e}"),
            StrategyFailReason::UPnPNotAttempted => write!(f, "未嘗試"),
            StrategyFailReason::CgnatDetected => write!(f, "偵測到 CGNAT"),
        }
    }
}

/// 策略結果
#[derive(Debug, Clone)]
pub enum StrategyOutcome {
    Success,
    Failed(StrategyFailReason),
    Skipped,
}

/// 單個策略的嘗試結果
#[derive(Debug, Clone)]
pub struct StrategyResult {
    pub method: StrategyKind,
    pub outcome: StrategyOutcome,
    /// 從嘗試開始到成功/失敗的總耗時
    pub duration_ms: u64,
}

/// UPnP mapping 成功的結果
pub struct UPnPMappingResult {
    pub external_addr: SocketAddr,
}

/// Host 端：建立 QUIC endpoint（只 bind，不 accept）
///
/// 優先嘗試 19870（StunInfo 相容），碰撞時 fallback ephemeral。
/// 返回 (endpoint, local_port)，caller 負責 accept。
pub fn bind_host_endpoint(tunnel_token: &str) -> Result<(quinn::Endpoint, u16)> {
    let alpn = make_alpn(tunnel_token);
    let (cert, key) = generate_self_signed()?;

    let mut crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    crypto.alpn_protocols = vec![alpn];

    let quic_crypto = QuicServerConfig::try_from(crypto)?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));
    let transport = Arc::get_mut(&mut config.transport).unwrap();
    transport.max_concurrent_uni_streams(0u32.into());
    transport.max_concurrent_bidi_streams(1u32.into());

    let preferred: SocketAddr = ([0, 0, 0, 0, 0, 0, 0, 0], QUIC_PREFERRED_PORT).into();
    let preferred_v4: SocketAddr = ([0, 0, 0, 0], QUIC_PREFERRED_PORT).into();
    let ephemeral: SocketAddr = ([0, 0, 0, 0, 0, 0, 0, 0], 0).into();
    let ephemeral_v4: SocketAddr = ([0, 0, 0, 0], 0).into();
    let endpoint = quinn::Endpoint::server(config.clone(), preferred)
        .or_else(|_| quinn::Endpoint::server(config.clone(), preferred_v4))
        .or_else(|_| quinn::Endpoint::server(config.clone(), ephemeral))
        .or_else(|_| quinn::Endpoint::server(config, ephemeral_v4))
        .context("QUIC endpoint bind 失敗")?;

    let local_port = endpoint.local_addr()?.port();
    info!(port = local_port, "QUIC host endpoint 已建立");
    Ok((endpoint, local_port))
}

/// Host 端：在已建立的 endpoint 上等待 joiner 連線
pub async fn accept_on_endpoint(
    endpoint: &quinn::Endpoint,
) -> Result<(quinn::SendStream, quinn::RecvStream)> {
    let incoming = tokio::time::timeout(CONNECT_TIMEOUT, endpoint.accept())
        .await
        .context("QUIC accept timeout")?
        .context("no incoming connection")?;
    let conn = tokio::time::timeout(CONNECT_TIMEOUT, incoming)
        .await
        .context("QUIC handshake timeout")?
        .context("QUIC handshake 失敗")?;
    info!(remote = %conn.remote_address(), "QUIC 直連建立");

    let (send, recv) = conn.accept_bi().await?;
    Ok((send, recv))
}

/// Joiner 端：連線到 host QUIC
///
/// `target` 為完整 SocketAddr（IP + port）
pub async fn connect_direct(
    target: SocketAddr,
    tunnel_token: &str,
) -> Result<(quinn::SendStream, quinn::RecvStream)> {
    let alpn = make_alpn(tunnel_token);

    let mut crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();
    crypto.alpn_protocols = vec![alpn];

    let quic_crypto = QuicClientConfig::try_from(crypto)
        .map_err(|e| anyhow::anyhow!("QUIC client config: {e}"))?;
    let bind_addr: SocketAddr = if target.ip().is_ipv6() {
        ([0u8; 16], 0).into()
    } else {
        ([0, 0, 0, 0], 0).into()
    };
    let mut endpoint = quinn::Endpoint::client(bind_addr)?;
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(quic_crypto)));

    info!(%target, "QUIC joiner 連線中");

    let conn = tokio::time::timeout(CONNECT_TIMEOUT, endpoint.connect(target, "war3-tunnel")?)
        .await
        .context("QUIC connect timeout")??;
    info!(remote = %conn.remote_address(), "QUIC 直連建立");

    let (send, recv) = conn.open_bi().await?;
    Ok((send, recv))
}

/// Host 端：嘗試 UPnP port mapping
///
/// 1. 搜尋 gateway
/// 2. 取得外部 IP，與 `stun_ip` 比較偵測 CGNAT
/// 3. 新增 port mapping
///
/// 成功返回 `UPnPMappingResult`，caller 負責在不需要時呼叫 `remove_port()` 清理
pub async fn attempt_upnp_mapping(
    local_port: u16,
    stun_ip: Option<IpAddr>,
) -> std::result::Result<UPnPMappingResult, StrategyFailReason> {
    use igd_next::SearchOptions;
    use igd_next::aio::tokio::search_gateway;

    // 1. 搜尋 gateway
    let opts = SearchOptions {
        timeout: Some(UPNP_SEARCH_TIMEOUT),
        ..Default::default()
    };
    // SearchOptions.timeout 已處理逾時，不需外層 tokio::time::timeout
    let gateway = search_gateway(opts)
        .await
        .map_err(|_| StrategyFailReason::UPnPGatewayNotFound)?;
    info!(gateway = %gateway.addr, "UPnP gateway 找到");

    // 2. 取得外部 IP
    let external_ip = tokio::time::timeout(UPNP_MAP_TIMEOUT, gateway.get_external_ip())
        .await
        .map_err(|_| StrategyFailReason::UPnPMappingFailed("取得外部 IP 逾時".into()))?
        .map_err(|e| StrategyFailReason::UPnPMappingFailed(format!("取得外部 IP: {e}")))?;

    // 3. CGNAT 偵測：比較 UPnP 外部 IP 和 STUN 看到的 IP
    if let Some(stun) = stun_ip {
        let upnp_ip = external_ip;
        if upnp_ip != stun {
            warn!(
                upnp = %upnp_ip,
                stun = %stun,
                "CGNAT 偵測：UPnP 外部 IP 與 STUN IP 不同，跳過 UPnP"
            );
            return Err(StrategyFailReason::CgnatDetected);
        }
    }

    // 4. Port mapping
    let local_addr: SocketAddr = ([0, 0, 0, 0], local_port).into();
    tokio::time::timeout(
        UPNP_MAP_TIMEOUT,
        gateway.add_port(
            igd_next::PortMappingProtocol::UDP,
            UPNP_EXTERNAL_PORT,
            local_addr,
            UPNP_LEASE_SECS,
            "War3 Battle Tool",
        ),
    )
    .await
    .map_err(|_| StrategyFailReason::UPnPMappingFailed("port mapping 逾時".into()))?
    .map_err(|e| StrategyFailReason::UPnPMappingFailed(format!("{e}")))?;

    let external_addr = SocketAddr::new(external_ip, UPNP_EXTERNAL_PORT);
    info!(%external_addr, "UPnP port mapping 成功");

    Ok(UPnPMappingResult { external_addr })
}

/// 清理 UPnP port mapping（背景呼叫，失敗靜默）
#[allow(dead_code)]
pub async fn cleanup_upnp_mapping() {
    use igd_next::SearchOptions;
    use igd_next::aio::tokio::search_gateway;

    let opts = SearchOptions {
        timeout: Some(Duration::from_secs(1)),
        ..Default::default()
    };
    match search_gateway(opts).await {
        Ok(gw) => {
            match gw
                .remove_port(igd_next::PortMappingProtocol::UDP, UPNP_EXTERNAL_PORT)
                .await
            {
                Ok(()) => info!("UPnP port mapping 已清理"),
                Err(e) => warn!(%e, "UPnP port mapping 清理失敗"),
            }
        }
        Err(_) => {
            // Gateway 找不到，可能 mapping 已過期
        }
    }
}

/// tunnel_token 前 16 bytes 做 ALPN protocol（避免 ALPN 過長）
fn make_alpn(tunnel_token: &str) -> Vec<u8> {
    let short = tunnel_token.get(..16).unwrap_or(tunnel_token);
    format!("w3t-{short}").into_bytes()
}

fn generate_self_signed() -> Result<(
    rustls::pki_types::CertificateDer<'static>,
    rustls::pki_types::PrivateKeyDer<'static>,
)> {
    let cert = rcgen::generate_simple_self_signed(vec!["war3-tunnel".to_string()])?;
    let cert_der = rustls::pki_types::CertificateDer::from(cert.cert);
    let key_der = rustls::pki_types::PrivateKeyDer::try_from(cert.key_pair.serialize_der())
        .map_err(|e| anyhow::anyhow!("key conversion: {e}"))?;
    Ok((cert_der, key_der))
}

/// 跳過 server cert 驗證（用 ALPN token 做身份驗證）
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
