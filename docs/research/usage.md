# agend-terminal 實際使用率調查（唯讀）

> 歸檔註：設計階段的原始紀錄，內容原樣保存（2026-09-24）。只把絕對路徑換成佔位符：`<scratchpad>` = 當時的暫存目錄、`<v1-repo>` = agend-terminal repo、`<tmp>` = 系統暫存目錄、`~` = 使用者家目錄。文中提到的腳本、log、schema dump 沒有一起歸檔。

調查時間：2026-09-24。方法：直接讀取 `~/.agend` 與 `~/.agend-terminal` 下的
fleet.yaml / *.jsonl / *.log / *.json，並比對原始碼 `src/mcp/registry.rs`、
`src/mcp/handlers/mod.rs`、`docs/MCP-TOOLS.md` 判定統計口徑。全程未寫入、未啟
停 daemon。

## 0. 兩個 daemon home 的真實狀態（重要前提）

- `~/.agend`：`fleet.yaml` 內容為 `instances: {}`（空），最後一筆
  daemon log（`~/.agend/daemon.2026-09-03.log`）末行是
  `ERROR no instances found in fleet.yaml`，之後無任何新 log。
  `mcp-usage-stats.jsonl` 只有 2889 行（2026-06-12～09-03，且多為
  `"action":"bogus"` 測試資料）。→ 這是一個已死亡/棄用的實例，不是文件命名
  暗示的「主要」家目錄。
- `~/.agend-terminal`：daemon.2026-09-23.log 最後一行時間戳是
  `2026-09-24T05:15:57`（今天），持續有 fleet 活動。→ **這才是使用者實際在用
  的 daemon home**，以下統計全部以它為準。

## 1. fleet.yaml 設定（`~/.agend-terminal/fleet.yaml`）

- 13 個已建立 instance，backend 分布（來自 fleet.yaml，並以
  `snapshot.json` 2026-09-23T21:20 的 13 個 live agent 交叉核對，backend_command
  完全一致）：
  - `codex`：7（archfix-codex-reviewer, archfix-codex-reviewer-2,
    archfix-codex-dev, codex-125550, codex-nulls-lead,
    codex-nulls-training-data-02-dev, codex-nulls-reviewer-7）
  - `claude`：3（claude-ba5740, general[backend 預設為 claude],
    claude-nulls-darwin-render）
  - `opencode`：3（opencode-nulls-recovered, opencode-af1149,
    opencode-linux-render）
  - `kiro-cli`：只出現在 `templates.dev`（未實例化的範本），從未真的建立
    instance；`grok`、`shell` 完全沒出現。
- Teams：2 個，`archfix`（4 members, orchestrator codex-125550）、
  `nulls-research`（6 members, orchestrator codex-nulls-lead）。
- Channel：只設定 `telegram`（`channel.type: telegram`，topic mode，
  有 bot_token_env / group_id / user_allowlist，值不外流）。fleet.yaml 全文
  搜尋 `discord` 無結果；`~/.agend-terminal/catalog.checkpoint.json` 中出現過
  "discord" 字樣，但那是一個 13MB 的通用 checkpoint 快取檔，非 channel 設定，
  不算 discord 功能被啟用。
- Tray：fleet.yaml、runtime-config.json、session.json 均無 `tray` 設定鍵；
  daemon 近兩天 log 中 `tray` 命中 634 次，全部來自
  `dependabot/cargo/tray-icon-0.25.1` 這個 PR 分支名稱（與最近的
  tray-icon 版本 bump PR 對應），完全不是 tray 圖示子系統的執行期紀錄；
  `tray_icon|TrayIcon|systray` 精確比對命中 0 次。→ tray 只在編譯 feature
  層級存在，找不到任何執行期使用證據。
- Schedules / ci_watch 屬於可選功能，見第 3 節。

## 2. MCP tool 呼叫次數排名

**方法**：`src/mcp/handlers/mod.rs:198-211` 是唯一的 dispatch 收斂點，每次
「非唯讀」tool 呼叫都會呼叫 `crate::mcp::usage_stats::record()` 寫一行 JSON
到 `<home>/mcp-usage-stats.jsonl`（`src/mcp/usage_stats.rs`）。該檔案依
`src/jsonl_retention.rs` 設定為「1MB / 5 個 rotation / 目標保留 30 天」，
但因為呼叫量大，`~/.agend-terminal` 目前實際只保留了
2026-09-14T17:53 ～ 2026-09-24T05:15（main + .1~.5 共 47,802 筆），
**不到 10 天**，比要求的「近 30 天」短，且沒有更舊的資料可補（已確認
舊 rotation 已被覆寫，這是唯一可得的全部歷史）。因為現存資料本來就落在最近
30 天窗口內，「近 30 天」與「全期（現存）」兩欄數字相同，故只列一欄。

排名（`grep -o '"tool":"[^"]*"' mcp-usage-stats.jsonl{,.1..5} | sort | uniq -c`）：

| 排名 | tool | 次數 |
|---|---|---|
| 1 | inbox | 18937 |
| 2 | task | 12193 |
| 3 | send | 9679 |
| 4 | ci | 1595 |
| 5 | repo | 1357 |
| 6 | release_worktree | 1114 |
| 7 | decision | 900 |
| 8 | reply | 824 |
| 9 | set_waiting_on | 343 |
| 10 | instance | 128 |
| 11 | team | 112 |
| 12 | bind_self | 101 |
| 13 | create_instance | 85 |
| 14 | schedule | 67 |
| 14 | restart_instance | 67 |
| 16 | delete_instance | 57 |
| 17 | revoke_review_assignment | 54 |
| 17 | operator_page | 54 |
| 19 | usage_limit_takeover | 38 |
| 19 | set_model | 38 |
| 21 | interrupt | 20 |
| 22 | restart_daemon | 11 |
| 23 | health | 9 |
| 24 | deployment | 7 |
| 25 | set_metadata | 6 |
| 26 | move_pane | 3 |
| 27 | start_instance | 2 |
| 28 | config | 1 |

文件（`docs/MCP-TOOLS.md`）列出 34 個 tool，上表只有 28 個出現過。缺席的 6 個：
`correct_review_class`、`download_attachment`、`bind_topic`、
`list_instances`、`pane_snapshot`、`binding_state`。

**但要注意一個統計口徑陷阱**：`src/mcp/handlers/mod.rs:199-211` 明確說「唯讀
tool 跳過 usage-stats 寫入」，而 `src/mcp/registry.rs` 把
`list_instances`（FAST_READ_ONLY）、`pane_snapshot`、`binding_state`
（皆為 READ_ONLY）標記為唯讀 → **這 3 個工具的 0 次紀錄是統計方法的死角，不
代表真的沒人用**（CLAUDE.md 本身的 worktree 流程就要求呼叫
`binding_state`）。
真正「該記錄但確實 0 次」、可信地判定為幾乎沒被使用的只有 3 個：
`correct_review_class`（SIDE_EFFECT class）、
`download_attachment`（RETRY_SAFE class）、
`bind_topic`（SIDE_EFFECT class）——這三個在 registry.rs 都不是唯讀，
理論上每次呼叫都會被記錄，但近 10 天完整資料裡一次都沒出現。

其餘低頻（個位數～數十次）但非零的：`config`(1)、`start_instance`(2)、
`move_pane`(3)、`set_metadata`(6)、`deployment`(7)、`health`(9)、
`restart_daemon`(11)。

## 3. CLI 子命令 / schedules / ci_watch / decisions / task board

- **CLI 子命令（互動式）**：查了 `~/.zsh_history`（14,761 行，涵蓋
  2024-09～2026-09 整個歷史），對 `agend`、`agend-terminal` 做
  grep，**完全 0 筆命中**；daemon.log 也沒有看到 API/CLI 存取的
  access-log 樣式紀錄。**結論：未找到使用者手動下達 CLI 子命令的證據**
  （已嘗試方法：`grep -o 'agend[a-zA-Z_-]*' ~/.zsh_history`、
  `grep "target/release/agend"`、daemon.log 內搜尋 `cli_command=`／
  `"command":`／`GET /`／`POST /` 樣式）。無法排除的可能：agent
  子行程呼叫 CLI 但沒有寫回這個 shell 的 history 檔。所有可觀察到的操作
  都是透過 MCP tool（第 2 節）或排程自動觸發。
- **Schedules**：`schedules.json`（141KB）目前有 98 筆
  schedule，其中 90 筆是 `trigger.kind=once`（多為 agent 自建的一次性
  提醒/追蹤任務，例如 issue 檢查點），8 筆 `cron`（含每 30 分鐘的
  autonomous-monitor 巡邏、股癌/投資癮 podcast 排程），目前仍
  `enabled` 的只剩 3 筆。`schedules-archive.jsonl` 有 38 筆已歸檔紀錄，
  `run_history` 顯示確實被觸發過（`status: ok_queued`）。→ 功能有實際
  被使用，但多數是一次性任務，長駐 cron 只剩 3 個。
- **ci_watch**：`ci-watches/` 目錄有 153 個 watch 狀態檔
  （`*.json` + `*.lock` 成對），加上 daemon.log 近兩天 WARN 最大宗
  （見第 4 節）就是 ci_watch 相關訊息（`PR-3: open PR has no armed
  ci-watch...`、`ancestry compare failed`、`gh branches list failed`
  等），是活動量最大的可選功能之一。
- **decisions**：`decisions/` 目錄 5486 個檔案（`.json`/`.lock` 成對，
  約 2743 筆 decision），大量真實使用。
- **task board**：`task-progress/` 9529 個檔案，`task_events.jsonl`
  13,492 行事件，`task` 是 MCP tool 呼叫排名第 2（12,193 次）→ task
  board 是核心高頻功能。

## 4. daemon.log 近期 ERROR/WARN 分群（前 15，依訊息模板正規化計數）

範圍：`daemon.2026-09-22.log` + `daemon.2026-09-23.log`（=2026-09-22 00:00
～2026-09-24 05:15，這是目前留存的**全部** daemon.log，因為 `#914` 的
retention 預設只留 3 天／`AGEND_LOG_RETAIN_DAYS`，更舊的 log 已被輪替刪除，
所以「近期」= 目前能拿到的最大範圍）。ERROR 總數僅 13、WARN 總數 17133。

WARN 前 15（次數 / 樣板）：
1. 8440 — `PR-N: open PR has no armed ci-watch AND no bound agent — cannot auto-arm`
2. 2973 — `#N Nb: ancestry compare failed — freshness_error`
3. 1541 — `#N-BN: gh branches list failed — skipping remote GC this tick`
4.  861 — `#N gh-poll: scanner-thread slip > Ns`
5.  542 — `#N dispatch test-name check skipped — no resolvable PR tree`
6.  505 — `Codex app-server output`（stderr passthrough）
7.  445 — `CI check failed`（多為 GitHub API 403 rate limit）
8.  301 — `worktree has WIP, skipping GC`
9.  196 — `TUI bridge retirement port read failed`
10. 194 — `provenance injection failed — no active channel`
11. 123 — `delivery_worker: structured transport delivery failed`
12. 123 — `telegram notify failed`（多為對 api.telegram.org 的網路錯誤）
13. 108 — `fetch --prune failed during worktree/branch sweep`
14. 108 — `list_worktrees: git worktree list exited non-zero`
15.  85 — `CI batch poll API error; repo backing off`

ERROR（僅 3 種樣板，13 次）：
- 7 — `full_delete_instance left residual state — silent-drop class pattern blocked`
- 3 — `op failed — result dropped (silent-loss #1630/#1647)`
- 3 — `#1492/#1535/#1629 lock-across-self-IPC deadlock risk — refusing self-IPC`

**痛點解讀**：前 5 名（合計 14,357 / 17,133 ≈ 84% 的 WARN）全部圍繞
「GitHub CI/ci_watch 子系統」：PR 沒綁 ci-watch、GitHub API 403 rate limit、
`gh` CLI 呼叫失敗。這是目前最大的實際痛點來源，其次是 worktree/GC 清理
（WIP 檔案擋住 GC、`git worktree list`/`fetch --prune` 失敗）。

## 5. 磁碟佔用

`~/.agend`：總計 3.3M，幾乎全是 `.locks`（1.7M，8023 個鎖檔）——這個家目錄
本身就是廢棄狀態，沒有實質資料堆積問題。

`~/.agend-terminal`：**總計 161G**，前幾大：

| 目錄 | 大小 | 說明 |
|---|---|---|
| evidence/ | 108G | 433,532 個檔案；其中單一子目錄
  `aud07-pr380-exit-handoff-20260920T2001+0800` 就佔 84G，副檔名以
  `.sc/.toml/.rmat/.csv/.h/.glb/.ktx/.sctx/.cpp` 為主（像是把整個遊戲
  引擎資產樹當成單一任務的「證據」整包複製進來，從未清理）。這是目前最大的
  狀態堆積問題。 |
| workspace/ | 23G | 各 instance 的工作目錄 |
| worktrees/ | 19G | git worktree（大量並行 agent 開發分支） |
| verification/ | 3.1G | 驗證用暫存 |
| cache/ | 2.2G | |
| .trash/ | 1.4G | 「已刪除」但從未真的清空 |
| backend-data/ | 660M | |
| runs/ | 543M | |
| recovery/ | 543M | |
| task_events_archive/ | 336M | |
| scratch/ | 195M | |
| reconcile-backups/ | 136M | |
| task-progress/ | 38M（9529 檔） | |
| boards/ | 38M（25 個 repo 的看板） | |
| decisions/ | 11M（5486 檔） | |

檔案數異常大的鎖目錄：`.locks/` 有 10,376 個檔案（僅 4.6M，量體小但檔案數
極多，NFS/APFS 目錄效能隱憂）。
