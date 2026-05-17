<!-- Adapted from hottim900/quality-tracker -->
# 探索式測試 Charter 模板

> **用途：** 輕量的 SBTM (Session-Based Test Management) charter 模板。
> 5 分鐘寫完一份 charter，30 分鐘執行一次 session。
> 結果流入既有的 Issue 追蹤系統。

---

## Quick Start

3 步開始你的第一次 ET session：

1. **複製下方模板**，填入 4T 欄位
2. **設定 30 分鐘計時器**，開始探索
3. **記錄發現**，建 Issue（加上 `discovery-method:et-session` label）

不確定 charter 要寫什麼？從搜查手冊的 [charter seed](./defect-taxonomy.md) 開始。每個 D-XXX 類別都有一段「What grep can't find」，那就是最好的探索起點。

---

## Charter 模板（4T 欄位）

```markdown
## ET Charter: [標題]

**Target（目標）：** 要探索的系統區域或功能
**Task（任務）：** 具體要做什麼、要尋找什麼
**Timebox（時限）：** 30 min（預設，可依需要調整）
**Trigger（觸發）：** 什麼原因讓你決定做這次 ET

### Session Report

**日期：** YYYY-MM-DD
**實際耗時：** __ min（探索 __% / 調查 __% / 記錄 __%）

#### 發現

- [ ] Issue #__ — 描述

#### 新 Pattern 發現

- [ ] 可加入搜查手冊？（見 Pattern 推廣標準）

#### 筆記

自由記錄：觀察、疑問、後續想法
```

---

## 複製即用範例

以下是一份填寫完成的 charter，展示格式和內容的詳細程度：

```markdown
## ET Charter: 分類管理的業務邊界條件

**Target（目標）：** 分類的 CRUD 操作，特別是刪除和封存
**Task（任務）：** 探索「最後一個」的邊界：刪除最後分類、
  移動所有項目到另一個分類後的空分類、並發操作同一分類
**Timebox（時限）：** 30 min
**Trigger（觸發）：** D-EDGE 搜查完成，grep 已覆蓋 schema
  限制但業務規則邊界未測試

### Session Report

**日期：** 2026-04-01
**實際耗時：** 35 min（探索 60% / 調查 25% / 記錄 15%）

#### 發現

- [ ] #42 — 封存最後一個分類後，「新增項目」的預設分類指向
  已封存的分類，UI 無錯誤提示但 API 回傳 400

#### 新 Pattern 發現

- [ ] `archived` 狀態的 entity 被其他 entity 引用為預設值
  → 可能可以用 grep 搜查：找出所有引用 `default` + entity
  關聯的地方，檢查是否處理了 archived 狀態

#### 筆記

- 「最後一個」的邊界不只是空集合，還有「最後一個被引用的」
- 並發操作沒有在這次 session 中重現問題，可能需要更大的
  timebox 或不同的探索角度
```

---

## 好 Charter vs. 壞 Charter

| 好 Charter | 壞 Charter |
| ---- | ---- |
| Target 指向具體的功能或區域 | Target 是「整個應用程式」 |
| Task 描述具體的探索動作 | Task 是「找 bug」 |
| Trigger 連結到搜查手冊類別或具體事件 | 沒有 Trigger（隨意探索） |
| 30 min timebox | 無時限，做到累為止 |

---

## Session 結果如何流入追蹤系統

1. **發現的缺陷** → 建 Issue，加上 `discovery-method:et-session` label
2. **新的 grep pattern** → 評估是否符合[推廣標準](./discovery-strategy.md#pattern-推廣標準)，符合就加入搜查手冊
3. **Session 本身** → 建一個 Issue 記錄 session（標題如「ET Session: D-EDGE 業務邊界探索」），發現和筆記作為 comment。這樣所有 session 都有追蹤紀錄。

---

## 沒有發現怎麼辦？

沒有發現不代表 session 失敗。可能的情況：

- **系統在這個區域確實很穩健** — 記錄「未發現問題」和探索的範圍，這本身就是品質信號
- **Charter 的 scope 太窄** — 下次擴大 Target 或換一個角度
- **Charter 的 scope 太廣** — 30 分鐘不夠深入，下次縮小到更具體的功能
- **探索方向錯了** — 檢查 charter seed，是否有更高風險的盲區未探索

Session report 中記錄你探索了什麼、為什麼沒發現問題，這些資訊對未來的 charter 設計有價值。

---

## 技術棧適配

搜查手冊中的 charter seed 以 TypeScript/React 為範例。適配到你的專案時：

- 保留業務規則和跨功能互動的探索方向（這些是技術棧無關的）
- 將技術特定的範例替換為你的技術棧的等價概念（例如 Zod schema → FluentValidation，React state → Blazor component state）
- 完整的技術棧適配範例見 [examples/](../examples/)

---

## See also

- [discovery-strategy.md](./discovery-strategy.md) — 雙層模型與互饋迴圈
- [defect-taxonomy.md](./defect-taxonomy.md) — 搜查手冊（含各類別的 charter seed）
- [backlog-audit.md](./backlog-audit.md) — Deferred items 處理協議
- [README.md](./README.md) — 品質管理追蹤總覽
- [examples/sparkle/et-charters.md](../examples/sparkle/et-charters.md) — 真實 ET 執行範例
