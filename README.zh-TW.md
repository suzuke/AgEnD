# claude-channel-daemon

將 [Claude Code](https://docs.anthropic.com/en/docs/claude-code) Channels 包裝成可靠的背景服務。透過 `node-pty` 執行 Claude Code CLI，提供自動 session 管理、context window 輪替與記憶備份。

[English README](README.md)

> **⚠️ 安全提醒：** 這個 daemon 以 `bypassPermissions` 模式執行 Claude Code。所有權限決策都交給 PreToolUse hook 和 Telegram 遠端批准系統處理。如果批准 server 無法連線，**所有工具調用會被拒絕**，直到 server 恢復為止。內建的 deny list 會擋掉已知的破壞性指令，但這套架構本質上依賴批准 server 做存取控制。**使用風險自負。** 部署前請詳閱[權限控制](#權限控制)段落。

## 為什麼需要這個

Claude Code 的 Telegram plugin 需要一個活著的 CLI session — 關掉終端機 bot 就斷了。這個 daemon 解決了以下問題：

- 透過 `node-pty` 讓 Claude Code 在背景持續運行
- 掛掉時自動重啟（指數退避策略）
- Context 使用量過高時自動輪替 session
- 將記憶備份到 SQLite
- 可安裝為系統服務（macOS launchd / Linux systemd）

## 快速開始

```bash
# Clone 並安裝
git clone https://github.com/suzuke/claude-channel-daemon.git
cd claude-channel-daemon
npm install

# 互動式設定
npx tsx src/cli.ts init

# 啟動 daemon
npx tsx src/cli.ts start
```

## CLI 指令

```
claude-channel-daemon start      啟動 daemon
claude-channel-daemon stop       停止 daemon
claude-channel-daemon status     查看運行狀態
claude-channel-daemon logs       查看日誌 (-n 行數, -f 即時追蹤)
claude-channel-daemon install    安裝為系統服務
claude-channel-daemon uninstall  移除系統服務
claude-channel-daemon init       互動式設定精靈
```

## 架構

```
┌─────────────────────────────────────────────┐
│              claude-channel-daemon           │
│                                             │
│  ┌─────────────────┐  ┌──────────────────┐  │
│  │ Process Manager  │  │ Context Guardian │  │
│  │ (node-pty)       │  │ (自動輪替)       │  │
│  └────────┬─────────┘  └────────┬─────────┘  │
│           │                      │            │
│  ┌────────┴─────────┐  ┌────────┴─────────┐  │
│  │  Memory Layer     │  │   Service        │  │
│  │  (SQLite 備份)    │  │   (launchd/      │  │
│  │                   │  │    systemd)      │  │
│  └───────────────────┘  └──────────────────┘  │
│                                             │
│           ┌──────────────┐                  │
│           │  Claude Code  │                  │
│           │  CLI (PTY)    │                  │
│           │  + Telegram   │                  │
│           │    Plugin     │                  │
│           └──────────────┘                  │
└─────────────────────────────────────────────┘
```

### Process Manager

透過 `node-pty` 起 Claude Code，啟用 channel 模式。處理 session 持久化（UUID resume）、優雅關閉（`/exit`），以及可設定退避策略的自動重啟。

### Context Guardian

透過 Claude Code 的 status line JSON 監控 context window 使用量。當使用量超過設定的閾值或 session 超過最大年齡時觸發輪替。支援三種策略：`status_line`、`timer` 或 `hybrid`。

### Memory Layer

使用 chokidar 監控 Claude 的記憶目錄，將檔案備份到 SQLite，確保 session 輪替後記憶不會遺失。

### Service Installer

產生並安裝系統服務檔案 — macOS 用 launchd plist，Linux 用 systemd unit。開機自動啟動。

## 設定

設定檔路徑：`~/.claude-channel-daemon/config.yaml`

```yaml
channel_plugin: telegram@claude-plugins-official
working_directory: /path/to/your/project

restart_policy:
  max_retries: 10
  backoff: exponential  # 或 linear
  reset_after: 300      # 穩定運行幾秒後重設重試計數器

context_guardian:
  threshold_percentage: 80  # context 達到此 % 時輪替
  max_age_hours: 4          # session 最大存活時間
  strategy: hybrid          # status_line | timer | hybrid

memory:
  auto_summarize: true
  watch_memory_dir: true
  backup_to_sqlite: true

log_level: info  # debug | info | warn | error
```

## 資料目錄

所有狀態儲存在 `~/.claude-channel-daemon/`：

| 檔案 | 用途 |
|------|------|
| `config.yaml` | 主設定檔 |
| `daemon.pid` | Process ID（運行中） |
| `session-id` | 儲存的 UUID，用於 session resume |
| `statusline.json` | 目前 context/費用狀態 |
| `claude-settings.json` | 注入的 Claude Code 設定 |
| `memory.db` | SQLite 記憶備份 |
| `.env` | Telegram bot token |

## 權限控制

**這個 daemon 使用 `bypassPermissions` 模式。** Claude Code 內建的權限提示完全關閉，所有存取控制由三層機制處理：

1. **PreToolUse hook** — 每次工具調用都會 POST 到 Telegram plugin 的批准 server（`127.0.0.1:18321`），由 server 決定放行或拒絕。

2. **危險偵測** — 批准 server 用 regex 分類操作：
   - **安全**（自動放行）：唯讀操作、安全的 bash 指令、網頁搜尋
   - **危險**（需要 Telegram 批准）：`rm`、`sudo`、`git push`、`chmod`、敏感路徑（`.env`、`.claude/settings.json`）
   - **硬性拒絕**：`rm -rf /`、`git push --force`、`git reset --hard`、`dd`、`mkfs`

3. **Health check + 故障安全** — daemon 每 30 秒 ping 批准 server。如果連不上：
   - 所有工具調用會被**拒絕**（不是放行）
   - 每 60 秒記錄警告，直到恢復

**為什麼用 `bypassPermissions`？** Claude Code 有內部保護路徑（例如 `~/.claude/skills/`），即使 hook 回傳 allow，仍會跳終端機權限提示。在無人值守的 daemon 環境下，這些提示會永遠卡住。`bypassPermissions` 把所有決策交給 hook 層，避免這個問題。

**風險：** 如果有人能存取 `127.0.0.1:18321`，就能批准任意操作。server 只在 localhost 監聽，並且會驗證 Telegram access allowlist。

## 系統需求

- Node.js >= 20
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) CLI 已安裝
- Telegram bot token（透過 [@BotFather](https://t.me/BotFather) 建立）

## 授權

MIT
