# War3 Battle Tool (War3 對戰工具)

打開就能看到誰在線上，點一下就能一起玩。

## 功能

- 自動發現玩家 -- 不用手動輸入 IP
- 一鍵加入房間
- 支援 Warcraft III 1.27 / 1.29c

## 系統需求

- Windows
- [Npcap](https://npcap.com) (用於區網封包偵測)

## 安裝

1. 從 [GitHub Releases](https://github.com/user/war3-battle-tool/releases) 下載最新版
2. 安裝 [Npcap](https://npcap.com)
3. 執行 `war3-client.exe`

## 使用方式

1. 開啟程式
2. 設定你的暱稱
3. 畫面會顯示目前在線的房間
4. 點「加入」就能開始遊戲

## 開發者

### 建置

```bash
cargo build --workspace
```

### 專案架構

workspace 包含 3 個 crate：

| Crate | 說明 |
|-------|------|
| `war3-client` | GUI 客戶端 (eframe)，負責房間探索與加入 |
| `war3-server` | WebSocket 大廳伺服器，負責撮合玩家 |
| `war3-protocol` | 共用的訊息格式與 War3 封包定義 |

### 執行測試

```bash
cargo test --workspace
```

## License

MIT
