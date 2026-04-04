# Windows 整合測試指南

## 前提
- Rust 已安裝 (rustup)
- Visual Studio Build Tools 已安裝 (C++ workload)
- Warcraft III 1.27 或 1.29c 已安裝

## 建置

```powershell
cd C:\Users\<username>\war3-battle-tool
git pull origin master

# build.rs 會自動下載 16MB CJK 字型
cargo build --release --package war3-client
```

## 測試流程

### 基本連線測試

1. 執行 `target\release\war3-client.exe`
2. 驗證：
   - [ ] 啟動顯示首次設定精靈
   - [ ] 輸入暱稱，選版本，進入大廳
   - [ ] 狀態列顯示 "● 已連線"

### 建房 + 加入測試（LAN，同一台機器）

**Host 端：**
1. 開 War3，建立區域網路遊戲
2. 在 War3 Battle Tool 點「建立房間」
3. 驗證：大廳顯示你的房間

**Joiner 端（第二個 client）：**
1. 開另一個 War3 Battle Tool
2. 看到房間，點「加入」
3. 驗證：
   - [ ] Log 顯示 "加入成功！正在建立 tunnel 連線..."
   - [ ] Log 顯示 "Tunnel proxy 就緒"
   - [ ] Log 顯示 "GAMEINFO 注入開始"
4. 切到 War3 區域網路畫面
5. 驗證：
   - [ ] War3 顯示房間
   - [ ] 點 Join，遊戲開始

### Tunnel Relay 測試（WAN，兩個不同網路）

同上流程，但兩台 Windows 在不同網路環境。這是 go/no-go gate。

## 疑難排解

### War3 看不到房間
- 確認 War3 在區域網路畫面
- 確認 Log 有 "GAMEINFO 注入開始"
- 確認 127.0.0.2 loopback 可用：`ping 127.0.0.2`

### Tunnel 連線失敗
- 確認 server 可達：瀏覽器開 `https://war3.kalthor.cc/health`
- 確認 nginx 有 `/tunnel` 路由設定
- 檢查 Log 的錯誤訊息

### Port 6112 被佔用
- War3 正在 host 模式會佔用 6112
- Joiner 端不要同時 host War3 遊戲
