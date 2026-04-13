# 更新日誌 (Changelog)

本專案的所有顯著變更都將記錄在此檔案中。

格式基於 [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)。

## [未發佈] (Unreleased)

### 新增
- `replace_instance` MCP 工具 — 原子性替換 instance，從 daemon 的 ring buffer 收集交接 context 並透過標準訊息傳遞路徑注入新 instance
- Workflow template 新增溝通效率規則 — 禁止客套、沉默即同意、合併要點、review 一次來回
- Fleet 事件（輪轉、懸掛、成本警報）的 Webhook 通知
- 用於外部監控的 HTTP 健康檢查端點 (`/health`, `/status`)
- 在 Context 輪轉時具有驗證與重試機制的結構化交接範本
- 權限中繼 UX 改進（逾時倒數、持久化的「一律允許」、決定後的回饋）
- 主題圖示自動更新（執行中/已停止）+ 閒置封存
- 過濾 Telegram 服務訊息（主題重新命名、置頂等）以節省 token

### 變更
- **ContextGuardian 簡化為純監控** — 移除 max_age 計時器、狀態機（NORMAL/RESTARTING/GRACE）和所有重啟觸發器。所有 CLI 後端（Claude Code、Codex、Gemini CLI、OpenCode、Kiro CLI）都有內建的 auto-compact 處理 context 限制。
- **Crash recovery 優先嘗試 --resume** — 崩潰重生時先嘗試 `--resume` 恢復完整對話歷史，失敗才 fallback 到全新 session + snapshot 注入。Resume 成功時節省 context。

### 修復
- 最小化的 `claude-settings.json` — 允許列表中僅包含 AgEnD MCP 工具，不再覆蓋使用者全域的權限設定

## [1.14.0] - 2026-04-07

### 新增
- **Plugin 系統 + Discord adapter 獨立** — Discord adapter 搬到獨立 `agend-plugin-discord` package；factory.ts 支援 `agend-plugin-{type}` / `agend-adapter-{type}` / 裸名稱慣例；主 package 匯出（`/channel`、`/types`）讓第三方 plugin 可用
- **Web UI Phase 2：完整操控面板** — instance stop/start/restart/delete（name 確認）、建立 instance 表單（directory 可選、backend 自動偵測）、Task board CRUD、排程管理、團隊管理、Fleet 設定編輯器（表單式 + 敏感欄位遮蔽）
- **Web UI 版面：Fleet vs Instance** — Sidebar 加「Fleet」入口顯示 fleet 級 tabs（Tasks、Schedules、Teams、Config）；Instance 只保留 Chat + Detail；跨導航連結
- **Web UI UX 改善** — Toast 通知、載入狀態、Cron 人類可讀描述、加大狀態點、空狀態引導、成本標註、網站一致風格（`#2AABEE` 強調色、Inter + JetBrains Mono 字體）
- **Backend 自動偵測** — `GET /ui/backends` 掃描 PATH；建立 instance 的 dropdown 顯示安裝/未安裝狀態
- **指定 instance 重啟** — `agend fleet restart <instance>` 透過 fleet HTTP API
- **一鍵安裝腳本** — `curl -fsSL https://suzuke.github.io/AgEnD/install.sh | bash`
- **project_roots 限制** — `create_instance` 拒絕不在設定 roots 範圍內的目錄

### 修復
- **Web UI 回覆 context** — 首次 web 訊息不再出現「No active chat context」；使用真實 Telegram group_id/topic_id
- **Web↔Telegram 雙向同步** — Web 訊息以 `🌐` 前綴轉發到 Telegram；Telegram 訊息透過 SSE 推送到 Web UI
- **SSE 即時狀態刷新** — 操作按鈕在 stop/start/restart/delete 後即時更新
- **.env 覆蓋** — `.env` 檔案值無條件覆蓋繼承的 shell 環境變數
- **tmux duplicate session race** — `ensureSession()` 處理並行啟動時的競爭條件
- **建立 Instance 表單** — directory 改為可選，topic_name 動態必填

### 變更
- **discord.js 從核心依賴移除** — 僅在安裝 `agend-plugin-discord` 時需要
- **Web API 抽取到 `web-api.ts`** — 縮減 fleet-manager.ts；所有 `/ui/*` 路由集中管理
- **認證統一** — 所有 Web UI 端點（含 restart）都需要 token 認證

## [1.13.0] - 2026-04-06

### 新增
- **Web UI Phase 2：完整操控面板** — 建立/刪除 instance、Task board CRUD（建立、認領、完成）、排程管理（建立、刪除）、團隊管理（成員勾選建立、刪除）、Fleet 設定檢視（唯讀、已清理敏感資訊）
- **Web UI 風格統一** — 對齊網站設計：Telegram 藍 `#2AABEE` 強調色、Inter + JetBrains Mono 字體、深色主題、圓角卡片、Toast 通知、載入狀態
- **一鍵安裝腳本** — `curl -fsSL https://suzuke.github.io/AgEnD/install.sh | bash` 一行完成安裝（Node.js via nvm、tmux、agend、後端偵測）
- **project_roots 限制** — `create_instance` 拒絕不在 `project_roots` 範圍內的目錄
- **認證統一** — 所有 Web UI 端點（包含 restart）都需要 token 認證

### 修復
- **Web UI 回覆 context** — 首次從 Web UI 發訊不再出現「No active chat context」錯誤；使用真實 Telegram group_id/topic_id
- **即時狀態刷新** — Instance 操作按鈕在 stop/start/restart/delete 後透過 SSE 即時更新
- **Web↔Telegram 雙向同步** — Web 訊息以 `🌐` 前綴轉發到 Telegram topic；Telegram 訊息透過 SSE 推送到 Web UI

### 文件
- 全面文件盤點：所有文件新增 20+ 遺漏功能
- 網站全面改版為 Spectra 風格深色設計

## [1.12.0] - 2026-04-06

### 新增
- **Web UI 儀表板** — `agend web` 啟動瀏覽器 fleet 監控，SSE 即時更新 + 整合聊天介面，支援 Telegram 雙向同步
- **agend quickstart** — 簡化 4 問題設定精靈，取代 `agend init` 作為推薦的新手入口
- **project_roots 限制** — `create_instance` 驗證工作目錄在設定的 `project_roots` 範圍內
- **HTML 對話匯出** — `agend export-chat` 匯出 fleet 活動為獨立 HTML，支援日期篩選（`--from`、`--to`）
- **Mirror Topic** — `mirror_topic_id` 設定，在專屬 topic 觀察跨 instance 通訊

### 修復
- **平行啟動** — 處理多 instance 同時啟動時的 tmux duplicate session race
- **.env 優先覆蓋** — `.env` 的值正確覆蓋繼承的 shell 環境變數
- **Web UI 聊天同步** — Web UI 與 Telegram 之間的雙向訊息同步

### 文件
- README 大改版：hero section、功能亮點、架構圖、運作原理說明
- Quick Start 改為使用 `agend quickstart`
- 全面文件盤點：features.md、cli.md、configuration.md 更新所有 v1.11.0-v1.12.0 功能

## [1.11.0] - 2026-04-05

### 新增
- **Kiro CLI backend** — 新增 AWS Kiro CLI 支援（`backend: kiro-cli`）。支援 session resume、MCP config、error patterns。模型：auto、claude-sonnet-4.5、claude-haiku-4.5、deepseek-3.2 等
- **內建 workflow 模板** — fleet 協作流程透過 MCP instructions 自動注入。可在 fleet.yaml 的 `workflow` 欄位設定（`"builtin"`、`"file:path"` 或 `false`）
- **Workflow 分層：coordinator vs executor** — General instance 取得完整 coordinator 指南（Choosing Collaborators、Task Sizing、Delegation Principles、Goal & Decision Management）。其他 instance 取得精簡的 executor 版本（Communication Rules、Progress Tracking、Context Protection）
- **`create_instance` 的 systemPrompt 參數** — 建立 instance 時可傳入自訂 system prompt（僅支援 inline 文字）
- **Fleet ready Telegram 通知** — `startAll` 和 `restartInstances` 完成後發送「Fleet ready. N/M instances running.」到 General topic，含失敗 instance 報告
- **E2E 測試框架** — 79+ 測試在 Tart VM 中隔離執行。Mock backend 支援 `pty_output` 指令模擬錯誤。T15 workflow 模板測試、T16 failover cooldown 測試
- **Token overhead 量測** — 測試腳本（`scripts/measure-token-overhead.sh`）與報告。Full profile：+887 tokens（佔 200K context 的 0.44%，$0.003/msg）
- **Codex 用量限制偵測** — 「You've hit your usage limit」error pattern（action: pause）
- **MockBackend error patterns** — `MOCK_RATE_LIMIT` 和 `MOCK_AUTH_ERROR` 供 E2E 測試使用

### 修復
- **Crash recovery snapshot restore** — 在 crash 偵測時寫入 snapshot（不只 context rotation）；以 in-memory `snapshotConsumed` flag 取代 single-consume 刪除，檔案保留供 daemon 重啟恢復
- **Codex session resume** — `CodexBackend.buildCommand()` 現在在 session-id 存在時使用 `codex resume <session-id>`（#11）
- **Rate limit failover 循環** — failover 類型的 PTY error 加入 5 分鐘 cooldown，防止 terminal buffer 殘留文字重複觸發（#10）
- **PTY error monitor hash dedup** — recovery 時記錄 pane hash，同畫面同 error 不重複觸發
- **CLI restart 等待** — bootout/bootstrap 之間的固定 1 秒改為動態 polling（最多 30 秒），修復多 instance 時「Bootstrap failed: Input/output error」
- **CLI attach 互動選單** — fuzzy match 多個結果時顯示編號選單而非報錯
- **CLI logs ANSI 清理** — 增強 `stripAnsi()` 處理 cursor 移動、DEC private modes、carriage returns 等
- **agent 訊息中的 `reply_to_text`** — 用戶回覆的原始訊息內容現在包含在 paste 給 agent 的格式化訊息中
- **General instructions 按 backend 產生** — auto-create 根據 `fleet.defaults.backend` 寫入對應檔案（CLAUDE.md、AGENTS.md、GEMINI.md、.kiro/steering/project.md）
- **General instructions 每次啟動確認** — `ensureGeneralInstructions()` 在每次 `startInstance` 時呼叫，不只 auto-create
- **內建文字英文化** — 所有系統產生的文字從中文改為英文（排程通知、語音訊息標籤、general instructions）
- **General 委派原則** — 改寫為 coordinator 角色：主動委派，以具體條件判斷

### 變更
- Fleet start/restart 通知統一為「Fleet ready. N/M instances running.」格式，送到 General topic
- 移除 `buildDecisionsPrompt()` dead code（v1.9.0 已故意停用）
- 移除 fleet-manager 的 `getActiveDecisionsForProject()`（dead code）

### 文件
- OpenCode MCP instructions 限制（v1.3.10 不讀取 MCP instructions 欄位）
- Kiro CLI MCP instructions 限制（未驗證）
- Token overhead 報告（EN + zh-TW）含可重現的測試腳本

## [1.10.0] - 2026-04-05

_中間版本，改動已包含在 1.11.0。_

## [1.9.1] - 2026-04-03

### 修復
- Health-check 重新啟動時注入 session snapshot — 崩潰/kill 恢復也能還原 context
- Snapshot 貼入時附加「不要回覆」指令，防止模型嘗試 IPC 回覆導致逾時

## [1.9.0] - 2026-04-03

### 破壞性變更
- **System prompt 注入改為 MCP instructions。** Fleet context、自訂 `systemPrompt`、協作規則現在透過 MCP server instructions 注入，不再使用 CLI 的 `--system-prompt` 等 flag。變更原因：
  - Claude Code：`--system-prompt` 傳了檔案路徑而非檔案內容 — fleet prompt **自始至終都沒有正確注入**
  - Gemini CLI：`GEMINI_SYSTEM_MD` 會覆蓋內建 system prompt 並破壞 skills 功能
  - Codex：`.prompt-generated` 是 dead code — 寫入但 CLI 從未讀取
  - OpenCode：`instructions` 陣列被覆蓋而非追加，破壞專案原有的 instructions
- **對現有設定的影響：**
  - `fleet.yaml` 的 `systemPrompt` 欄位保留 — 改由 MCP instructions 注入
  - 不再產生 `.prompt-generated`、`system-prompt.md`、`.opencode-instructions.md` 檔案
  - 各 CLI 的內建 system prompt 不再被覆蓋或修改
  - Active Decisions 不再預載到 system prompt — 改用 `list_decisions` 工具按需查詢
  - Session snapshot（context rotation 接續）改為第一則 inbound 訊息送入（`[system:session-snapshot]`），不再嵌入 system prompt

## [1.8.5] - 2026-04-03

### 修復
- 統一 log 與通知格式為 `sender → receiver: summary` 風格，適用於所有跨 instance 訊息
- Task/query 通知顯示完整訊息內容；report/update 通知僅顯示摘要

## [1.8.4] - 2026-04-03

### 修復
- 跨 instance 通知格式改為 `sender → receiver: summary` 格式
- General Topic instance 不再收到跨 instance 通知貼文
- 降低跨 instance 通知噪音 — 移除發送方 topic 貼文；目標通知優先使用 `task_summary`

## [1.8.3] - 2026-04-03

### 新增
- **Team 支援** — 具名的 instance 群組，用於精準廣播
  - `create_team` — 建立含成員與描述的 team
  - `list_teams` — 列出所有 team 及其成員
  - `update_team` — 新增/移除成員或更新描述
  - `delete_team` — 刪除 team 定義
  - `broadcast` 新增 `team` 參數，可對指定 team 的所有成員廣播
  - `fleet.yaml` 新增 `teams` 區塊，用於持久化 team 定義

## [1.8.2] - 2026-04-03

### 新增
- `fleet.yaml` 中 `working_directory` 現在為選填 — 未指定時自動建立 `~/.agend/workspaces/<name>`
- `create_instance` 的 `directory` 參數現在為選填（省略時自動建立工作空間）

### 修復
- Topic 模式下，Context-bound routing 現在在 IPC 轉發前執行（修正「chat not found」錯誤）
- Telegram：`thread_id=1` 正確視為 General Topic（不傳送 thread 參數）
- Scheduler 在 instance 啟動前完成初始化，確保 fleet 啟動時能正確載入 decisions

## [1.8.1] - 2026-04-03

### 新增
- `reply`、`react`、`edit_message` 改為 context-bound — 不再需要在 tool call 中指定 `chat_id` 和 `thread_id`；daemon 自動從當前對話 context 填入
- PTY 監控的後端錯誤模式偵測 — 偵測到頻率限制、認證錯誤或崩潰時自動通知
- 自動關閉執行時對話框（如 Codex 頻率限制的模型切換提示）
- 模型容錯移轉 — 達到頻率限制時自動切換備用模型（statusline + PTY 偵測）

### 修復
- PTY 錯誤監控處理後發送恢復通知
- 降低錯誤監控誤報；自動從 context 修正無效的 `chat_id`

## [0.3.7] - 2026-03-27

### 新增
- 用於移除實例的 `delete_instance` MCP 工具
- `create_instance --branch` — 用於功能分支隔離的 git worktree 支援
- 外部轉接器外掛載入 — 透過 `npm install agend-adapter-*` 安裝社群轉接器
- 從套件進入點導出頻道類型，供轉接器作者使用
- Discord 轉接器 (MVP) — 連接、發送/接收訊息、按鈕、反應
- 優雅重啟後 Telegram 主題中的每個實例重啟通知

### 修復
- `start_instance`、`create_instance`、`delete_instance` 已加入權限允許列表
- Worktree 實例名稱使用 `topic_name` 而非目錄基底名稱，以避免 Unix socket 路徑溢位（macOS 104 位元組限制）
- 帶有分支的 `create_instance` 不再對基礎 repo 觸發錯誤的 `already_exists`
- `postLaunch` 穩定性檢查替換為 10 秒寬限期
- 重啟通知使用 `fleetConfig.instances` + IPC 推送
- 解決了 Discord 轉接器的 TypeScript 錯誤

## [0.3.6] - 2026-03-27

### 修復
- 防止實例重啟時產生 MCP server 殭屍進程
- 強化 `postLaunch` 自動確認以應對邊緣案例

## [0.3.5] - 2026-03-26

### 新增
- 透過 `create_instance(model: "sonnet")` 進行各實例的模型選擇
- 實例 `description` 欄位，在 `list_instances` 中提供更好的可發現性
- 每 5 分鐘自動從 `sessionRegistry` 清理過期的外部 session
- AgEnD 到陸頁網站（Astro + Tailwind，英文/繁體中文雙語）
- 用於網站部署的 GitHub Actions 工作流
- README 中的安全考量章節

### 變更
- 簡化模型選擇 — 僅可透過 `create_instance` 配置，而非逐條訊息配置
- 使用單一 `query_sessions_response` 進行 session 清理

### 修復
- 安全強化 — 10 項漏洞修復（路徑遍歷、輸入驗證等）
- 向 Telegram 發送完整的跨實例訊息，而非截斷為 200 字元的預覽
- 移除 IPC 秘密驗證 — socket `chmod 0o600` 已足夠且更簡單

## [0.3.4] - 2026-03-26

### 變更
- 移除斜線指令 (`/open`, `/new`, `/meets`, `/debate`, `/collab`) — General 實例透過 `create_instance` / `start_instance` 處理專案管理
- 移除無用程式碼：`sendTextWithKeyboard`、`spawnEphemeralInstance`、會議頻道方法

## [0.3.3] - 2026-03-25

### 修復
- 修正測試斷言中的 `statusline.sh` → `statusline.js`

## [0.3.2] - 2026-03-25

### 新增
- 帶有動態匯入的頻道轉接器工廠，用於未來的多平台支援
- 意圖導向的轉接器方法：`promptUser`、`notifyAlert`、`createTopic`、`topicExists`
- Telegram 權限提示上的「一律允許」按鈕
- `InstanceConfig` 中的每個實例 `cost_guard` 欄位
- `ChannelAdapter` 上的 `topology` 屬性 (`"topics"` | `"channels"` | `"flat"`)

### 變更
- 頻道抽象化階段 A — 從業務邏輯中移除所有 TelegramAdapter 耦合（fleet-manager, daemon, topic-commands 現在使用通用的 ChannelAdapter 介面）
- CLI 版本從 package.json 讀取而非硬編碼值
- 排程子指令現在有 `.description()` 用於幫助文字

### 修復
- statusline 腳本中的 shell 注入 — 將 bash 替換為 Node.js 腳本
- 設定精靈與配置中的時區驗證 (Intl.DateTimeFormat)
- `max_age_hours` 預設值在設定精靈、配置和 README 中統一為 8 小時
- `pino-pretty` 從 devDependencies 移至 dependencies（修復 `npm install -g`）
- 在重啟時清除 `toolStatusLines` 以防止無限增長
- 為 daemon-entry 中的 `--config` `JSON.parse` 加入 try-catch
- 移除無用程式碼 `resetToolStatus()`
