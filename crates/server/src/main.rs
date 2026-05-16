mod state;
mod tunnel;
mod ws;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use tokio::time::interval;
use tower_http::cors::CorsLayer;
use tracing::{error, info, warn};
use url::Url;

use state::AppState;
use tunnel::TunnelState;

struct SharedState {
    app: Arc<AppState>,
    tunnel: Arc<TunnelState>,
}

const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);
const GRACE_PERIOD: Duration = Duration::from_secs(60);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(10);

const ORIGIN_ALLOWLIST_ENV: &str = "WAR3_ALLOWED_ORIGINS";
const ORIGIN_DENY_BODY: &str = "Origin not allowed. Set WAR3_ALLOWED_ORIGINS on server. See https://github.com/hottim900/war3-battle-tool/blob/master/docs/SELF-HOSTING.md";

static ALLOWED_ORIGINS: OnceLock<Vec<String>> = OnceLock::new();

fn browser_allowed_origins() -> &'static [String] {
    ALLOWED_ORIGINS.get().map(Vec::as_slice).unwrap_or(&[])
}

/// 解析單一 allowlist entry。要求 RFC 6454 serialized origin 形式：
/// scheme + host + optional port，禁止 path/query/fragment/userinfo。
fn parse_allowed_origin(s: &str) -> Result<String, String> {
    let url = Url::parse(s).map_err(|e| format!("無法解析 {s:?}：{e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("scheme 必須是 http/https：{s:?}"));
    }
    if !url.path().is_empty() && url.path() != "/" {
        return Err(format!("不可含 path：{s:?}"));
    }
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
        return Err(format!("不可含 query/fragment/userinfo：{s:?}"));
    }
    let host = url.host_str().ok_or_else(|| format!("無 host：{s:?}"))?;
    let scheme = url.scheme();
    Ok(match url.port() {
        Some(p) => format!("{scheme}://{host}:{p}"),
        None => format!("{scheme}://{host}"),
    })
}

/// 從 WAR3_ALLOWED_ORIGINS env var 載入 allowlist。
/// 未設或為純空白時用內建預設（production + localhost dev variants）。
fn load_allowed_origins() -> Result<Vec<String>, String> {
    let raw = std::env::var(ORIGIN_ALLOWLIST_ENV).ok();
    let raw = match raw.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return Ok(vec![
                "https://war3.kalthor.cc".to_string(),
                "http://localhost".to_string(),
                "https://localhost".to_string(),
                "http://127.0.0.1".to_string(),
                "http://[::1]".to_string(),
            ]);
        }
    };
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_allowed_origin)
        .collect()
}

/// 嚴格 Origin 驗證（per RFC 6454）：
/// - 無 ORIGIN header → Ok（native War3 client 不送 Origin）
/// - non-UTF-8 / multiple Origin headers → Err
/// - 非 http(s) scheme / 有 path/query/fragment/userinfo → Err
/// - 解析後 scheme+host[:port] 對應 allowlist：
///   - allowlist entry 含 port：必須完全相符
///   - allowlist entry 不含 port（如 `http://localhost`）：該 host 任何 port 接受
///
/// 失敗時 Err 內含 rejected origin（給 warn log 用）。
fn validate_origin_against(headers: &HeaderMap, allowed: &[String]) -> Result<(), String> {
    let mut iter = headers.get_all(header::ORIGIN).iter();
    let Some(raw) = iter.next() else {
        return Ok(());
    };
    if iter.next().is_some() {
        return Err("multiple Origin headers".into());
    }
    let s = raw.to_str().map_err(|_| "non-UTF-8 Origin".to_string())?;
    let url = Url::parse(s).map_err(|_| s.to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(s.to_string());
    }
    if !url.path().is_empty() && url.path() != "/" {
        return Err(s.to_string());
    }
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
        return Err(s.to_string());
    }
    let host = url.host_str().ok_or_else(|| s.to_string())?;
    let scheme = url.scheme();
    let port = url.port_or_known_default();
    let candidate = match port {
        Some(p) => format!("{scheme}://{host}:{p}"),
        None => format!("{scheme}://{host}"),
    };
    let plain = format!("{scheme}://{host}");
    if allowed
        .iter()
        .any(|a| a.eq_ignore_ascii_case(&candidate) || a.eq_ignore_ascii_case(&plain))
    {
        Ok(())
    } else {
        Err(s.to_string())
    }
}

fn validate_origin(headers: &HeaderMap) -> Result<(), String> {
    validate_origin_against(headers, browser_allowed_origins())
}

/// 拒絕 Origin：共用 warn log + 403 body，ws_handler 和 tunnel_handler 都走此 path。
fn deny_origin(rejection: &str) -> axum::response::Response {
    // `rejection` 可能是 raw origin（不在 allowlist）或 header 結構錯誤的描述
    // （"multiple Origin headers" / "non-UTF-8 Origin"）。
    //
    // 不把整份 allowlist 寫進每筆 warn — 攻擊者狂 spam bad Origin 會炸 log。
    // Allowlist 在啟動時 info! 一次（main()），這裡只記 rejection + env var
    // 名稱讓 operator 知道哪個 knob 控制。
    warn!(
        rejection = %rejection,
        env_var = ORIGIN_ALLOWLIST_ENV,
        "拒絕未在 allowlist 的 Origin"
    );
    (StatusCode::FORBIDDEN, ORIGIN_DENY_BODY).into_response()
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "war3_server=info".parse().unwrap()),
        )
        .init();

    let allowed = match load_allowed_origins() {
        Ok(a) => a,
        Err(e) => {
            error!(error = %e, env_var = ORIGIN_ALLOWLIST_ENV, "Origin allowlist 設定錯誤，server 終止");
            std::process::exit(1);
        }
    };
    info!(allowlist = ?allowed, env_var = ORIGIN_ALLOWLIST_ENV, "Origin allowlist 載入完成");
    ALLOWED_ORIGINS
        .set(allowed)
        .expect("ALLOWED_ORIGINS 已初始化");

    let app_state = AppState::new();
    let tunnel_state = TunnelState::new();

    spawn_cleanup_task(app_state.clone(), tunnel_state.clone());

    let shared = Arc::new(SharedState {
        app: app_state,
        tunnel: tunnel_state,
    });

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/tunnel", get(tunnel_handler))
        .route("/health", get(|| async { "ok" }))
        // 全域 permissive：對 /health 等 HTTP endpoint 生效；
        // WebSocket 升級不走 CORS preflight，安全來自 Origin allowlist (v0.4.1+)
        // + 連線後限流 + 訊息協定。詳見 CLAUDE.md「Server 安全模型」
        .layer(CorsLayer::permissive())
        .with_state(shared);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let bind_addr: std::net::IpAddr = std::env::var("BIND")
        .ok()
        .and_then(|b| b.parse().ok())
        .unwrap_or_else(|| [127, 0, 0, 1].into());
    let addr = SocketAddr::from((bind_addr, port));
    info!("War3 發現伺服器啟動於 {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

/// 從 X-Real-IP header 取得真實 IP，僅信任來自 loopback 的連線（nginx）
fn real_ip(headers: &HeaderMap, fallback: SocketAddr) -> IpAddr {
    if fallback.ip().is_loopback() {
        headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<IpAddr>().ok())
            .unwrap_or_else(|| fallback.ip())
    } else {
        fallback.ip()
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(shared): State<Arc<SharedState>>,
) -> impl IntoResponse {
    // E3: Origin 驗證必須在 try_acquire_connection 之前，否則 hostile Origin
    // 仍會佔 per-IP 連線 slot 達 connection lifetime（permanent leak until release）
    if let Err(origin) = validate_origin(&headers) {
        return deny_origin(&origin);
    }

    let client_ip = real_ip(&headers, addr);
    let state = shared.app.clone();

    if !state.try_acquire_connection(client_ip).await {
        warn!(%client_ip, "連線數超過上限，拒絕連線");
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Too many connections from this IP",
        )
            .into_response();
    }

    let tunnel_state = shared.tunnel.clone();
    info!(%client_ip, "WebSocket 連線");
    ws.on_upgrade(move |socket| async move {
        ws::handle_socket(socket, client_ip, state.clone(), tunnel_state).await;
        state.release_connection(client_ip).await;
    })
    .into_response()
}

async fn tunnel_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
    State(shared): State<Arc<SharedState>>,
) -> impl IntoResponse {
    // E3: 見 ws_handler — validate 必須在 try_acquire 之前
    if let Err(origin) = validate_origin(&headers) {
        return deny_origin(&origin);
    }

    let client_ip = real_ip(&headers, addr);
    let tunnel_state = shared.tunnel.clone();

    let token = match params.get("token") {
        Some(t) if !t.is_empty() => t.clone(),
        _ => {
            return (StatusCode::BAD_REQUEST, "Missing token parameter").into_response();
        }
    };

    let role = match params.get("role") {
        Some(r) if r == "host" || r == "join" => r.clone(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "Invalid role parameter (host|join)",
            )
                .into_response();
        }
    };

    if !tunnel_state.try_acquire_tunnel_connection(client_ip).await {
        warn!(%client_ip, "Tunnel 連線數超過上限");
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Too many tunnel connections from this IP",
        )
            .into_response();
    }

    let token_short = token.get(..8).unwrap_or(&token).to_string();
    info!(%client_ip, %token_short, %role, "Tunnel WebSocket 連線");

    ws.on_upgrade(move |socket| async move {
        tunnel::handle_tunnel(socket, client_ip, token, role, tunnel_state.clone()).await;
        tunnel_state.release_tunnel_connection(client_ip).await;
    })
    .into_response()
}

fn spawn_cleanup_task(state: Arc<AppState>, tunnel_state: Arc<TunnelState>) {
    tokio::spawn(async move {
        let mut tick = interval(CLEANUP_INTERVAL);
        loop {
            tick.tick().await;

            // 清理過期 tunnel sessions
            tunnel_state.cleanup_expired().await;

            let expired = state.find_expired(HEARTBEAT_TIMEOUT, GRACE_PERIOD).await;

            if expired.is_empty() {
                continue;
            }

            for player_id in &expired {
                warn!(%player_id, "玩家超時（心跳或 grace period），移除");
                state.remove_player(player_id).await;
            }
            state.broadcast_state().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn default_allowlist() -> Vec<String> {
        vec![
            "https://war3.kalthor.cc".to_string(),
            "http://localhost".to_string(),
            "https://localhost".to_string(),
            "http://127.0.0.1".to_string(),
            "http://[::1]".to_string(),
        ]
    }

    fn make_headers(origin: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
        h
    }

    fn assert_allow(origin: &str) {
        let h = make_headers(origin);
        assert!(
            validate_origin_against(&h, &default_allowlist()).is_ok(),
            "expected allow: {origin}"
        );
    }

    fn assert_deny(origin: &str) {
        let h = make_headers(origin);
        assert!(
            validate_origin_against(&h, &default_allowlist()).is_err(),
            "expected deny: {origin}"
        );
    }

    #[test]
    fn no_origin_header_allowed() {
        let h = HeaderMap::new();
        assert!(validate_origin_against(&h, &default_allowlist()).is_ok());
    }

    #[test]
    fn non_utf8_origin_rejected() {
        let mut h = HeaderMap::new();
        h.insert(
            header::ORIGIN,
            HeaderValue::from_bytes(&[0xff, 0xfe, 0xfd]).unwrap(),
        );
        assert!(validate_origin_against(&h, &default_allowlist()).is_err());
    }

    #[test]
    fn multiple_origin_headers_rejected() {
        let mut h = HeaderMap::new();
        h.append(
            header::ORIGIN,
            HeaderValue::from_static("https://war3.kalthor.cc"),
        );
        h.append(header::ORIGIN, HeaderValue::from_static("https://evil.com"));
        assert!(validate_origin_against(&h, &default_allowlist()).is_err());
    }

    #[test]
    fn production_origin_allowed() {
        assert_allow("https://war3.kalthor.cc");
    }

    #[test]
    fn production_origin_case_insensitive() {
        assert_allow("https://WAR3.KALTHOR.CC");
    }

    #[test]
    fn localhost_any_port_allowed() {
        assert_allow("http://localhost:3000");
        assert_allow("https://localhost:5173");
        assert_allow("http://127.0.0.1:8080");
        assert_allow("http://[::1]:5173");
    }

    #[test]
    fn evil_origin_rejected() {
        assert_deny("https://evil.com");
    }

    #[test]
    fn suffix_attack_rejected() {
        // Critical: prevent `*.war3.kalthor.cc.evil.com` impersonation.
        assert_deny("https://war3.kalthor.cc.evil.com");
    }

    #[test]
    fn userinfo_plus_suffix_attack_rejected() {
        assert_deny("https://attacker@war3.kalthor.cc.evil.com");
    }

    #[test]
    fn origin_with_path_rejected() {
        assert_deny("https://war3.kalthor.cc/path");
    }

    #[test]
    fn origin_with_query_rejected() {
        assert_deny("https://war3.kalthor.cc?x=1");
    }

    #[test]
    fn chrome_extension_scheme_rejected() {
        assert_deny("chrome-extension://abc/");
    }

    #[test]
    fn file_scheme_rejected() {
        assert_deny("file:///home/user");
    }

    #[test]
    fn ftp_scheme_rejected() {
        assert_deny("ftp://war3.kalthor.cc");
    }

    #[test]
    fn malformed_url_rejected() {
        assert_deny("not-a-url");
    }

    #[test]
    fn opaque_null_origin_rejected() {
        // RFC 6454 sandboxed contexts send "null" — must reject
        assert_deny("null");
    }

    #[test]
    fn punycode_lookalike_rejected() {
        // Explicit guard: even if visually resembles war3.kalthor.cc, must reject
        assert_deny("xn--war3-1234.kalthor.cc");
    }

    #[test]
    fn parse_allowed_origin_normalizes_scheme_host_port() {
        assert_eq!(
            parse_allowed_origin("https://example.com").unwrap(),
            "https://example.com"
        );
        assert_eq!(
            parse_allowed_origin("http://example.com:8080").unwrap(),
            "http://example.com:8080"
        );
        assert_eq!(
            parse_allowed_origin("http://[::1]").unwrap(),
            "http://[::1]"
        );
    }

    #[test]
    fn parse_allowed_origin_rejects_invalid_entries() {
        assert!(parse_allowed_origin("not-a-url").is_err());
        assert!(parse_allowed_origin("ftp://example.com").is_err());
        assert!(parse_allowed_origin("https://example.com/path").is_err());
        assert!(parse_allowed_origin("https://example.com?q=1").is_err());
        assert!(parse_allowed_origin("https://user@example.com").is_err());
    }

    #[test]
    fn allowlist_entry_with_port_requires_exact_match() {
        let allow = vec!["http://localhost:3000".to_string()];
        let h = make_headers("http://localhost:3000");
        assert!(validate_origin_against(&h, &allow).is_ok());
        let h2 = make_headers("http://localhost:5173");
        assert!(validate_origin_against(&h2, &allow).is_err());
    }

    #[test]
    fn empty_origin_value_rejected() {
        // 空字串 Origin（malformed client / fuzzer input）— Url::parse 拒絕
        assert_deny("");
    }
}
