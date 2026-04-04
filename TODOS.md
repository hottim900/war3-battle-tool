# War3 Battle Tool — Deferred Items

## From /autoplan review (2026-04-04)

### 玩家發現/推廣策略
- 在 War3 玩家聚集地宣傳（巴哈姆特、Discord、PTT、FB 群組）
- 製作 5 分鐘上手影片
- 測試朋友推薦的 viral loop
- **Why:** CEO review 指出發現比安裝更重要。零配置解決「想用但裝不了」，但「不知道有這工具」是更大的問題。

### 地理聚焦策略
- 決定優先經營台灣、大陸、還是東南亞市場
- 針對目標區域做在地化（用語、延遲優化、社群）
- **Why:** 開源被 fork 的風險。先在一個地方做深，建立 brand loyalty。

### 競爭對手分析
- 調查台灣/海外老 War3 社群是否有其他類似工具
- 分析 W3Champions 對舊版的策略（是否計畫支援）

### ~~NpcapSender 程式碼品質~~
- **Completed:** v0.2.0 (2026-04-04) — npcap 已移除，改用 raw UDP + tunnel relay

### 多區域 Relay Server
- 目前只有東京一個 VPS，台灣雙方延遲 ~60ms
- 評估新加坡或香港 VPS 降低延遲
- **When:** 使用者數量成長後

### 127.0.0.2 環境相容性
- 在多種 Windows 10/11 環境測試 127.0.0.2 loopback 是否可用
- 防火牆、企業環境、VPN 可能影響
- **When:** v0.2.0 發布後收集回饋
