# War3 Battle Tool

台灣/海外華人 Warcraft III LAN 對戰配對工具。使用者開啟 → 看到誰在線上 → 一鍵加入。

## 架構

```
crates/
├── client/    # Windows GUI (egui + npcap + tokio)
├── server/    # Linux server (axum WebSocket)
├── protocol/  # 共用型別 (ClientMessage, ServerMessage, War3Version)
└── spike-packet/  # Windows-only 封包診斷工具（不納入預設 build）
```

- Server 部署在 `wss://war3.kalthor.cc/ws`，VPS 139.162.118.18，nginx + CF Origin Cert
- Client 透過 WebSocket 連線，npcap 注入 W3GS 封包讓 War3 看到 LAN 房間
- CI/CD：push master → GitHub Actions → 自動部署 server 到 VPS

## 常用指令

```bash
# 編譯檢查（排除 Windows-only 的 spike-packet）
cargo check --workspace --exclude spike-packet
cargo test --workspace --exclude spike-packet
cargo clippy --workspace --exclude spike-packet -- -D warnings
cargo fmt --all -- --check

# 單獨編譯
cargo build --release --package war3-server
cargo build --release --package war3-client  # 需要 Windows 或 npcap SDK
```

## 開發注意事項

- **語言**：繁體中文（台灣用語）用於 UI 文字、commit message、註解
- **Edition**：Rust 2024
- **TLS**：client 用 `rustls-tls-native-roots`，不依賴系統 OpenSSL
- **npcap**：不能 bundle，使用者需自行安裝。client 有 blocking panel 引導
- **spike-packet** 依賴 pcap crate，Linux CI 沒裝 libpcap 會 link fail，已用 `default-members` + `--exclude` 排除
- **Server bind**：預設 `127.0.0.1`（非 `0.0.0.0`），走 nginx reverse proxy
- **IP 來源**：server 從 `X-Real-IP` header 讀取真實 IP，僅信任 loopback 連線（nginx）

## Server 安全模型

- IP 不廣播，僅在 JoinRoom 時交換（需版本一致 + 房間未滿 + 房主在線）
- 每 IP 3 連線、每連線 10 msg/s、訊息 ≤ 4KB
- Join 冷卻 5 秒、全域 500 玩家 / 200 房間上限
- bounded channel (64)、send_task + receive_loop 用 select! 互相取消

## 部署

- **Server**：push master 自動 deploy（CI build → scp → systemctl restart）
- **Client**：push `v*` tag 觸發 release build（Windows exe + Linux server binary）
- GitHub Secrets：`DEPLOY_HOST`, `DEPLOY_USER` (war3-deploy), `DEPLOY_SSH_KEY`, `DEPLOY_HOST_KEY`

## PR 流程

- master 有 branch protection（CI check 必須通過）
- 開 feature branch → PR → CI 綠燈 → squash merge

## Skill routing

When the user's request matches an available skill, ALWAYS invoke it using the Skill
tool as your FIRST action. Do NOT answer directly, do NOT use other tools first.
The skill has specialized workflows that produce better results than ad-hoc answers.

Key routing rules:
- Product ideas, "is this worth building", brainstorming → invoke office-hours
- Bugs, errors, "why is this broken", 500 errors → invoke investigate
- Ship, deploy, push, create PR → invoke ship
- QA, test the site, find bugs → invoke qa
- Code review, check my diff → invoke review
- Update docs after shipping → invoke document-release
- Weekly retro → invoke retro
- Design system, brand → invoke design-consultation
- Visual audit, design polish → invoke design-review
- Architecture review → invoke plan-eng-review
- Save progress, checkpoint, resume → invoke checkpoint
- Code quality, health check → invoke health
