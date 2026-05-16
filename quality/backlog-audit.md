# Backlog Audit Protocol

> **問題：** PR review 過程中常出現「現在不該做但值得記下來」的 observation——若全部 keep open，半年後 backlog 變成 ghost ticket 堆，沒人記得當初為什麼留著；若全部 close，下次 trigger 真的滿足時要從頭重發現。
>
> **解法：** 每個 deferred issue 在 body 寫明 `為什麼現在不做` + trigger condition；每個 minor release cycle 對這些 issues 做 staleness review，留 machine-parseable comment 記錄審查結果。

**建立日期：** 2026-05-17
**最後更新：** 2026-05-17

---

## 何時執行

每個 **minor release cycle**（例如 v0.3.x → v0.4.0）的 post-CHANGELOG drafting 階段，對符合審查標的的 issues 各加一條 audit comment。

可由 `/autoplan` 或 `/quality` skill session 觸發，或手動執行。

## 審查標的

同時符合下列條件的 open issues：

- Label 含 `priority:low`
- Body 含 `為什麼現在不做` 字串

> **格式契約：**
> - 新建立的 deferred issue **必須**在 **issue body**（不是 comment、不是 PR description）寫 `為什麼現在不做` 段落（繁中）。`gh issue list --search` 的 `in:body` qualifier 不掃 comments；放錯位置會 silently 漏掃。
> - 即使 issue 標題或主文是英文，marker 一律用繁中 `為什麼現在不做`。GitHub issue search 不支援 boolean `OR`（會被 silently ignored），本協議只認單一 marker。

```bash
# 取得本輪 release 應審查的 issues
gh issue list \
  --state open \
  --label "priority:low" \
  --search "為什麼現在不做 in:body" \
  --json number,title,body
```

## Comment 格式（machine-parseable）

每條 audit comment 以下列格式寫入，方便後續 tooling/AI 解析：

```
<!-- audit-v1 -->
audit-date: 2026-05-17
trigger-condition: {從 issue body 抄錄原始 trigger 文字}
current-status: {trigger 是否成立的一句話判斷}
decision: keep-open | close-as-obsolete | close-as-superseded
keep-until: 2026-11-17
audit-source: {planning doc 檔名或 manual-audit-YYYYMMDD}
```

> **解析契約：**
> - 標頭 marker 正規式：`<!--\s*audit-v1\s*-->`（容忍空白變體 `<!--audit-v1-->` / `<!--  audit-v1 -->`，但不接受 `<!-- audit-v1 final -->` 等加字尾）。
> - Key/value 分隔：每行第一個 `: `（冒號＋空格）為分隔；之後整段為 value，trim 行尾空白與 `\r`。
> - Keys 用英文小寫 + 連字號（穩定機器解析）；values 可用繁中（人讀脈絡）。
> - 每個欄位 value 必須**單行**——多行值會被 line-based parser silently truncate。長 context 拆成多條 audit comment。
> - `decision` value 必須完全符合三個 enum 字串之一（小寫、連字號）。`keepopen` / `close_as_obsolete` 等變體**不接受**。

### 欄位語意

| 欄位 | 必填 | 說明 |
|---|---|---|
| `audit-date` | ✅ | 審查當日的絕對日期（ISO-8601）。不寫相對日期。 |
| `trigger-condition` | ✅ | 從 issue body 抄錄原始 trigger 文字，便於 inline 對比。 |
| `current-status` | ✅ | 一句話描述 trigger 目前是否成立，附判斷依據（觀察值 / 使用者回饋 / threat model 變化）。 |
| `decision` | ✅ | 三擇一，見下節「Decision 判準」。 |
| `keep-until` | ✅ when `decision: keep-open` | 下一次回頭審查的截止日。**預設 = audit-date + 6 個月**（若有更明確 trigger 日期可改寫）。**目前是 review-by，不是 auto-close**——未來 `/autoplan` session 看到過期應重審或 close。 |
| `audit-source` | ✅ | 觸發本次審查的 planning doc 檔名（建立追溯鏈）。手動審查無 planning doc 時寫 `manual-audit-YYYYMMDD`。 |

## Decision 判準

### `keep-open`

Trigger condition 仍可能成立、未來仍與專案方向一致。預設選項。

例：`#36 cleanup_old_logs N+1 防護`——trigger 是「使用者投訴啟動慢」，目前無投訴管道；keep-open + keep-until = 2026-11-17。

### `close-as-obsolete`

Trigger condition 已不可能滿足，或專案方向變更導致此 issue 失去前提。**沒有後續 PR 接手**。

例：若 CommandPalette feature 整個被砍，「CommandPalette refactor」issue 直接 close-as-obsolete（假設性例子）。

### `close-as-superseded`

Trigger condition 已被另一個 PR 或 issue 整併解決。**與 obsolete 的差別**：obsolete 是「不再 relevant」，superseded 是「relevant 但已在別處做掉」——保留審查 trail 才能找到接手者。

選此 decision 時必須額外加 `superseded-by: #NN` 一行（指向接手的 issue/PR）。

## 為什麼用 HTML comment 而不是純 text key-value

`<!-- audit-v1 -->` 標頭讓 future tooling（grep / AI session / GitHub API filter）能可靠識別「這是 audit comment」而不是一般討論。`v1` 預留 schema 升級空間（例如未來加 `auto-close-on: 2026-11-17` 真的執行 close）。

## Keep-until 的執行模型

目前 `keep-until` 是 **review-by 日期**，不是 auto-close。意思是：

- ✅ 過期後下次 `/autoplan` session 看到，會把該 issue 重新列入審查列表，產生新一條 audit comment
- ❌ 目前沒有 cron job 自動 close 過期 issue

**Escalation 規則**：同一個 issue 連續兩次 audit（兩條 `<!-- audit-v1 -->` comments）都是 `decision: keep-open` 且 `current-status` 沒有實質變化（trigger 仍未滿足、無新觀察值），下一次 audit 必須升級成 `close-as-obsolete`。理由：超過一年沒進展 + 沒新訊息進來 = trigger 條件實際上不會發生，繼續 keep-open 就是 ghost ticket。

未來若需求增加，可加 GitHub Actions cron job 自動掃 `<!-- audit-v1 -->` + `keep-until` 過期 + `decision: keep-open` 但 audit-date 已超過 keep-until 的項目，發 stale notification 或自動 close。**今天先建立資料 schema 與 escalation 規則，執行邏輯未來再加。**

## 範例

`keep-open`（最常見）：

```markdown
<!-- audit-v1 -->
Reviewed 2026-05-17 during v0.4.1 audit cycle.

audit-date: 2026-05-17
trigger-condition: log_dir 改可指向任意路徑，或使用者投訴啟動慢
current-status: 預設 keep_files=30 穩態約 31 個 .log；無使用者投訴管道；log_dir 仍 hardcoded 在 default_log_dir() 不可外部指定
decision: keep-open
keep-until: 2026-11-17
audit-source: tim-master-design-20260516-231321.md
```

`close-as-superseded`（被別處整併解決）：

```markdown
<!-- audit-v1 -->
audit-date: 2026-05-17
trigger-condition: 出現第 6-7 個 cmd_tx call site 或實際看到 silent drop regression
current-status: codebase 已有 20+ 個 mpsc::send sites，trigger 已滿足；newtype 在 #32 完成
decision: close-as-superseded
superseded-by: #32
audit-source: tim-master-design-20260516-231321.md
```

## 與 quality system 的關係

- `defect-taxonomy.md` 是「**找出**新缺陷」的搜查手冊（grep-based 主動發現）
- `backlog-audit.md`（本文件）是「**處理**已知 deferred items」的審查協議（被動 staleness review）
- 兩者互補：前者源源不斷產生新 issues，後者讓既有 backlog 不會堆積成 ghost tickets

每次 `/quality` skill session 跑完搜查手冊建新 issues 後，若該 issue 是「deferred」性質（建立時就標 `priority:low` + 寫 `為什麼現在不做`），自動進入下次 minor release cycle 的本協議審查範圍。
