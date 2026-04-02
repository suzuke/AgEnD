# AgEnD 發展藍圖 (Roadmap)

> 最後更新：2026-04-01 (v1.3.0)
> 由多代理共識產出：Claude Code, Codex, Gemini CLI, OpenCode

## 已完成 (v1.0–v1.3)

- [x] 多後端 (Multi-backend) 支援 (Claude Code, Codex, Gemini CLI, OpenCode)
- [x] 多頻道 (Multi-channel) 支援 (Telegram, Discord)
- [x] Fleet 編排 (Fleet orchestration)（持久化專案實例）
- [x] 跨實例委派 (Cross-instance delegation) (`send_to_instance`, `delegate_task`, `report_result`)
- [x] Cron 排程 (Cron scheduling)
- [x] 帶有每日限制的成本防護 (Cost guard)
- [x] 上下文輪轉 (Context rotation)（自動刷新過期 session）
- [x] `/sysinfo` fleet 診斷
- [x] `safeHandler` 非同步錯誤邊界 (Async error boundaries)
- [x] FleetManager 模組化 (`RoutingEngine`, `InstanceLifecycle`, `TopicArchiver`, `StatuslineWatcher`, `OutboundHandlers`)
- [x] IPC socket 強化 (umask TOCTOU 修復)
- [x] 平台無關核心（所有 Telegram/Discord 特定內容都在轉接器中）

---

## 第一階段：可觀測性與儀表板 (Observability & Dashboard)

**目標：** 在不離開瀏覽器的情況下使 fleet 運作可見。

### 1.1 REST API 擴展
將現有的健康檢查伺服器擴展為完整的 fleet API：
- `GET /api/fleet` — `getSysInfo()` JSON
- `GET /api/instances/:name` — 實例詳情、日誌、成本
- `GET /api/events` — `EventLog` 查詢（成本快照、輪轉、懸掛）
- `GET /api/cost/timeline` — 用於圖表的成本趨勢數據
- `POST /api/instances/:name/restart` — 觸發重啟

**工作量：** 約 200 行。數據已存在於 `EventLog` (SQLite) 和 `getSysInfo()` 中。

### 1.2 成本分析儀表板 (MVP)
由 daemon 提供的輕量級網頁 UI：
- 每個實例的成本趨勢圖（來自 `EventLog` `cost_snapshot` 的數據）
- Fleet 狀態板（包含狀態/IPC/成本/頻率限制的實例列表）
- 透過 SSE 或 WebSocket 進行即時更新

**技術棧：** 靜態 HTML + Chart.js，由健康檢查伺服器提供。MVP 不需要框架。

### 1.3 任務時間軸與錯誤檢視器
- 任務指派/完成時間軸
- 帶有 `safeHandler` 上下文標籤的錯誤日誌檢視器
- 排程執行歷史

---

## 第二階段：工程工作流整合 (Engineering Workflow Integration)

**目標：** 讓 AgEnD 成為實際工程工作流的一部分，而不僅僅是一個聊天工具。

### 2.1 GitHub / GitLab 整合
- 從 issue、PR 或 webhook 觸發 agent 任務
- 將結果作為 PR 評論或 issue 更新回報
- 排程 repo 維護（每晚分流、依賴項更新）

### 2.2 CI/CD 鉤子 (Hooks)
- Fleet as Code — 透過 git 管理實例配置
- 透過 PR 合併部署/更新實例
- 用於 agent 輔助審核的 pre-commit 鉤子

### 2.3 對話歷史與持久化
- 將所有進站/出站訊息記錄到 SQLite
- 每個實例的可搜尋對話歷史
- 跨 session 上下文延續

---

## 第三階段：外掛與技能系統 (Plugin & Skills System)

**目標：** 讓社群在不 fork 的情況下擴展 AgEnD。

### 3.1 外掛架構 (Plugin architecture)
- 掃描 `~/.agend/plugins/` 中的 npm 套件
- 用於後端、頻道和工具外掛的動態 `import()`
- 標準介面已存在：`CliBackend`、`ChannelAdapter`、`outboundHandlers` Map

### 3.2 技能 / 任務範本
- 可重複使用的執行指南 (runbooks)（例如：「安全性掃描」、「依賴項更新」、「程式碼審閱」）
- 帶有核准流程的參數化任務範本
- 可透過 npm 套件分享

### 3.3 策略與權限 (Policy & permissions)
- 每個實例的環境/沙盒控制
- 高風險操作的人工核准流程
- 基於團隊角色的存取控制

---

## 第四階段：生態系統擴展 (Ecosystem Expansion)

**目標：** 跨頻道、後端和案例擴大影響力。

### 4.1 更多頻道
- **Slack**（透過 Bolt SDK 約 300-400 行）— 企業採用
- **網頁聊天 (Web Chat)** (WebSocket server) — 自託管控制面板
- `ChannelAdapter` 抽象已得到證明；新的轉接器不會觸碰核心程式碼

### 4.2 更多後端
- **Aider**（約 50-80 行）— 最受歡迎的開源編碼 agent
- **Cursor Agent**（當 CLI 模式可用時）
- **自訂 CLI** — 說明如何為任何工具實作 `CliBackend`

### 4.3 智慧後端路由
- 依任務類型自動選擇後端（快速修復 → 快速模型，架構 → 強大模型）
- 比較各後端的成本/延遲/成功率
- 基於歷史表現的路由建議

---

## 第五階段：進階運作（長期）(Advanced Operations)

### 5.1 Agent 群集協調 (Agent swarm coordination)
- 自動任務分解與委派
- Agent 對 Agent 招募（編碼 agent → 安全掃描 agent → 審閱 agent）
- 帶有結果聚合的並行執行

### 5.2 全 Fleet 知識中心 (Fleet-wide knowledge hub)
- 跨實例共享上下文（架構決策、技術債、偏好）
- 基於 RAG 的專案文件檢索
- 從過去的任務結果中學習

### 5.3 自癒 Fleet (Self-healing fleet)
- 在重複失敗時自動重啟並切換模型
- 頻率限制預測與先發制人的後端切換
- 成本/延遲模式的異常偵測

### 5.4 控制平面 / 數據平面分離 (Control Plane / Data Plane separation)
- 數據平面（本地）：daemon 在代碼和機密資訊附近執行
- 控制平面（選用雲端）：跨機器發現、全域排程、統一監控

---

## 明確延後 (Explicitly Deferred)

| 方向 | 理由 |
|-----------|--------|
| Agent 市集 | 生態系統尚不成熟；需要先有外掛系統 |
| 多機器分佈式 Fleet | 架構變動過大；先專注於單機卓越 |
| LINE 頻道 | API 複雜，全域市場有限 |
| 原生桌面應用程式 | 開發成本高；網頁 UI 已能滿足需求 |

---

## 產品定位

> **AgEnD 不是另一個編碼 agent。它是讓編碼 agent 作為一個團隊運作的維運層。**

- 後端無關：適用於任何編碼 CLI
- 頻道原生：Telegram/Discord 作為人工參與的控制平面
- 持久化實例：每個專案/repo 一個實例，而非丟棄式的聊天執行緒
- Fleet 協調：跨專案和後端進行委派、排程、監控和控制
