# War3 Battle Tool (War3 對戰工具)

下載、打開、對戰。不需要安裝任何東西，不需要設定 port forward。

## 功能

- 零配置：下載 exe 打開就能用
- 自動發現玩家，不用手動輸入 IP
- 一鍵加入房間，War3 自動出現遊戲
- 雙方都不需要開 port（透過 Cloud Relay）
- 支援 Warcraft III 1.27 / 1.29c

## 系統需求

- Windows 10/11

## 安裝

1. 從 [GitHub Releases](https://github.com/hottim900/war3-battle-tool/releases) 下載最新版 `war3-client.exe`
2. 執行 `war3-client.exe`

## 使用方式

1. 開啟程式，設定暱稱
2. 畫面會顯示目前在線的房間
3. 點「加入」，然後切到 War3 區域網路畫面
4. War3 會自動顯示房間，點 Join 開始遊戲

### 建房

1. 先在 War3 建立區域網路遊戲
2. 回到 War3 Battle Tool，點「建立房間」
3. 其他玩家就能看到你的房間並加入

## 開發者

### 建置

```bash
cargo build --workspace --exclude spike-packet
```

### 專案架構

| Crate | 說明 |
|-------|------|
| `war3-client` | Windows GUI 客戶端 (eframe + tokio)，tunnel relay + raw UDP 注入 |
| `war3-server` | WebSocket 大廳 (/ws) + 遊戲 relay (/tunnel) 伺服器 |
| `war3-protocol` | 共用的訊息格式與 War3 封包定義 |
| `spike-raw-udp` | 驗證用 PoC 工具（不納入預設 build） |

### 執行測試

```bash
cargo test --workspace --exclude spike-packet
```

## 自架 server

想用自己的 domain 跑 server？見 [docs/SELF-HOSTING.md](docs/SELF-HOSTING.md) — 涵蓋 `WAR3_ALLOWED_ORIGINS` 語意、nginx 反向代理、TLS、與部署不變式。

## License

MIT
