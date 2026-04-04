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

### NpcapSender 程式碼品質
- payload overflow 邊界檢查 (npcap_sender.rs:build_loopback_packet)
- IP checksum 長度驗證
- loopback adapter 名稱匹配改為實際嘗試開啟
- loopback init 失敗改為 bail（非 warn）
- **When:** 僅在 raw UDP 實驗失敗、需要保留 npcap 時修正
