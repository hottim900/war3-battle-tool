# 缺陷分類學 — 系統性搜查手冊（Rust 適配版）

> **用途：** 定義已知和潛在的缺陷類別，提供可重複執行的搜查模式，讓開發者能快速找出同類問題。
> 每個類別附帶定義、搜查方式、和搜查結果記錄。

**建立日期：** 2026-04-07
**最後更新：** 2026-04-07

> 優先級、嚴重度、成本定義見 [README.md](./README.md#定義參考)

### 搜查結果記錄格式

每個類別搜查完畢後，記錄以下內容：

1. **搜查範圍與命中數** — 例如「`crates/server/` + `crates/client/` + `crates/protocol/` 全部 `.rs`，共 N 個命中」。這是下次搜查的**基線**。
2. **發現（建 Issue）** — 確認的缺陷，已建立追蹤 Issue。格式：`#N — 描述`
3. **Low-risk observations（不建 Issue）** — 可疑但影響太低，不值得正式追蹤。記錄檔案位置和原因。
4. **審查但判定合理（非缺陷）** — 經審查排除的項目。記錄判定理由，避免下次重複審查。

### 分類策略

每個類別的 grep 命中數會有差異。使用以下分類原則：

- **D-SILENT 分類：** 建 Issue 如果 unwrap/ok/let_ 在網路 I/O 路徑或訊息處理器中。啟動初始化或 CLI 解析中的屬於 low-risk。
- **D-AUTH 分類：** 建 Issue 如果端點缺少 rate limiting 或輸入驗證。內部使用或已有緩解措施的屬於 low-risk。
- **D-PERF 分類：** clone() 和 allocation patterns 命中數會很高。只建 Issue 給 hot path 中的問題（async loop、每幀執行的 UI code）。一次性初始化的 clone/allocation 記錄為「判定合理」。
- **D-RACE 分類：** spawn() 命中：只建 Issue 給 JoinHandle 被丟棄且沒有 abort() 或 select! 的情況。`tokio::select!` 搭配 JoinHandle arm 是結構化併發（安全），不需要追蹤。Arc<> 命中：只建 Issue 給 Arc 在 async await 點跨越迴圈的情況。

**搜查上限：** 每次搜查最多建立 5 個 Issues。超出的發現記錄在對應類別的「## Pending Triage」段落。

> **Claude Code 提示：** 以下各類別的 grep 指令為搜查邏輯的參考寫法。在 Claude Code 中請用內建 Grep 工具執行，`--include` 對應 `glob` 參數。

---

## 分類總覽

| 代號     | 缺陷類別             | 層級          | 搜查狀態 |
| -------- | -------------------- | ------------- | -------- |
| D-SILENT | 靜默失敗與錯誤吞沒   | 全層          | 待搜查   |
| D-VALID  | 輸入驗證缺口         | Protocol/API  | 待搜查   |
| D-AUTH   | 認證、授權與安全防線 | Server        | 待搜查   |
| D-TYPE   | 型別安全漏洞         | 全層          | 待搜查   |
| D-PERF   | 效能問題             | 全層          | 待搜查   |
| D-EDGE   | 邊界條件與資源限制   | 全層          | 待搜查   |
| D-RACE   | 競態條件與並發問題   | Server/Client | 待搜查   |
| D-DOC    | 文件與實作不一致     | 全層          | 待搜查   |

---

## D-SILENT: 靜默失敗與錯誤吞沒

### 定義

unwrap()/expect() 導致 panic、.ok() 靜默丟棄 Result 的 Err、let _ = 靜默丟棄 Result/Option、未完成的 code paths。在面向玩家的工具中，panic = 閃退，靜默失敗 = 玩家看不到錯誤訊息。

### 搜查方式

```bash
# unwrap() / expect() — panic 源頭
grep -rn "\.unwrap()" . --include="*.rs" --exclude-dir=target --exclude-dir=tests | grep -v "#\[cfg(test" | grep -v "#\[test"
grep -rn "\.expect(" . --include="*.rs" --exclude-dir=target --exclude-dir=tests | grep -v "#\[cfg(test" | grep -v "#\[test"

# .ok() 靜默丟棄 Result 的 Err
grep -rn "\.ok()" . --include="*.rs" --exclude-dir=target

# let _ = expr; 靜默丟棄 Result/Option
grep -rn "let _ =" . --include="*.rs" --exclude-dir=target

# todo!() / unimplemented!() — 未完成的 code paths
grep -rn "todo!()\|unimplemented!()" . --include="*.rs" --exclude-dir=target

# match 中的 _ => {} 空 catch-all
grep -rn "_ => {}" . --include="*.rs" --exclude-dir=target
```

> **搜查策略：** 優先檢查 unwrap()/expect()，因為這是最直接的 panic 源頭。.ok() 和 let _ = 次之。

**搜查狀態：** 已搜查（2026-04-07）

### What grep can't find (Charter seed)

D-SILENT 的 grep pattern 能偵測 unwrap、.ok()、let _ =。它們**搜不到**：

- **錯誤處理的語意正確性：** ? 運算子有用，但回傳的錯誤是否包含足夠上下文？例如：連線失敗只回傳 "connection error" 而非包含 IP 和 port
- **try_send 的失敗影響：** mpsc channel 的 try_send 回傳 Err 時，被丟棄的訊息是否影響使用者？
- **async 中的隱含 panic：** tokio::spawn 內部的 panic 不會傳播到外層，只會 abort 該 task

Suggested Charter:
```
Target: 錯誤傳遞鏈 — 從 service 層到使用者面前
Task: 故意觸發各種網路失敗（斷線、timeout、server 重啟），
  追蹤錯誤訊息從產生到 UI 顯示的完整路徑，
  檢查使用者是否得到足夠資訊
Timebox: 30 min
Trigger: D-SILENT 搜查完成
```

### 搜查結果

**搜查範圍：** `crates/server/` + `crates/client/` + `crates/protocol/`（排除 tests/ 和 spike crates），共 ~80 個 unwrap/expect + ~30 個 let _ = 命中

**發現：**

- #23 — `app.rs:197,338,409` 的 `let _ = cmd_tx.send(...)` 靜默丟棄使用者命令（Register/CreateRoom）

**Low-risk observations：**

- `client/src/net/tunnel.rs` 多處 `let _ = event_tx.send(TunnelEvent::...)` — fire-and-forget 事件通知模式，channel 關閉時丟棄是預期行為
- `client/src/net/quic.rs:116` — `Arc::get_mut().unwrap()` — 在 config 建構時呼叫，此時 Arc 只有唯一 owner

**審查但判定合理：**

- `discovery.rs:95,130,137` — `serde_json::to_string(&msg).unwrap()` — 序列化已知 enum variant，不會失敗
- `main.rs:25,36` — tracing filter parse unwrap — 啟動初始化，固定字串
- `server/main.rs:68,74` — TcpListener::bind/serve unwrap — server 啟動必要步驟，失敗應立即 panic
- `client/main.rs:20,39,65` — rustls/runtime/eframe expect — 啟動必要步驟

---

## D-VALID: 輸入驗證缺口

### 定義

WebSocket 訊息處理缺少 size/format 驗證、serde deserialize 未驗證、數值 parse 未驗證範圍、字串長度用 bytes 而非字元數。

### 搜查方式

```bash
# WebSocket message 處理（是否有 size/format 驗證）
grep -rn "Message::Text\|Message::Binary" . --include="*.rs" --exclude-dir=target

# serde deserialize 是否有驗證
grep -rn "serde::Deserialize\|#\[derive.*Deserialize" . --include="*.rs" --exclude-dir=target

# 數值 parse 未驗證範圍
grep -rn "\.parse::<\|from_str" . --include="*.rs" --exclude-dir=target

# 字串長度驗證是否用 bytes 而非字元數（CJK 使用者會遇到）
grep -rn "\.len() > MAX_\|\.len() >= MAX_" . --include="*.rs" --exclude-dir=target
```

**搜查狀態：** 待搜查

### What grep can't find (Charter seed)

- **業務語意驗證：** 欄位有格式檢查，但值在遊戲語境合理嗎？例如：max_players 設為 0 或 999、room_name 全空白
- **跨欄位依賴：** war3_version 和 gameinfo 的相容性驗證

Suggested Charter:
```
Target: WebSocket 訊息驗證 — 惡意或異常輸入
Task: 送出格式正確但語意異常的 ClientMessage（超長暱稱、
  不存在的 room_id、重複 Register），觀察 server 反應
Timebox: 30 min
Trigger: D-VALID 搜查完成
```

### 搜查結果

（依[記錄格式](#搜查結果記錄格式)填寫）

---

## D-AUTH: 認證、授權與安全防線

### 定義

WebSocket 路由未保護、rate limit 不足或可繞過、IP 驗證漏洞、CORS 設定過寬、unsafe 使用。

### 搜查方式

```bash
# WebSocket 路由處理
grep -rn "\.route\|Router::new" . --include="*.rs" --exclude-dir=target

# rate limit 相關
grep -rn "rate\|limit\|throttle" . --include="*.rs" --exclude-dir=target

# unsafe 使用
grep -rn "unsafe " . --include="*.rs" --exclude-dir=target

# IP 驗證 / header 信任
grep -rn "X-Real-IP\|x-real-ip\|remote_addr\|peer_addr" . --include="*.rs" --exclude-dir=target

# CORS 設定
grep -rn "CorsLayer\|cors\|Access-Control" . --include="*.rs" --exclude-dir=target
```

**搜查狀態：** 已搜查（2026-04-07）

### What grep can't find (Charter seed)

- **rate limit 的正確性：** rate limiter 存在，但實作是否允許 burst？epoch boundary 是否有 2x burst 問題？
- **IP spoofing：** X-Real-IP header 是否只信任 loopback？
- **WebSocket impersonation：** `__web-viewer-` prefix 是否可被任意客戶端使用？

Suggested Charter:
```
Target: 安全防線 — rate limit 和 IP 驗證
Task: 模擬快速重連、burst messaging、IP header 偽造，
  驗證 rate limiter 和 per-IP 限制是否有效
Timebox: 30 min
Trigger: D-AUTH 搜查完成
```

### 搜查結果

**搜查範圍：** `crates/server/`（主要安全防線所在），共 ~15 個路由/rate/limit/CORS 命中

**發現：**

- #24 — `main.rs:54` CorsLayer::permissive() 安全模型未文件化
- #25 — ws.rs rate limiter token bucket 允許 epoch boundary 2x burst

**Low-risk observations：**

- `__web-viewer-` prefix 可被任意客戶端使用，但後果是自我限制（120s auto-disconnect），不構成安全威脅

**審查但判定合理：**

- `/ws`, `/tunnel`, `/health` 三個路由都有適當的 rate limiting 和 per-IP 連線限制
- X-Real-IP header 只在 loopback 連線時信任（`main.rs:82`），外部連線使用 peer_addr
- 零 `unsafe` 使用 — 全 codebase 沒有任何 unsafe 區塊

---

## D-TYPE: 型別安全漏洞

### 定義

as 強制轉型可能溢出、transmute 使用、手動 pointer 操作。Rust 的型別系統已消除大部分問題，此類別聚焦於顯式繞過型別安全的模式。

### 搜查方式

```bash
# as 強制轉型（可能溢出）
grep -rn " as u\| as i\| as f" . --include="*.rs" --exclude-dir=target --exclude-dir=tests | grep -v "#\[cfg(test" | grep -v "#\[test"

# transmute（極危險）
grep -rn "transmute" . --include="*.rs" --exclude-dir=target

# 手動 pointer 操作
grep -rn "\*const \|\*mut " . --include="*.rs" --exclude-dir=target
```

**搜查狀態：** 待搜查

### What grep can't find (Charter seed)

- **語意型別正確性：** 編譯通過但型別在業務上有誤。例如：player_id 和 room_id 都是 String，可能混用

Suggested Charter:
```
Target: 型別語意 — ID 和 String 的混用風險
Task: 追蹤 player_id 和 room_id 在整個流程中的使用，
  檢查是否有可能混用兩者導致邏輯錯誤
Timebox: 30 min
Trigger: D-TYPE 搜查完成
```

### 搜查結果

（依[記錄格式](#搜查結果記錄格式)填寫）

---

## D-PERF: 效能問題

### 定義

clone() 過度使用、blocking 呼叫在 async 中、不必要的 allocation。

### 搜查方式

```bash
# clone() 過度使用
grep -rn "\.clone()" . --include="*.rs" --exclude-dir=target --exclude-dir=tests | grep -v "#\[cfg(test" | grep -v "#\[test"

# 迴圈內的 allocations（手動輔助）
# 步驟：先找所有迴圈位置，再逐一人工檢查是否有不必要的 allocation
grep -rn "loop \{\|for .* in\|while " . --include="*.rs" --exclude-dir=target
# 也可用 cargo clippy 的 perf lints 輔助：clippy::perf 群組

# blocking 呼叫在 async 中
grep -rn "std::thread::sleep\|std::fs::" . --include="*.rs" --exclude-dir=target
```

> **分類策略：** clone() 和 allocation patterns 命中數會很高。只建 Issue 給 hot path 中的問題（async loop、每幀執行的 UI code）。一次性初始化的 clone/allocation 記錄為「判定合理」。

**搜查狀態：** 待搜查

### What grep can't find (Charter seed)

- **效能退化趨勢：** 5 個玩家時快，500 個玩家時 broadcast_state 的 O(n) clone 是否成為瓶頸？
- **Lock contention：** RwLock 在高並發下的等待時間

Suggested Charter:
```
Target: 高負載效能 — broadcast_state 在 500 玩家時的表現
Task: 模擬高頻率 room 變更（join/leave），觀察 broadcast
  延遲和 lock 等待時間
Timebox: 30 min
Trigger: D-PERF 搜查完成
```

### 搜查結果

（依[記錄格式](#搜查結果記錄格式)填寫）

---

## D-EDGE: 邊界條件與資源限制

### 定義

數值溢出風險、空集合未處理、timeout 設定不當。

### 搜查方式

```bash
# 數值溢出風險
grep -rn "as u8\|as u16\|as i8\|as i16" . --include="*.rs" --exclude-dir=target

# 空集合未處理
grep -rn "\[0\]\|\.first()\|\.last()" . --include="*.rs" --exclude-dir=target

# timeout 相關
grep -rn "timeout\|Duration::" . --include="*.rs" --exclude-dir=target
```

**搜查狀態：** 待搜查

### What grep can't find (Charter seed)

- **全域上限邊界：** 500 玩家 / 200 房間上限到達時的行為，是否有 graceful degradation？
- **時間邊界：** heartbeat timeout 和 cleanup interval 的交互，是否有 cleanup 把還活著的連線清掉的可能？

Suggested Charter:
```
Target: 資源限制邊界 — 全域上限行為
Task: 模擬接近 500 玩家上限、200 房間上限的情境，
  觀察新連線被拒絕時的錯誤訊息和恢復行為
Timebox: 30 min
Trigger: D-EDGE 搜查完成
```

### 搜查結果

（依[記錄格式](#搜查結果記錄格式)填寫）

---

## D-RACE: 競態條件與並發問題（自定義類別）

### 定義

RwLock/Mutex 使用不當、channel 通訊問題、Arc 共享狀態風險、tokio::select! 分支行為、spawn 後未 join。War3 大量使用 tokio async + mpsc + RwLock，這是最高風險區。

### 搜查方式

```bash
# RwLock / Mutex 使用
grep -rn "RwLock\|Mutex" . --include="*.rs" --exclude-dir=target
grep -rn "\.lock()\|\.read()\.await\|\.write()\.await" . --include="*.rs" --exclude-dir=target

# channel 使用
grep -rn "mpsc::\|channel()\|\.send(\|\.recv(" . --include="*.rs" --exclude-dir=target

# Arc 共享狀態
grep -rn "Arc::new\|Arc::clone\|Arc<" . --include="*.rs" --exclude-dir=target

# tokio::select! 分支
grep -rn "tokio::select!\|select!" . --include="*.rs" --exclude-dir=target

# spawn 後沒有 join
grep -rn "tokio::spawn\|thread::spawn" . --include="*.rs" --exclude-dir=target
```

> **分類策略：**
> - `tokio::select!` 搭配 JoinHandle arm 是結構化併發（安全），不需要追蹤。只標記沒有用 select! 管理的 fire-and-forget spawn。
> - spawn() 命中：只建 Issue 給 JoinHandle 被丟棄且沒有 abort() 或 select! 的情況。
> - Arc<> 命中：只建 Issue 給 Arc 在 async await 點跨越迴圈的情況。
> - 特別注意 TOCTOU 模式：check-then-act 跨越了 await point（如 read lock check + 後續 write lock update）。

**搜查狀態：** 待搜查

### What grep can't find (Charter seed)

D-RACE 的 grep pattern 能偵測 lock/channel/spawn 使用位置。它們**搜不到**：

- **跨 lock 的邏輯不一致：** 兩個 RwLock 之間的操作順序是否保證一致性？例如：register_token 寫 token_bindings 和 upnp_pending 是兩個獨立 write
- **TOCTOU 競態：** read lock 檢查條件後釋放，write lock 修改時條件可能已變。例如：JoinRoom 的 room_full 檢查
- **Channel backpressure：** mpsc channel 滿時的 try_send 失敗是否影響遊戲流程？
- **select! 取消安全性：** select! 取消一個 arm 時，被取消的 future 是否留下了不一致的狀態？

Suggested Charter:
```
Target: 並發狀態一致性 — RwLock 和 channel 的交互
Task: 追蹤 AppState 和 TunnelState 的所有 write 操作，
  檢查是否有跨 await point 的 check-then-act 模式，
  模擬高並發 JoinRoom 驗證 TOCTOU 風險
Timebox: 30 min
Trigger: D-RACE 搜查完成
```

### 搜查結果

（依[記錄格式](#搜查結果記錄格式)填寫）

---

## D-DOC: 文件與實作不一致

### 定義

CLAUDE.md、註解、PR description 等文件描述的行為與實作不符。包括：常數值過時（code 改了 doc 沒同步）、機制描述錯誤（如「CORS layer 保護 WebSocket」實際 WS 不走 CORS）、安全模型 caveat 未紀錄。

對使用者的影響通常**間接**：下個維護者照文件做決策、引入新 bug 或誤判風險。

### 搜查方式

```bash
# 常數值 vs 文件對齊：抓 code 內的數字 const，比對 doc 內出現的數字
grep -rn "const MAX_\|const [A-Z_]\+: u\?\(8\|16\|32\|64\|size\) = [0-9]" . --include="*.rs" --exclude-dir=target
# 然後在 CLAUDE.md / README.md 內查這些數字是否一致

# inline 註解標示的「TODO/FIXME/XXX」是否與目前 code 一致
grep -rn "TODO\|FIXME\|XXX\|HACK" . --include="*.rs" --include="*.md" --exclude-dir=target

# 「為了 X 才這樣做」類註解：審查 X 是否還成立
grep -rn "^[[:space:]]*//.*為了\|^[[:space:]]*//.*因為" . --include="*.rs" --exclude-dir=target
```

> **搜查策略：** 此類別的 grep 命中很多但 false positive 也多。建議 doc 改動 PR 觸發、或每季抽樣審 CLAUDE.md 內的具體數字 / 機制描述。

**搜查狀態：** 待搜查

### What grep can't find (Charter seed)

- **misleading 措辭**：doc 寫「CORS layer 保護 WebSocket」聽起來合理但實際無效；grep 抓不到語意層級
- **安全模型 caveat 缺漏**：例如「server bind 0.0.0.0 後 X-Real-IP 信任就是漏洞」這種部署 invariant
- **隱含假設過時**：code 改了 protocol，但文件還在描述舊 protocol

Suggested Charter:
```
Target: CLAUDE.md「Server 安全模型」段所述條目
Task: 逐項驗證——找出 code 對應位置，確認數字、機制、行為皆吻合。
  特別檢查 「per-IP 連線數」、「rate limit」、「CORS」、「X-Real-IP 信任邊界」。
Timebox: 45 min
Trigger: 重要 PR 修改安全相關 code 後
```

### 搜查結果

**搜查範圍：** PR #30 順手對 CLAUDE.md「Server 安全模型」段做了一次手動審查
**發現：**
- #24/#25 已修：CORS 對 WebSocket 是 no-op 的 misleading 描述 + rate limit 真實上限未量化 + per-IP 連線數過時（3 → 10）
- #34 新建：WebSocket Origin 驗證缺漏（從 CORS 文件化過程衍生發現）

---

## 搜查執行紀錄

| 日期 | 類別 | 命中數 | 發現/排除 | 備註 |
| ---- | ---- | ------ | --------- | ---- |
| 2026-04-07 | D-SILENT | ~110 | 1 發現 (#23), 2 low-risk, 7 判定合理 | 首輪搜查 |
| 2026-04-07 | D-AUTH | ~15 | 2 發現 (#24, #25), 1 low-risk, 3 判定合理 | 首輪搜查 |
| 2026-04-07 | D-RACE | — | 1 發現 (#21), 來自 autoplan eng review | 非 grep 搜查，autoplan 發現 |
| 2026-04-07 | D-VALID | — | 1 發現 (#22), 來自 autoplan eng review | 非 grep 搜查，autoplan 發現 |
| 2026-05-16 | D-DOC | — | 3 發現修復 (#24/#25 + CLAUDE.md 數字修正), 1 衍生 (#34) | PR #30 doc PR 過程手動審 |
| 2026-05-16 | follow-up | — | 6 個 follow-up issues 建立 (#32-#37) | PR #28/29/30 review skipped items 整理 |

---

## See also

- [discovery-strategy.md](./discovery-strategy.md) — 雙層模型：搜查手冊與 ET 如何互饋
- [et-charter-template.md](./et-charter-template.md) — Charter 模板與 Quick Start
- [backlog-audit.md](./backlog-audit.md) — Deferred items 處理協議（搜查找出來後的下一站）
- [README.md](./README.md) — 品質管理追蹤總覽

---

## 如何新增自定義類別

當你的專案有特定領域的缺陷模式時，可以擴充搜查手冊。

### 步驟

1. 在[分類總覽](#分類總覽)表格新增一行
2. 在本文件新增一個 `## D-XXXX: 類別名稱` 段落
3. 填寫：定義、搜查方式（具體的 grep 指令）、搜查狀態
4. 執行首次搜查，依[記錄格式](#搜查結果記錄格式)記錄結果
