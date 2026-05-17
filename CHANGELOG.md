# Changelog

All notable changes to this project will be documented in this file.

## [0.4.1] - 2026-05-17

### Security
- Server WebSocket `/ws` 與 `/tunnel` 加 RFC 6454 strict Origin allowlist 驗證：瀏覽器來源限定 `war3.kalthor.cc` 與 localhost 變體，native War3 client 不送 Origin → 不受影響。Validate-before-acquire 保證 hostile Origin 不會佔 per-IP 連線 slot（#34）

### Upgrade notes
- **自架 server 用自家 domain 的 deployer 必須設 `WAR3_ALLOWED_ORIGINS`** env var。預設只接受 `war3.kalthor.cc`、`localhost`、`127.0.0.1`、`[::1]` 的瀏覽器來源；其他 domain 的 browser-origin 連線會被 403 拒絕。範例：`WAR3_ALLOWED_ORIGINS=https://my-war3.example.com ./war3-server`。完整指引見 [`docs/SELF-HOSTING.md`](docs/SELF-HOSTING.md)
- Native War3 client（Windows exe）不送 Origin header → 不受影響，繼續連線

### For contributors
- Client `mpsc::UnboundedSender<ClientMessage>` 包成 `CmdSender` newtype，型別系統強制所有 cmd 走 `send_or_warn`（含 logging + `#[must_use]`），消除未來 call site 直接 `.send()` 繞過 logging 的 regression 風險（#23 silent-drop 第三層防線、#32）
  - **Migration**：`try_send_cmd(&self.cmd_tx, msg, label)` → `self.cmd_tx.send_or_warn(msg, label)`，message format 完全相同（`"{label} 未送出：背景任務已中斷（...）"`），UX 不變
- 新增 [`quality/backlog-audit.md`](quality/backlog-audit.md) staleness review 協議：machine-parseable `<!-- audit-v1 -->` 格式 + `decision: keep-open | close-as-obsolete | close-as-superseded` enum + escalation 規則（連續兩次無變化升級 close）。每個 minor release cycle 對 deferred issues 跑一輪審查
- Workspace `version.workspace` 改為 `0.4.1`（先前 `[workspace.package].version` 停在 `0.1.0` 導致 client binary 自報版本錯誤）

### Deferred (trigger 條件未滿足，audit comments 留 trail keep-until 2026-11-17)
- #33 `AppConfig::normalize()` 擴展到 nickname/server_url — when threat model 加入 config tampering（`local_ip` 欄位於後續清理中已移除）
- #35 `LogPanel::set_max_entries` runtime live-resize — when 使用者抱怨「重啟才生效」
- #36 `cleanup_old_logs` N+1 防護 — when log_dir 改可指定任意路徑，或啟動慢投訴出現
- #37 `logging.rs` 拆檔 — when file/layer 加 rotation/compression/metric，或行數 > 500

### Note
- Tag `v0.4.0` 已於 2026-04-07 由 PR #18 使用（orphan tag — release binary 仍在 GitHub Releases）。本次直接跳到 `v0.4.1`，GitHub milestone 也同步 rename。

## [0.3.6] - 2026-05-16

### Fixed
- 拔網路或 server 重啟瞬間按「加入」/「建立房間」不再 spinner 卡死：transport 失敗時把進行中操作換成紅色 banner「與伺服器中斷連線，操作已取消」+ 確定按鈕（#40）
- 斷線後 lobby 不再保留 stale 玩家/房間列表，避免使用者點 stale 房間又卡 Joining（#40）
- server 拒絕原因（暱稱超長、版本不符、房間已存在、Join 冷卻等）原本只寫進 log、UI 沒提示，現在顯示為紅 banner「錯誤：{message}」（#41）
- 重連完成後 stale 錯誤 banner 自動消失，避免「● 已連線」+ 紅 banner「中斷連線」自相矛盾
- queued `JoinRoom` 在 reconnect 後送達 server 不再造成「已取消 banner + 自動進遊戲」鬼影：JoinResult handler 加 state guard，pending 非 Joining 時略過 tunnel 啟動（#44）

### Changed
- 斷線取消訊息統一為「與伺服器中斷連線，操作已取消」
- `ServerMessage::Error` 處理改為 state-aware：Joining 中 → JoinFailed banner（語意一致）；CreatingRoom / 無 pending → ServerError banner；JoinSuccess / 已存在錯誤 → 只 log 不覆蓋（避免吃掉成功狀態或無聲蓋掉前一則錯誤）

### For contributors
- `PendingAction` enum 加 `ServerError { message }` variant + `is_in_flight()` helper 消重複 `matches!`
- 引入 AI ET pre-flight pattern：對 ET charter 先跑 AI agent code-trace static analysis，HIGH 信心發現直接開 issue 不必等人類 ET 驗證（#42、charter `quality/et-sessions/2026-05-16-*.md`）
- 三個 fix 都經 multi-agent review（reuse + quality + efficiency / correctness + race / UX 三視角並行），P0 findings 整合進同一 PR，P0-deferred 開為 follow-up issue（#44 即是這個流程的產物）

## [0.3.5] - 2026-05-16

### Fixed
- 按「建立房間」/「加入」/「關閉房間」等按鈕不再有「沒反應」的情況：背景任務斷線時 client 會在 log panel 顯示 warn 並清除 pending 狀態（#23）

### Added
- 設定頁可調 log 緩衝大小 1000-5000 筆（滑桿，500 步進，重啟生效）（#20）

### Changed
- CLAUDE.md「Server 安全模型」段補完：CORS 真實覆蓋範圍（WebSocket 不走 preflight）、rate limit 真實上限（per-IP 100 msg/s 穩態、邊界期 200 msg/60ms）、X-Real-IP 信任邊界部署不變式；糾正 `/ws` 連線數從過時的 3 為 10（#24, #25）

### For contributors
- 日誌系統補齊 9 個 unit/integration test：ring buffer overflow、filter、search、file layer、log dir 失敗 fallback、buffer size clamp 等（#19）
- `setup_file_writer` + `default_log_dir` 從 `main.rs` 移到 `logging.rs`，加 `log_dir` / `keep_files` 參數方便測試；log 檔名加毫秒精度避免同秒重啟覆寫（#19/#20）
- `LOG_BUFFER_*` 常量從 `ui::log_panel` 移到 `config`，修正反向依賴
- `AppConfig::normalize()` 在 load 時 clamp 並 `tracing::warn`，避免下游每次都防禦
- 修 rust 1.95.0 `clippy::collapsible_match` 在 `protocol/messages.rs::validate()`（#31）
- 品質系統第二輪：新增 D-DOC 類別（文件與實作不一致），寫 2 個 ET charter（network disruption、rapid input），整理 6 個 review 跳過項為 follow-up issues #32-#37（#38）

## [0.3.4] - 2026-04-07

### Fixed
- CJK 暱稱/房名/地圖名長度驗證改用字元數，中文玩家不再被誤拒（#22）
- JoinRoom TOCTOU 競態條件：房間人數檢查與遞增合併為原子操作，避免超額加入（#21）

## [0.3.3] - 2026-04-06

### Changed
- 全域暗色主題：配色與 web viewer 一致（#1a1a2e / #16213e / #3b82f6）
- 房間列表改為卡片式佈局：房主名大字粗體 + 地圖名小字灰色 + 人數 pill badge（綠/黃/紅）
- 自己的房間用藍色邊框標示，滿房顯示停用的「已滿」按鈕
- 日誌面板配色融入暗色主題，時間戳改為低調灰色
- 狀態列、連線 overlay、pending action banner 配色統一
- section header（房間列表、線上玩家、日誌）改為小字灰色風格
- 延遲顯示改用繁體中文（直連/中繼）
- 全域字型大小 14→15px，間距微調
- 截斷過長的房主名和地圖名，hover 顯示全文

### Fixed
- 滿房的房間不再顯示可點擊的「加入」按鈕

## [0.3.2] - 2026-04-06

### Added
- 首次啟動說明頁：在設定精靈前加一頁，說明 Windows 防火牆、UPnP、P2P 直連、SmartScreen 警告
- 設定頁「網路說明」區塊：可摺疊的防火牆/UPnP/P2P 說明，讓使用者事後重新查看
- 設定精靈「上一步」按鈕：可從暱稱設定頁回到說明頁
