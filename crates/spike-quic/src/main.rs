//! quinn QUIC PoC: 驗證 UDP hole punch + TCP↔QUIC bridge
//!
//! 用法：
//!   spike-quic host <bind_port>         # 監聽 QUIC，bridge 到 127.0.0.1:6112
//!   spike-quic join <peer_addr:port>    # 連線 QUIC，監聽 127.0.0.2:6112 bridge
//!
//! 驗證項目：
//! 1. quinn 基本 QUIC 連線（self-signed cert）
//! 2. TCP ↔ QUIC stream bridge
//! 3. tunnel_token ALPN 驗證

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use quinn::Endpoint;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info};

/// 模擬 tunnel_token 做 ALPN 驗證
const ALPN_PROTOCOL: &[u8] = b"war3-tunnel-v1";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "spike_quic=info".parse().unwrap()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("用法:");
        eprintln!("  spike-quic host <bind_port>");
        eprintln!("  spike-quic join <peer_addr:port>");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "host" => {
            let port: u16 = args[2].parse().context("invalid port")?;
            run_host(port).await
        }
        "join" => {
            let peer_addr: SocketAddr = args[2].parse().context("invalid peer address")?;
            run_join(peer_addr).await
        }
        _ => {
            eprintln!("未知指令: {}。用 host 或 join。", args[1]);
            std::process::exit(1);
        }
    }
}

/// 產生 self-signed cert（PoC 用，正式版用 tunnel_token PSK）
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

fn make_server_config(
    cert: rustls::pki_types::CertificateDer<'static>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
) -> Result<quinn::ServerConfig> {
    let mut crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    crypto.alpn_protocols = vec![ALPN_PROTOCOL.to_vec()];

    let quic_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(crypto)?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));
    let transport = Arc::get_mut(&mut config.transport).unwrap();
    transport.max_concurrent_uni_streams(0u32.into());
    transport.max_concurrent_bidi_streams(1u32.into());
    Ok(config)
}

fn make_client_config() -> quinn::ClientConfig {
    let mut crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();
    crypto.alpn_protocols = vec![ALPN_PROTOCOL.to_vec()];

    let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
        .expect("failed to create QUIC client config");
    quinn::ClientConfig::new(Arc::new(quic_crypto))
}

/// Host: 監聽 QUIC，接受連線後 bridge 到 127.0.0.1:6112（War3）
async fn run_host(port: u16) -> Result<()> {
    let (cert, key) = generate_self_signed()?;
    let server_config = make_server_config(cert, key)?;

    let bind_addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    let endpoint = Endpoint::server(server_config, bind_addr)?;
    info!(%bind_addr, "QUIC host 監聽中");

    // 只接受一個連線（PoC）
    let incoming = endpoint.accept().await.context("no incoming connection")?;
    let conn = incoming.await?;
    info!(remote = %conn.remote_address(), "QUIC 連線建立");

    // 開啟雙向 stream
    let (mut quic_send, mut quic_recv) = conn.accept_bi().await?;
    info!("QUIC bi-stream 建立");

    // 連線到 War3 (127.0.0.1:6112)
    let tcp = TcpStream::connect("127.0.0.1:6112").await?;
    let (mut tcp_read, mut tcp_write) = tcp.into_split();
    info!("TCP 連線到 127.0.0.1:6112");

    // Bridge: TCP ↔ QUIC
    let quic_to_tcp = async {
        let mut buf = [0u8; 8192];
        while let Some(n) = quic_recv.read(&mut buf).await? {
            tcp_write.write_all(&buf[..n]).await?;
        }
        anyhow::Ok(())
    };

    let tcp_to_quic = async {
        let mut buf = [0u8; 8192];
        loop {
            let n = tcp_read.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            quic_send.write_all(&buf[..n]).await?;
        }
        anyhow::Ok(())
    };

    tokio::select! {
        r = quic_to_tcp => {
            if let Err(e) = r { error!(%e, "QUIC→TCP 錯誤"); }
        }
        r = tcp_to_quic => {
            if let Err(e) = r { error!(%e, "TCP→QUIC 錯誤"); }
        }
    }

    info!("Bridge 結束");
    endpoint.close(0u32.into(), b"done");
    Ok(())
}

/// Join: 連線到 host QUIC，監聽 127.0.0.2:6112 bridge 給 War3
async fn run_join(peer_addr: SocketAddr) -> Result<()> {
    let client_config = make_client_config();
    let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    info!(%peer_addr, "連線到 QUIC host");
    let conn = endpoint.connect(peer_addr, "war3-tunnel")?.await?;
    info!("QUIC 連線建立");

    // 開啟雙向 stream
    let (mut quic_send, mut quic_recv) = conn.open_bi().await?;
    info!("QUIC bi-stream 建立");

    // 監聽 127.0.0.2:6112
    let listener = TcpListener::bind("127.0.0.2:6112").await?;
    info!("等待 War3 連線到 127.0.0.2:6112");

    let (tcp, from) = listener.accept().await?;
    info!(%from, "War3 TCP 連線");
    let (mut tcp_read, mut tcp_write) = tcp.into_split();

    // Bridge: TCP ↔ QUIC
    let quic_to_tcp = async {
        let mut buf = [0u8; 8192];
        while let Some(n) = quic_recv.read(&mut buf).await? {
            tcp_write.write_all(&buf[..n]).await?;
        }
        anyhow::Ok(())
    };

    let tcp_to_quic = async {
        let mut buf = [0u8; 8192];
        loop {
            let n = tcp_read.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            quic_send.write_all(&buf[..n]).await?;
        }
        anyhow::Ok(())
    };

    tokio::select! {
        r = quic_to_tcp => {
            if let Err(e) = r { error!(%e, "QUIC→TCP 錯誤"); }
        }
        r = tcp_to_quic => {
            if let Err(e) = r { error!(%e, "TCP→QUIC 錯誤"); }
        }
    }

    info!("Bridge 結束");
    endpoint.close(0u32.into(), b"done");
    Ok(())
}

/// 跳過 server cert 驗證（PoC 用，正式版用 tunnel_token PSK）
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
