# Windows 整合測試指南

## 前提
- Rust 已安裝 (rustup)
- npcap 已安裝 (npcap.com，勾選 WinPcap API-compatible mode)
- Visual Studio Build Tools 已安裝 (C++ workload)

## 步驟

```powershell
cd C:\Users\Tim\war3-battle-tool
git pull origin feat/client-crate

# 1. Build (build.rs 會自動下載 16MB CJK 字型)
cargo build --release --package war3-client
cargo build --release --package war3-server

# 2. 啟動 server
start target\release\war3-server.exe

# 3. 啟動 client
target\release\war3-client.exe
```

## 驗證清單

- [ ] client 啟動，顯示首次設定精靈
- [ ] 輸入暱稱，選版本，進入大廳
- [ ] 狀態列顯示 "● 已連線"
- [ ] 建立房間，大廳顯示房間
- [ ] 開第二個 client，看到房間列表
- [ ] 點加入，log 顯示 "封包注入成功"
- [ ] 開 War3 → 區域網路 → 看到房間

## 如果 War3 看不到房間

封包格式可能需要調整。用 Wireshark 抓 loopback 上的 UDP port 6112 流量，對比正常 LAN 的 W3GS_GAMEINFO 封包。

## Claude Code prompt (Windows session)

```
在 C:\Users\Tim\war3-battle-tool 專案。所有程式碼已寫好，需要 Windows 整合測試。
1. cargo build --release --package war3-client 確認編譯
2. 如果 NpcapSender 編譯失敗，debug pcap crate linking
3. 啟動 server + client，驗證 GUI 正常
4. 開 War3 1.27，測試加入房間是否能在 LAN 畫面看到房間
5. 如果看不到，用 Wireshark 分析封包格式差異並修正
```
