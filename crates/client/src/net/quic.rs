//! QUIC P2P 直連模組
//!
//! Host 監聽 QUIC endpoint，Joiner 連線。
//! 雙方使用 tunnel_token 做 ALPN 驗證。
//! 失敗時靜默回傳 Err，caller fallback 到 WS relay。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use tracing::info;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// QUIC 監聽的固定 port（host 端）
const QUIC_PORT: u16 = 19870;

/// Host 端：監聽 QUIC，等待 joiner 連線
///
/// 返回 (send, recv) stream，或 Err（caller fallback relay）
pub async fn accept_direct(tunnel_token: &str) -> Result<(quinn::SendStream, quinn::RecvStream)> {
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

    // 嘗試 dual-stack (IPv6 any)，失敗則 fallback IPv4 any
    let bind_addr: SocketAddr = ([0, 0, 0, 0, 0, 0, 0, 0], QUIC_PORT).into();
    let endpoint = quinn::Endpoint::server(config.clone(), bind_addr)
        .or_else(|_| {
            let v4: SocketAddr = ([0, 0, 0, 0], QUIC_PORT).into();
            quinn::Endpoint::server(config, v4)
        })
        .context("QUIC endpoint bind 失敗")?;
    info!(%bind_addr, "QUIC host 等待直連");

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
/// 返回 (send, recv) stream，或 Err（caller fallback relay）
pub async fn connect_direct(
    peer_ip: std::net::IpAddr,
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
    let bind_addr: SocketAddr = if peer_ip.is_ipv6() {
        ([0u8; 16], 0).into()
    } else {
        ([0, 0, 0, 0], 0).into()
    };
    let mut endpoint = quinn::Endpoint::client(bind_addr)?;
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(quic_crypto)));

    let target = SocketAddr::new(peer_ip, QUIC_PORT);
    info!(%target, "QUIC joiner 連線中");

    let conn = tokio::time::timeout(CONNECT_TIMEOUT, endpoint.connect(target, "war3-tunnel")?)
        .await
        .context("QUIC connect timeout")??;
    info!(remote = %conn.remote_address(), "QUIC 直連建立");

    let (send, recv) = conn.open_bi().await?;
    Ok((send, recv))
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
