<!-- Adapted from hottim900/quality-tracker -->

# 品質管理追蹤

war3-battle-tool 的品質管理體系。追蹤缺陷、技術債、功能缺口、測試覆蓋與工具建設，並維護品質防線。

透過 GitHub Issues 管理品質項目（`#N` 格式引用），搭配 [defect-taxonomy.md](./defect-taxonomy.md) 搜查手冊進行系統性缺陷掃查。

---

## 快速查詢

> 品質項目以 Issue 追蹤，透過 label 過濾取得即時結果。

| 查詢                 | 指令                                                                         |
| -------------------- | ---------------------------------------------------------------------------- |
| 活躍項目             | `gh issue list --label "type:defect" --state open`                          |
| Critical/High 活躍   | `gh issue list --label "priority:critical" --state open`                    |
| In Progress 項目     | `gh issue list --label "status:in-progress" --state open`                   |
| Blocked 項目         | `gh issue list --label "status:blocked-by-decision" --state open`           |
| 搜查進度             | 見 [defect-taxonomy.md 分類總覽](./defect-taxonomy.md#分類總覽)              |

---

## 分類體系

| 分類             | 定義                         | 處理策略                | Label              |
| ---------------- | ---------------------------- | ----------------------- | ------------------ |
| **Defect**       | 非預期的錯誤，寫入時就是錯的 | 立即修復 + 回溯流程漏洞 | `type:defect`      |
| **Tech Debt**    | 有意識的妥協，先上線再改     | 排優先級，安排容量      | `type:tech-debt`   |
| **Feature Gap**  | 功能不完整，缺少預期互動     | 放進 backlog            | `type:feature-gap` |
| **Test Infrastructure** | 測試覆蓋缺口與測試工具建設 | 排優先級，系統性補齊 | `type:test-infra` |

### 如何判斷分類？

```
這個問題是有意識的妥協嗎？
├── 是 → 妥協的是測試覆蓋或測試工具嗎？
│   ├── 是 → Test Infrastructure
│   └── 否 → Tech Debt
└── 否 → 程式碼行為與設計意圖一致嗎？
    ├── 否 → Defect
    └── 是 → 功能設計完整嗎？
        ├── 否 → 缺少的是測試覆蓋嗎？
        │   ├── 是 → Test Infrastructure
        │   └── 否 → Feature Gap
        └── 是 → 不需要追蹤
```

---

## 定義參考

### 優先級

| 優先級       | 定義                       | 處理時機              |
| ------------ | -------------------------- | --------------------- |
| **Critical** | 影響生產穩定性或安全性     | 立即處理（同日）      |
| **High**     | 阻礙開發效率或造成頻繁 bug | 下個 Sprint（1-2 週） |
| **Medium**   | 增加維護成本但不影響功能   | 規劃中處理（1 個月）  |
| **Low**      | 改善開發體驗，非必要       | 有空時處理（無 SLA）  |

### 嚴重度（Defect 專用）

| 嚴重度          | 定義                        |
| --------------- | --------------------------- |
| **S1-Critical** | 系統不可用或資料遺失        |
| **S2-Major**    | 功能異常，無合理 workaround |
| **S3-Minor**    | 功能異常，有 workaround     |
| **S4-Trivial**  | 外觀/文字問題               |

### 根因類別（Defect 專用）

| 根因                       | 定義                         |
| -------------------------- | ---------------------------- |
| **Design Defect**          | 架構/設計層面的錯誤決策      |
| **Implementation Error**   | 實作與設計意圖不符           |
| **Configuration Omission** | 配置遺漏                     |
| **Framework Limitation**   | 框架已知限制未規避           |
| **Missing Test Coverage**  | 缺少測試導致未發現           |

---

## Label 參考

| Prefix | 用途 | 必填？ | 值 |
| ------ | ---- | ------ | -- |
| `type:` | 項目類型 | **必填** | `defect` / `tech-debt` / `feature-gap` / `test-infra` |
| `priority:` | 優先級 | **必填** | `critical` / `high` / `medium` / `low` |
| `status:` | 細分狀態 | 選填 | `in-progress` / `blocked-by-decision` |
| `severity:` | 嚴重度（Defect） | 選填 | `s1-critical` / `s2-major` / `s3-minor` / `s4-trivial` |
| `cost:` | 成本 | 選填 | `s` / `m` / `l` / `xl` |
| `escape:` | 逃逸階段（Defect） | 選填 | `code-review` / `unit-test` / `integration-test` / `e2e-test` / `production` |
| `root-cause:` | 根因（Defect） | 選填 | `design` / `implementation` / `configuration` / `framework` / `test-coverage` |
| `discovery-method:` | 發現方式 | 選填 | `taxonomy-sweep` / `et-session` / `code-review` / `production` |

---

## 待追蹤發現

搜查中發現但尚未建立正式項目的問題。

> **AI 行動指引：** 此段落僅供參考。**不要主動升級為正式項目** — 由人類決定何時建立。

（尚無待追蹤項目）

---

## 建立新項目

1. 用[分類決策樹](#如何判斷分類)判斷類型
2. 用對應的 Issue 模板建立 Issue：
   - `gh issue create --template defect.yml`
   - `gh issue create --template tech-debt.yml`
   - `gh issue create --template feature-gap.yml`
   - `gh issue create --template test-infra.yml`
3. 填寫模板中的所有欄位，加上 `priority:` label
4. 若 Defect → 在 Issue body 填寫「缺陷子類別」連結搜查手冊
5. 開始處理時，加上 `status:in-progress` label

---

## 完成步驟

> **IMPORTANT:** 修復完成後，依序執行以下步驟。缺任何一步 = 未完成。

1. 關閉 Issue，填寫完成 comment：
   ```
   ## 完成紀錄
   **Commit/PR：** abc1234 或 PR 連結
   **修改摘要：** 簡述實際修改
   **測試結果：** X passed, 0 failed
   ```
2. 若有相依 Issue → 檢查對方 Issue 是否需更新
3. 若為 Defect 且在系統性搜查中發現 → 確認搜查結果已記錄於 [defect-taxonomy.md](./defect-taxonomy.md)

---

## See also

- [discovery-strategy.md](./discovery-strategy.md) — 雙層品質發現模型
- [et-charter-template.md](./et-charter-template.md) — 探索式測試 Charter 模板
- [defect-taxonomy.md](./defect-taxonomy.md) — 缺陷分類學搜查手冊
- [backlog-audit.md](./backlog-audit.md) — Deferred issues staleness 審查協議
- [quality-system-design-notes.md](./quality-system-design-notes.md) — 設計筆記與方法論來源
