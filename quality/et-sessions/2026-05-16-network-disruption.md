# ET Charter: 網路中斷與重連時 client 狀態一致性

**Target（目標）：** Client 的 connection lifecycle — `ConnectionState`、`PendingAction`、`LogPanel`、`LobbyPanel` 在網路擾動下的一致性

**Task（任務）：**
故意觸發網路擾動，觀察 client UI 與內部狀態是否同步、是否有殘留 stale state。具體執行情境：

1. **拔網路線/Wi-Fi 切 4G** — 連線中、Lobby 顯示房間時切斷
2. **DNS 失敗** — 修改 hosts 把 `war3.kalthor.cc` 指向 127.0.0.1，重啟 client
3. **Server 重啟** — pending join/create room 時 server 重啟，看 reconnect 行為
4. **網路 flaky** — 用 `tc netem` 加 30% 封包丟失，玩到一半連線抖動
5. **慢速網路** — 用 `tc netem` 加 500ms 延遲，看 pending banner 是否 stuck

**Timebox（時限）：** 45 min（單一情境約 7-8 min × 5）

**Trigger（觸發）：** PR #28 (cmd_tx silent drop fix) merge 後驗證 — `try_send_cmd` 修了「按按鈕沒反應」，但只在 receiver dropped 時生效。網路抖動還會引發其他 stale state 嗎？

### 探索焦點（D-SILENT、D-DOC charter seeds 交集）

- `pending_action: Joining/CreatingRoom` 在連線失敗時是否被清除？
- `LogPanel` 是否在斷線時持續累積錯誤（影響 ring buffer 行為）？
- `connection_state: Reconnecting { attempt }` 顯示的 attempt 數是否與實際嘗試吻合？
- 重連成功後 `players` / `rooms` 是否會殘留斷線前的舊資料一段時間？
- 「複製連結」按鈕在 host 房間掉線後仍可點，連結是否仍有效？

### Session Report

**日期：** （執行時填）
**實際耗時：** __ min（探索 __% / 調查 __% / 記錄 __%）

#### 發現

- [ ] Issue #__ — 描述

#### 新 Pattern 發現

- [ ] 可加入搜查手冊？（D-XXXX 類別 / 新類別）

#### 筆記

（執行時自由記錄）
