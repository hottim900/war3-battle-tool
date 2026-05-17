<!-- Adapted from hottim900/quality-tracker -->
# 品質發現策略

> **用途：** 定義品質系統的雙層發現模型 — AI 執行層（搜查手冊 grep）與人類判斷層（探索式測試 ET）。
> 說明何時該用哪一層、兩層之間如何互饋，以及什麼問題只有人類才能發現。

---

## 雙層模型

品質發現分為兩層，各自覆蓋不同類型的問題：

```
┌─────────────────────────────────────────────────┐
│              品質發現策略                         │
│                                                   │
│  AI 執行層（搜查手冊）    人類判斷層（ET）         │
│  ─────────────────────   ─────────────────────    │
│  已知模式 → grep 搜查     未知模式 → 探索式測試   │
│  可自動化、可重複         需要判斷力、創造力       │
│  輸出：Issue + 搜查紀錄   輸出：Issue + 新 pattern │
│                                                   │
│            ◄── 互饋迴圈 ──►                       │
│  ET 發現新 pattern → 加入搜查手冊                 │
│  搜查手冊的盲區 → 產生 ET charter                 │
└─────────────────────────────────────────────────┘
```

**AI 執行層** — 搜查手冊（[defect-taxonomy.md](./defect-taxonomy.md)）中的 grep pattern 能系統性地找出已知缺陷模式。這一層的強項是覆蓋率和可重複性：跑一次就掃完整個 codebase，不遺漏。

**人類判斷層** — 探索式測試（Exploratory Testing, ET）處理搜查手冊無法覆蓋的問題。這些問題需要業務脈絡、使用者意圖判斷、或跨功能推理，不是 regex 能捕捉的。

兩層之間的互饋迴圈是系統的核心價值：
- ET session 發現的新 pattern，若符合[推廣標準](#pattern-推廣標準)，加入搜查手冊成為 grep pattern
- 搜查手冊每個類別的「盲區」（grep 搜不到的東西），產生 ET [charter seed](./defect-taxonomy.md)，指引下一次 ET session

---

## 末端問題 — grep 永遠找不到的東西

有四類問題本質上無法被自動化搜查發現。這些是 ET 的核心領域：

| 類型 | 說明 | 範例 |
| ---- | ---- | ---- |
| **業務邊界條件** | 業務規則在極端情況下的行為 | 使用者封存最後一個分類時，系統該怎麼辦？ |
| **意圖判斷** | 程式碼行為與設計意圖的偏差，需要理解「本來想做什麼」 | 排序演算法在相同分數時的順序是否符合使用者預期？ |
| **生產環境獨特性** | 只在真實環境才會出現的問題 | 高峰期並發存取、第三方 API timeout、時區邊界 |
| **Charter 定義** | 「我們該對什麼感到可疑」這個問題本身 | 新功能上線後，什麼樣的使用場景最可能出問題？ |

這不是說 grep 沒用。grep 處理它擅長的部分（已知模式），ET 處理 grep 處理不了的部分（需要判斷力的問題）。兩者互補，不是替代。

---

## 何時用哪一層？

```
發現了一個可疑的地方？
├── 能用 regex 表達嗎？
│   ├── 是 → 先查搜查手冊是否已有對應類別
│   │   ├── 有 → 用搜查手冊（AI 執行層）
│   │   └── 無 → 新增類別到搜查手冊，再執行搜查
│   └── 否 → 這是 ET 的領域
│
想主動找問題？
├── 剛完成搜查手冊某類別的掃查？
│   └── 是 → 該類別的 charter seed 就是下一步
├── 有 root-cause:design 的 Issue？
│   └── 是 → 設計缺陷 grep 找不到，用 ET 探索
├── 有 escape:production 的 Issue？
│   └── 是 → 自動化管道漏掉了，用 ET 探索
├── 剛上線新功能但沒有對應的搜查覆蓋？
│   └── 是 → 用 ET 探索新功能的邊界
└── 同區域短期內出現 2+ 個 Issue？
    └── 是 → 系統性問題，用 ET 探索根因
```

完整的 5 個 ET 觸發信號和操作流程見 `/quality` skill（SKILL.md）。

---

## 互饋迴圈

### 方向 1：ET → 搜查手冊

ET session 發現的問題，如果符合[推廣標準](#pattern-推廣標準)，應該被加入搜查手冊。這樣下次就不需要人類判斷，grep 就能自動找到。

### 方向 2：搜查手冊 → ET

搜查手冊每個 D-XXX 類別有一個「What grep can't find (Charter seed)」段落。這些 charter seed 描述了該類別的 grep pattern **搜不到**的具體問題，是 ET session 的起點。

Charter seed 不是通用的「多做一些測試」。它們指出具體的盲區：哪些業務規則、跨功能互動、或隱含假設是 grep 覆蓋不到的。

### Pattern 推廣標準

ET 發現要成為搜查手冊的新 grep pattern，必須通過三個檢查：

1. **可 regex 化？** — 能表達為 regex 嗎？如果不行，它留在 charter seed（未來 ET 的素材）
2. **精確度夠？** — 在 codebase 上執行時，false positive 低於 20% 嗎？
3. **跨專案有用？** — 對使用相同技術棧的其他專案也適用嗎？

三項都通過 → 加入 D-XXX 類別的搜查方式。任一項不通過 → 記錄在 charter seed 的「What grep can't find」段落。

---

## ET 方法論

本系統的 ET 方法基於 Session-Based Test Management (SBTM)，為單人/小團隊 + AI 開發場景簡化：

- **Charter** — 一份輕量計畫，用 4T 欄位（Target, Task, Timebox, Trigger）定義探索範圍
- **Session** — 一段有時間限制的聚焦探索（預設 30 分鐘）
- **Session Report** — 記錄發現、時間分配、新 pattern

Charter 模板和使用說明見 [et-charter-template.md](./et-charter-template.md)。

---

## 詞彙表

| 術語 | 定義 |
| ---- | ---- |
| **Charter Seed** | 搜查手冊中每個 D-XXX 類別的「What grep can't find」段落。描述 grep 的盲區，作為 ET charter 的起點。 |
| **Negative Space** | 搜查手冊 grep pattern 能搜到的東西，隱含定義了它搜不到的東西。這個「搜不到的空間」就是 negative space，也就是 ET 該去探索的地方。 |
| **4T 欄位** | Charter 的四個結構欄位：Target（探索目標）、Task（做什麼）、Timebox（時間限制）、Trigger（觸發原因）。 |
| **Pattern 推廣** | 將 ET 發現的問題轉化為搜查手冊的 grep pattern 的過程。需通過三項標準（可 regex 化、精確度、跨專案適用性）。 |
| **雙層模型** | 品質發現的兩個互補層：AI 執行層（grep 搜查）和人類判斷層（ET 探索）。 |
| **發現策略** | 系統性地決定「用什麼方法找問題」的框架。不只是找 bug，而是知道什麼方法適合找什麼類型的問題。 |
| **末端問題** | 本質上無法被自動化發現的四類問題：業務邊界條件、意圖判斷、生產環境獨特性、charter 定義。 |

---

## 方法論來源

| 來源 | 採用的概念 |
| ---- | ---- |
| Cem Kaner（ET 創始者, 1984） | 探索式測試的定義：同時設計、執行測試並從中學習 |
| James Bach（SBTM） | Session-Based Test Management：charter + session + debrief 結構 |
| Context-Driven School | 測試方法應隨情境調整，沒有「最佳實踐」只有「適合的實踐」 |

> 完整的方法論來源和設計決策見 [quality-system-design-notes.md](./quality-system-design-notes.md)。

---

## See also

- [et-charter-template.md](./et-charter-template.md) — Charter 模板與 Quick Start
- [defect-taxonomy.md](./defect-taxonomy.md) — 搜查手冊（含各類別的 charter seed）
- [backlog-audit.md](./backlog-audit.md) — Deferred items 處理協議（發現後不立即修的 staleness review）
- [README.md](./README.md) — 品質管理追蹤總覽
- [quality-system-design-notes.md](./quality-system-design-notes.md) — 設計筆記與方法論來源
- [examples/sparkle/et-charters.md](../examples/sparkle/et-charters.md) — 真實 ET 執行範例
