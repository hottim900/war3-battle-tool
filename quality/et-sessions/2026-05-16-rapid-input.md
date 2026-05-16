# ET Charter: 暴力輸入與奇怪字串

**Target（目標）：** Client UI 對 rapid user input 與 edge-case 字串的反應 — `LobbyPanel`、`SetupWizard`、`Settings` 三個輸入點

**Task（任務）：**
故意製造異常輸入，觀察 client 是否有 UI 殘留、protocol 拒絕、或 server 端錯誤訊息被吞。具體：

1. **暴力連點** — 「建立房間」連點 10 次、「加入」連點 5 次、設定頁滑桿快速拖拉
2. **CJK / Emoji / RTL** — 暱稱輸入 `🎮War3傳奇王者⚔️`、`مرحبا`（阿拉伯文）、`Тест`（西里爾文）
3. **長度邊界** — 暱稱 32 字元、33 字元、零寬空格、純空白
4. **特殊字元** — 暱稱含 `<script>`、`'; DROP TABLE`、`\n` 換行符
5. **Server URL 異常** — 設定頁填 `wss://nonexistent.invalid`、`http://localhost:9999`、空字串
6. **房間名稱** — 用 War3 客戶端建房名為 emoji、超長、雙引號

**Timebox（時限）：** 40 min

**Trigger（觸發）：**
- PR #29 (log buffer config) merge 後 — 設定頁加了滑桿，順便 stress test 整個設定頁
- #22 修了 CJK 長度驗證但僅針對 protocol，UI 邊界未測

### 探索焦點

- **連點建房**：是否會建出多間自己的房間？UI 是否 disable 第二次按？
- **服務端訊息錯誤**：server 回 `ServerMessage::Error` 時，client UI 是否顯示？還是只 log？（與 PR #28 的「使用者看不到失敗」相關）
- **CJK 顯示**：egui 是否正確渲染所有字元？emoji 是否 fallback？
- **設定頁** `local_ip` 填非法值（`abc.def`、`999.999.999.999`）能否儲存？儲存後重啟 crash 嗎？
- **log buffer 滑桿** 拖拉時 `config_changed = true` 觸發頻率，是否每步都寫 disk（看 settings.rs:save 路徑）
- **超長暱稱顯示** 在 lobby 卡片是否 truncate（v0.3.3 已加 .truncate()，驗證有效）

### Session Report

**日期：** （執行時填）
**實際耗時：** __ min

#### 發現

- [ ] Issue #__ — 描述

#### 新 Pattern 發現

- [ ] 可加入搜查手冊？

#### 筆記

（執行時自由記錄）
