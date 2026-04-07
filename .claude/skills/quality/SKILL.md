---
name: quality
description: Quality tracking system operations guide. Use when fixing bugs, managing defect/tech-debt/feature-gap/test-infra items, or running code quality audits.
user-invocable: true
---

# 品質管理追蹤系統

## 初始設定（首次使用時修改一次）

將下方 `QUALITY_DIR` 改為你的品質系統**絕對路徑**：

```
QUALITY_DIR=/home/tim/side-projects/war3/quality
```

## 快速操作

```bash
# 列出所有活躍 Defect
gh issue list --label "type:defect" --state open

# 列出所有活躍 Tech Debt / Feature Gap / Test Infrastructure
gh issue list --label "type:tech-debt" --state open
gh issue list --label "type:feature-gap" --state open
gh issue list --label "type:test-infra" --state open

# 列出正在處理中的項目
gh issue list --label "status:in-progress" --state open

# 列出 Critical/High 項目
gh issue list --label "priority:critical" --state open
gh issue list --label "priority:high" --state open

# 列出等待決策的項目
gh issue list --label "status:blocked-by-decision" --state open
```

### 建立新項目

依照 README.md「建立新項目」段落的完整步驟操作（路徑：`${QUALITY_DIR}/README.md`）。

簡要流程：

1. 判斷類型（見 `${QUALITY_DIR}/README.md` 的「如何判斷分類」決策樹）— Defect / Tech Debt / Feature Gap / Test Infrastructure
2. 用對應的 Issue 模板建立 Issue：
   - `gh issue create --template defect.yml`
   - `gh issue create --template tech-debt.yml`
   - `gh issue create --template feature-gap.yml`
   - `gh issue create --template test-infra.yml`
3. 加上 `priority:` label
4. 若 Defect → 在 Issue body 填寫「缺陷子類別」連結搜查手冊

### 修復完成後

> **IMPORTANT:** 嚴格執行完成步驟（見 `${QUALITY_DIR}/README.md` 的「完成步驟」段落），缺任何一步 = 未完成。

完成步驟適用於所有類型：
1. 關閉 Issue + 填寫完成 comment（Commit/PR、修改摘要、測試結果）
2. 若有相依 Issue → 檢查對方 Issue 是否需更新
3. 若 Defect 且為系統性搜查中發現 → 確認搜查結果已記錄於 taxonomy

---

## 搜查手冊

系統性搜查工具，定義已知缺陷類別。每個類別有：

- **定義**：什麼模式構成此類缺陷
- **搜查方式**：可執行的 grep/搜查指令
- **搜查結果**：範圍與命中數、發現、low-risk observations、判定合理

執行搜查時，讀取 `${QUALITY_DIR}/defect-taxonomy.md` 取得每個類別的具體搜查指令。

搜查完成後，檢查該類別是否有 charter seed（「What grep can't find」段落）。如果有，向使用者建議 ET session — 見下方「探索式測試 (ET)」章節。

---

## 探索式測試 (ET)

搜查手冊覆蓋已知的 grep 可搜模式。ET 處理 grep 搜不到的問題：業務邊界、意圖判斷、跨功能互動。

完整方法論見 `${QUALITY_DIR}/discovery-strategy.md`。Charter 模板見 `${QUALITY_DIR}/et-charter-template.md`。

### 何時建議 ET session

| # | 觸發條件 | 信號 | 建議的 Charter |
|---|----------|------|----------------|
| 1 | **Post-sweep** | 剛完成某 D-XXX 類別的搜查 | 該類別的 charter seed |
| 2 | **Design defect** | Issue 有 `root-cause:design` label | 探索同一設計決策影響的其他功能 |
| 3 | **Production escape** | Issue 有 `escape:production` label | 探索自動化管道漏掉的同類場景 |
| 4 | **New feature** | 使用者上線了新功能 | 探索邊界條件和錯誤處理路徑 |
| 5 | **Pattern repeat** | 同區域短期內出現 2+ 個 Issue | 探索系統性根因 |

### ET session 紀錄

ET session 紀錄存為 markdown 檔案在 `${QUALITY_DIR}/et-sessions/` 目錄。不使用 GitHub Issues。

---

## 行為準則

- **修復 bug 時**：檢查是否有對應的品質追蹤 Issue。若無且是系統性問題 → 建議建立（但由人類決定）。
- **發現新問題時**：記錄到 README「待追蹤發現」段落。**不要主動升級為正式項目**。
- **搜查手冊中發現同類問題時**：記錄到搜查手冊的「搜查結果」中。
- **完成修復後**：嚴格執行「完成步驟」，不要遺漏任何一步。
