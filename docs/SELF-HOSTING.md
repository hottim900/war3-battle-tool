# Self-Hosting the War3 Battle Tool Server

想用自己的 domain 跑一份 server？這份文件涵蓋部署所需的所有設定，重點在 v0.4.1+ 新增的 `WAR3_ALLOWED_ORIGINS` 環境變數。

## TTHW（Time To Hello World）

```bash
# 1. 抓 release binary（或 cargo build --release --package war3-server）
wget https://github.com/hottim900/war3-battle-tool/releases/latest/download/war3-server-linux

# 2. 設定 allowlist + 啟動
chmod +x war3-server-linux
WAR3_ALLOWED_ORIGINS=https://my-war3.example.com \
  PORT=3000 \
  ./war3-server-linux

# 3. 驗證 health
curl http://127.0.0.1:3000/health  # 應印 "ok"
```

Native War3 client 不送 `Origin` header，所以不論 allowlist 怎麼設都能連——`WAR3_ALLOWED_ORIGINS` 只影響瀏覽器來源（web viewer、自架的 lobby 頁面等）。

## 環境變數

| 變數 | 預設 | 用途 |
|---|---|---|
| `PORT` | `3000` | TCP 監聽埠 |
| `BIND` | `127.0.0.1` | 監聽位址。**改 `0.0.0.0` 前請看下方 X-Real-IP 警告** |
| `WAR3_ALLOWED_ORIGINS` | 內建 production+localhost 預設 | 瀏覽器 Origin 白名單 |
| `RUST_LOG` | `war3_server=info` | tracing 過濾器 |

## `WAR3_ALLOWED_ORIGINS` 語意

**未設或為純空白**：使用內建預設

```
https://war3.kalthor.cc
http://localhost
https://localhost
http://127.0.0.1
http://[::1]
```

**設為 comma-separated string**：用你給的清單替換預設（不是 append）

```bash
WAR3_ALLOWED_ORIGINS="https://my-war3.example.com,https://staging.example.com"
```

**設為空字串**：等同未設（fall back 到預設）。若要拒絕所有 browser Origins，給一個 unreachable origin：

```bash
WAR3_ALLOWED_ORIGINS="https://no-browsers-allowed.invalid"
# 任何瀏覽器來源都會被拒；native client 仍能連
```

### Entry 格式規則

每個 entry 必須是 RFC 6454 serialized origin：

| 範例 | 接受？ | 行為 |
|---|---|---|
| `https://example.com` | ✅ | 該 host 任何 port |
| `https://example.com/` | ✅ | 同上（trailing slash 等同 path="/"，視為空） |
| `https://example.com:8443` | ✅ | 該 host 僅限 port 8443 |
| `http://[::1]:5173` | ✅ | IPv6 + 明確 port |
| `https://example.com/path` | ❌ 啟動失敗 | 不可有非根 path |
| `https://example.com?x=1` | ❌ 啟動失敗 | 不可有 query/fragment |
| `https://user@example.com` | ❌ 啟動失敗 | 不可有 userinfo |
| `ftp://example.com` | ❌ 啟動失敗 | 必須 http/https |
| `*` / `*.example.com` | ❌ 啟動失敗 | **不支援 wildcard** |
| `example.com`（無 scheme） | ❌ 啟動失敗 | 必須含 scheme |

**Entry 含 port** = 該 port 才接受；**Entry 不含 port** = 該 host 任何 port 都接受。
產線部署一般用 `https://your-domain.com`（不含 port，因 HTTPS 自動 443），dev 用 `http://localhost`（涵蓋所有 dev server port）。

### 啟動驗證

Server 啟動時會 log allowlist，方便確認：

```
INFO Origin allowlist 載入完成 allowlist=["https://my-war3.example.com"]
```

若 entry 格式錯誤，server **拒絕啟動**並印錯誤：

```
ERROR WAR3_ALLOWED_ORIGINS 設定錯誤，server 終止 error=scheme 必須是 http/https："example.com"
```

這比 silent skip 安全——配置錯誤要早發現。

## Reverse proxy / TLS

production 建議走 nginx + Let's Encrypt + Cloudflare DNS only（不走 CF proxy，以保留真實 client IP）。範例 nginx 設定：

```nginx
upstream war3 {
    server 127.0.0.1:3000;
}

server {
    listen 443 ssl http2;
    server_name my-war3.example.com;

    ssl_certificate     /etc/letsencrypt/live/my-war3.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/my-war3.example.com/privkey.pem;

    # WebSocket /ws 與 /tunnel 都需要 upgrade headers
    location ~ ^/(ws|tunnel)$ {
        proxy_pass http://war3;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header X-Real-IP $remote_addr;   # server 從這個 header 取真實 IP
        proxy_set_header Host $host;
        proxy_set_header Origin $http_origin;       # 必須傳遞 Origin
        proxy_read_timeout 60s;
        # tunnel 用 — 配合 server-side 50KB/s rate limit
        limit_rate 100k;
    }

    location /health {
        proxy_pass http://war3;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

關鍵點：
- nginx 必須轉 `Origin` header（`proxy_set_header Origin $http_origin`），否則 server 收不到 → native client path 被誤判
- nginx 必須轉 `X-Real-IP`（server 從這取真實 IP）；server bind `127.0.0.1` 才能信任此 header

## 為什麼會收到 403？

| 症狀 | 原因 | 解法 |
|---|---|---|
| Browser fetch /ws → 403 "Origin not allowed" | 該 Origin 不在 allowlist | `WAR3_ALLOWED_ORIGINS` 加上你的 domain，重啟 server |
| Native client 連不上 → 403 | nginx 沒轉 `Origin` 但有送某個 header；或 `Origin` 是垃圾值 | 檢查 nginx 設定 `proxy_set_header Origin $http_origin` |
| 用 `Origin: null`（sandboxed iframe / file://） → 403 | RFC 6454 opaque origin，必須拒絕 | 用正規網域而非 file:// 開頁面 |

403 response body 會給你 env var 名稱 + 本文件 URL，方便除錯。

## 部署不變式（安全）

- **`BIND=127.0.0.1`**：預設，配合 nginx reverse proxy。若改 `0.0.0.0` 直接對外（開發測試），**必須移除 `X-Real-IP` 信任邏輯**（main.rs `real_ip()`），否則任何 client 都能透過 header 偽造 IP 繞過 per-IP 限制。
- **Origin allowlist 是 defense-in-depth，不是唯一防線**。每 IP 連線/訊息限流、Pairing token、訊息大小上限仍是 primary defense。Origin 主要 reduce CSRF surface area（為未來 cookie-based auth 預先部署）。
- **不要 hardcode wildcard `*`**：本實作刻意不支援，避免「演進到 cookie-based auth 後忘記收緊」的隱性 footgun。

## 未來會加但今天還沒做

以下旋鈕仍是 TODO（issue 在 backlog）：

- `BIND` 改自由綁定 + 自動偵測 X-Real-IP 信任邊界
- `WAR3_LOG_DIR` 環境變數（目前 `default_log_dir()` hardcoded）
- 自架者的 metric / health-extended endpoint
- 內建 Let's Encrypt（目前須走 nginx）

各項追蹤狀態見 `quality/backlog-audit.md`。

## 檢查清單

部署 production 前：

- [ ] `WAR3_ALLOWED_ORIGINS` 已設為你的 domain
- [ ] `curl https://my-war3.example.com/health` 印 `ok`
- [ ] 瀏覽器 devtools 模擬 `Origin: https://evil.com` 連你的 `/ws` → 應 403
- [ ] Native War3 client（exe）連你的 server → 仍 OK
- [ ] server 啟動 log 出現 `Origin allowlist 載入完成 allowlist=[...]` 且內容符合期望
- [ ] nginx 設定 `proxy_set_header Origin $http_origin` 與 `proxy_set_header X-Real-IP $remote_addr`
