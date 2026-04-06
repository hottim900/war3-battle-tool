# Changelog

All notable changes to this project will be documented in this file.

## [0.0.2.0] - 2026-04-06

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

## [0.0.1.0] - 2026-04-06

### Added
- 首次啟動說明頁：在設定精靈前加一頁，說明 Windows 防火牆、UPnP、P2P 直連、SmartScreen 警告
- 設定頁「網路說明」區塊：可摺疊的防火牆/UPnP/P2P 說明，讓使用者事後重新查看
- 設定精靈「上一步」按鈕：可從暱稱設定頁回到說明頁
