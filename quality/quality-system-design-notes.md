<!-- Adapted from hottim900/quality-tracker -->
# 品質管理追蹤體系設計筆記

> 可移植到任何專案的品質管理追蹤方法論。
> 基於 ODC、DORA、GitHub Label Taxonomy 等業界實踐，
> 針對「單人/小團隊 + AI 開發」場景優化。

---

## 1. 設計原則

### 1.1 核心目標

為 **AI 輔助開發** 設計的品質追蹤系統，最高優先級是：

1. **AI 可自主操作** — 每個工作流步驟都有明確指引，不依賴隱性知識
2. **單一來源** — 每筆資料只存在一處，避免多處同步導致不一致
3. **可擴展** — 從 8 項到 800 項，結構不需改變

### 1.2 設計取捨

| 決策             | 選擇                          | 放棄                | 理由                             |
| ---------------- | ----------------------------- | ------------------- | -------------------------------- |
| 追蹤機制         | GitHub/GitLab Issues + Labels   | Markdown 追蹤檔     | Issue tracker 原生支援搜尋、過濾、assign、自動 close、Board |
| Dashboard 策略   | Issue Board + Label 過濾        | 手動維護統計/表格   | 零同步，Issue 狀態為 source of truth |
| 發現機制         | `gh/glab issue list` + label 過濾 | `glob` + `grep`   | 語意查詢，不依賴檔案系統結構        |
| 模板分類         | 四種 Issue 模板（YAML forms / markdown） | 一個模板 + 條件欄位 | GitHub YAML forms 提供下拉選單驗證   |
| Metadata 編碼    | Label prefix（`type:` / `priority:` / `severity:`） | Markdown 表格欄位 | 可搜尋、可過濾、可統計              |
| 優先級 vs 嚴重度 | 分離為兩組 label                | 合併為一個          | 「何時修」和「多嚴重」是不同維度 |
| 內容重複         | canonical location + Issue 交叉引用 | 每處都完整記載  | 避免多處更新造成不一致           |

---

## 2. 分類體系

### 2.1 五分類 + 決策樹

完整決策樹見 [README.md 分類體系](./README.md#分類體系)。

第四類 **Test Infrastructure** 追蹤測試覆蓋缺口與測試工具建設 — 與 Defect/Tech Debt 不同，TI 項目的產出是「新增測試」而非「修改產品程式碼」。決策樹中 TI 透過兩個路徑進入 — 有意識的妥協分支（測試面）和功能不完整分支（測試覆蓋缺口），因為測試缺口常被混入 Tech Debt 或 Feature Gap，獨立分類讓追蹤更精確。

第五類 **Quality Gate** 獨立於決策樹 — 不是「要修的東西」，而是「防止 Defect / Tech Debt / Feature Gap 進入 codebase 的基礎設施」（例如架構測試、CI 品質關卡、搜查手冊）。TI 本身就是 Quality Gate 的一部分 — 補齊測試覆蓋就是在建立防線。

---

## 3. AI 效率優化設計

這是本體系與一般品質追蹤最大的差異。每個設計決策都考慮 AI 的工作方式。

### 3.1 發現性（AI 怎麼知道這個系統存在）

在專案的 `CLAUDE.md`（或等效的 AI 指令檔）加入入口。參考 `CLAUDE.md.snippet`。

### 3.2 完成步驟 Checklist（AI 防遺漏）

SKILL.md 明確列出完成步驟（關閉 Issue + 填寫完成 comment + 檢查相依 + 更新 taxonomy）。AI 最常犯的錯就是改完原始碼但忘記更新追蹤狀態。

### 3.3 待追蹤發現的行動指引

明確告訴 AI「不要主動升級為正式項目」，避免 AI 過度主動地建立大量低優先級項目。

### 3.4 連結與發現

- **Issue → Taxonomy：** Defect Issue body 的「缺陷子類別」欄位記錄 D-XXX 代碼
- **Taxonomy → Issue：** 搜查結果段落以 `#N` 格式記錄發現的 Issue（collocated）或 `owner/repo#N`（companion repo）

### 3.5 AI 效率 Checklist（啟用品質系統時確認）

| #   | 項目                                      | 驗證方式                                       |
| --- | ----------------------------------------- | ---------------------------------------------- |
| 1   | CLAUDE.md 有品質系統入口                  | `grep '品質' CLAUDE.md`                        |
| 2   | SKILL.md 有「完成步驟」指引               | `grep '完成步驟' .claude/skills/quality/SKILL.md` |
| 3   | README 有「快速查詢」section              | `grep '快速查詢' quality/README.md`            |
| 4   | SKILL.md 完成步驟與 README 一致           | 比對兩處步驟數與內容                           |
| 5   | Issue 模板已安裝（.github/ 或 .gitlab/）  | `ls .github/ISSUE_TEMPLATE/ 2>/dev/null \|\| ls .gitlab/issue_templates/` |
| 6   | README 有「建立新項目」流程               | `grep '建立新項目' quality/README.md`          |
| 7   | 「待追蹤發現」有行動指引                  | `grep 'AI 行動指引' quality/README.md`         |

### 3.6 Issue-Native 遷移理由

原系統使用 markdown 檔案（DEF-001.md 等）追蹤品質項目。遷移至 Issue/PR-native 的原因：

| 面向 | Markdown 追蹤檔 | Issue/PR-Native |
| ---- | -------------- | --------------- |
| 搜尋與過濾 | `grep` 文字搜尋 | Label 語意過濾，原生 Board |
| 協作 | 手動通知 | 原生 assign、comment、mention |
| 狀態管理 | 手動改 metadata 表 | Open/Closed + label，PR 自動 close |
| 統計 | 自訂 bash script | 平台原生 Insights / API |
| 可移植性 | 零依賴（純 markdown） | 需要 GitHub/GitLab + CLI |
| AI 操作 | 讀寫本地檔案 | 需要 `gh`/`glab` CLI |

**已知 trade-off：**
- Sweep checkbox 從 hook 強制驗證退化為 PR template 中的 honor-based checkbox（Issue-native 無對應的檔案層 hook 目標）
- 統計腳本從離線可用變為需要 CLI 認證 + 網路
- Companion repo 模式需使用 `owner/repo#N` 格式跨 repo 引用 Issue

---

## 4. 方法論來源

| 來源                                                                                                                                               | 採用的概念                                               |
| -------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------- |
| [Orthogonal Defect Classification (ODC)](https://en.wikipedia.org/wiki/Orthogonal_Defect_Classification)                                           | Root Cause 分類、Trigger（逃逸階段）、可重複的搜查模式   |
| [DORA Metrics](https://dora.dev/guides/dora-metrics/)                                                                                              | Change Failure Rate 的概念 → 逃逸階段追蹤                |
| [GitHub Label Taxonomy](https://robinpowered.com/blog/best-practice-system-for-organizing-and-tagging-github-issues)                               | 前綴式分類（type: / priority: / status:）                |
| [Shift-Left Testing](https://www.sonarsource.com/resources/library/shift-left/)                                                                    | 搜查手冊 → 自動化防線的演進路徑                          |
| [Escaped Defect Analysis](https://softwareengineeringauthority.com/index.php/tools/13-software-engineering-disciplines/14-escaped-defect-analysis) | 逃逸階段欄位設計                                         |
| [Martin Fowler Tech Debt Quadrant](https://en.wikipedia.org/wiki/Technical_debt)                                                                   | Deliberate vs Inadvertent → 決策樹的「有意識的妥協」分支 |
| [Cem Kaner — Exploratory Testing](https://en.wikipedia.org/wiki/Exploratory_testing) (1984)                                                        | ET 定義：同時設計、執行測試並從中學習；末端問題分類 |
| [James Bach — Session-Based Test Management](https://www.satisfice.com/sbtm/)                                                                      | SBTM：charter + session + debrief 結構 → 4T 欄位模板 |
| [Context-Driven School of Testing](https://context-driven-testing.com/)                                                                            | 測試方法隨情境調整 → 搜查手冊與 ET 互補的雙層模型 |

---

## 5. 雙層品質發現模型

品質發現分為 AI 執行層（搜查手冊 grep）與人類判斷層（探索式測試 ET）。

- **AI 執行層：** 系統性掃查已知缺陷模式，可自動化、可重複。搜查手冊（defect-taxonomy.md）是這一層的操作手冊。
- **人類判斷層：** 處理 grep 搜不到的問題：業務邊界條件、意圖判斷、生產環境獨特性。探索式測試（ET）透過 charter 結構化地引導人類探索這些盲區。
- **互饋迴圈：** ET 發現的新 pattern 可推廣為 grep pattern（需通過三項標準），搜查手冊的「盲區」（charter seed）產生 ET 探索目標。

完整方法論見 [discovery-strategy.md](./discovery-strategy.md)。
