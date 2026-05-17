# War3 Battle Tool — Deferred Items

## From /autoplan review (2026-04-04)

### ~~NpcapSender 程式碼品質~~
- **Completed:** v0.2.0 (2026-04-04) — npcap 已移除，改用 raw UDP + tunnel relay

### P2P 延伸
- [x] UPnP 支援 — **Completed:** PR #13 (2026-04-05), Connection Strategy Engine
- [x] Phase 2: Mid-game WS relay → QUIC direct hot swap — **Completed:** PR #10 + #15 (`net/tunnel.rs::bridge_tcp_ws_with_swap`, swap_poc.rs 兩個 test 涵蓋 zero data loss + heavy load)
- [ ] 多人遊戲（>2 人）host 端多 tunnel 支援
  - Host 同時只持一條 tunnel：新 joiner 進來 abort 舊的（`app.rs::start_host_tunnel` 開頭呼叫 `abort_tunnel`）
  - 需要：`tunnel_handle: HashMap<token, JoinHandle>` 多 active tunnel + 驗證 War3 host 對多 TCP connection 行為
  - 影響：3+ player 房先進入的 joiner 會被踢
- [ ] QUIC stream 斷線後 WS 重建（升級後 QUIC 抖動目前直接斷線，無 fallback）
- [ ] Binary size CI gate（監控依賴膨脹）
- [ ] 台灣 ISP hole punch 成功率實測（中華電信 vs 手機熱點）

### 多區域 Relay Server
- 目前只有東京一個 VPS，台灣雙方延遲 ~60ms
- 評估新加坡或香港 VPS 降低延遲
- **When:** 使用者數量成長後

### 127.0.0.2 環境相容性
- 在多種 Windows 10/11 環境測試 127.0.0.2 loopback 是否可用
- 防火牆、企業環境、VPN 可能影響
- **When:** v0.2.0 發布後收集回饋
