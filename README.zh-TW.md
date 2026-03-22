# claude-channel-daemon

用一個 Telegram bot 跑多個 Claude Code session，每個 Forum Topic 對應一個獨立的專案。內建批准系統、語音轉文字、自動 context 輪替、crash 自動恢復。

[English README](README.md)

> **⚠️ 注意：** daemon 會預先放行大部分工具，危險的 Bash 指令（rm、sudo、git push...）會透過 Telegram 按鈕讓你確認。批准 server 連不上的話，危險操作會被擋。詳見[權限機制](#權限機制)。

## 為什麼要做這個

Claude Code 的官方 Telegram plugin 是 1 bot = 1 session。終端機關掉，bot 就斷了。

這個 daemon 解決的問題：

- **Fleet 模式** — 1 個 Telegram bot、N 個 Forum Topics = N 個獨立 Claude session
- **tmux 架構** — Claude 跑在 tmux window 裡，daemon crash 也不影響
- **自動 context 輪替** — 到 60% context 就等 Claude 空閒，讓它存狀態後換新 session
- **語音訊息** — Telegram 語音 → Groq Whisper → 文字送 Claude
- **批准系統** — 危險 Bash 指令會送 Telegram inline 按鈕讓你決定
- **自動 Topic 綁定** — 在 Telegram 開個 topic，選專案目錄，搞定
- **系統服務** — 裝成 launchd（macOS）或 systemd（Linux）

## 開始用

```bash
git clone https://github.com/suzuke/claude-channel-daemon.git
cd claude-channel-daemon
npm install && npm link

# 需要：claude CLI + tmux
brew install tmux  # macOS

# 互動式設定
ccd init

# 啟動 fleet
ccd fleet start
```

## 指令

```
ccd init                  互動式設定精靈
ccd fleet start           啟動所有 instance
ccd fleet stop            停止所有 instance
ccd fleet status          看 instance 狀態
ccd fleet logs <name>     看 instance log
ccd fleet start <name>    啟動特定 instance
ccd fleet stop <name>     停止特定 instance
ccd topic list            列出 topic 綁定
ccd topic bind <n> <tid>  綁定 instance 到 topic
ccd topic unbind <n>      解除 topic 綁定
ccd access lock <n>       鎖定 instance 存取
ccd access unlock <n>     開放 instance 存取
ccd access list <n>       列出允許的使用者
ccd access remove <n> <uid> 移除使用者
ccd access pair <n> <uid> 產生配對碼
ccd install               裝成系統服務
ccd uninstall             移除系統服務
```

## 架構

```
┌──────────────────────────────────────────────────────────┐
│                    Fleet Manager                         │
│                                                          │
│  共用 TelegramAdapter（1 bot，Grammy long-polling）       │
│         │                                                │
│    threadId 路由表：#277→proj-a、#672→proj-b             │
│         │                                                │
│  ┌──────┴──────┐  ┌──────┴──────┐  ┌──────┴──────┐     │
│  │  Daemon A    │  │  Daemon B    │  │  Daemon C    │     │
│  │  IPC Server  │  │  IPC Server  │  │  IPC Server  │     │
│  │  Approval    │  │  Approval    │  │  Approval    │     │
│  │  Context     │  │  Context     │  │  Context     │     │
│  │  Guardian    │  │  Guardian    │  │  Guardian    │     │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘     │
│         │                  │                  │            │
│  ┌──────┴───────┐  ┌──────┴───────┐  ┌──────┴───────┐     │
│  │ tmux window   │  │ tmux window   │  │ tmux window   │     │
│  │ claude        │  │ claude        │  │ claude        │     │
│  │ + MCP server  │  │ + MCP server  │  │ + MCP server  │     │
│  └───────────────┘  └───────────────┘  └───────────────┘     │
└──────────────────────────────────────────────────────────┘
```

**Fleet Manager** — 擁有共用的 Telegram adapter。根據 `message_thread_id` 把訊息路由到正確的 daemon instance（透過 IPC）。處理 topic 自動建立、自動綁定（目錄瀏覽器）跟自動解除綁定。

**Daemon** — 每個 instance 的管理器。管理一個跑 Claude Code 的 tmux window（加上 `--dangerously-load-development-channels server:ccd-channel`）。跑批准 server、context guardian、transcript monitor。

**MCP Channel Server** — 作為 Claude 的子程序跑。透過 Unix socket IPC 跟 daemon 通訊。宣告 `claude/channel` capability，用 `notifications/claude/channel` 推送訊息。IPC 斷線會自動重連。

**Context Guardian** — 監控 Claude 的 status line JSON。是個 5 狀態的 state machine：NORMAL → PENDING（超過門檻，等 Claude 空閒）→ HANDING_OVER（送 prompt 讓 Claude 把狀態存到 `memory/handover.md`）→ ROTATING（砍 window，開新 session）→ GRACE（10 分鐘冷卻期）。預設門檻 60%，也會在 `max_age_hours`（預設 8h）後強制輪替。

**Memory Layer** — 用 chokidar 監控 `~/.claude/projects/.../memory/`。記憶檔有變動就備份到 SQLite。Session 重啟後 Claude 會自己讀 memory 目錄。

## 設定

Fleet 設定檔在 `~/.claude-channel-daemon/fleet.yaml`：

```yaml
project_roots:
  - ~/Projects

channel:
  type: telegram
  mode: topic           # topic（推薦）或 dm
  bot_token_env: CCD_BOT_TOKEN
  group_id: -100xxxxxxxxxx
  access:
    mode: locked         # locked 或 pairing
    allowed_users:
      - 123456789        # 你的 Telegram user ID

defaults:
  context_guardian:
    threshold_percentage: 60
    max_age_hours: 8
    max_idle_wait_ms: 300000
    completion_timeout_ms: 60000
    grace_period_ms: 600000
  log_level: info

instances:
  my-project:
    working_directory: /path/to/project
    topic_id: 277
```

Bot token 放在 `~/.claude-channel-daemon/.env`：
```
CCD_BOT_TOKEN=123456789:AAH...
GROQ_API_KEY=gsk_...          # 選用，語音轉文字用
```

## 權限機制

### 工具權限

所有工具都在每個 instance 的 `claude-settings.json` 裡預先放行：
```
Read, Edit, Write, Glob, Grep, Bash(*), WebFetch, WebSearch, Agent, Skill,
mcp__ccd-channel__reply, react, edit_message, download_attachment
```

### 危險操作攔截

PreToolUse hook（matcher: `"Bash"`）把 Bash 指令轉發到批准 server。Server 用 regex 判斷危險程度：

| 操作 | 結果 |
|------|------|
| `ls`、`cat`、`npm install` | 自動放行 |
| `rm`、`mv`、`sudo`、`kill`、`git push/reset/clean` | Telegram 按鈕讓你選 |
| `rm -rf /`、`dd`、`mkfs` | 設定檔直接擋 |
| 批准 server 連不上 | 擋掉（fail-closed）|

### 硬編碼路徑保護

Claude Code 對 `.git/`、`.claude/`、`.vscode/`、`.idea/` 的寫入有內建保護，在終端機會跳確認。daemon 透過 tmux 輸出偵測到這個提示，轉發到 Telegram 讓你按按鈕。兩分鐘沒按就自動拒絕。

### 整體流程

```
Claude 要用一個工具
    │
    ├─ permissions.allow → 工具在清單裡？→ 是 → 繼續
    │
    ├─ PreToolUse hook → POST 到批准 server
    │   安全操作 → 放行
    │   危險操作 → IPC → fleet manager → Telegram 按鈕 → 你決定
    │   server 掛了 → 擋掉
    │
    └─ 硬編碼路徑保護
        tmux 輸出偵測到確認提示 → 轉發到 Telegram → 你決定
```

## 資料目錄

`~/.claude-channel-daemon/`：

| 路徑 | 用途 |
|------|------|
| `fleet.yaml` | Fleet 設定 |
| `.env` | Bot token + API keys |
| `fleet.log` | Fleet log（JSON）|
| `instances/<name>/` | 每個 instance 的資料 |
| `instances/<name>/daemon.log` | Instance log |
| `instances/<name>/session-id` | 存的 session UUID，給 `--resume` 用 |
| `instances/<name>/statusline.json` | Claude 最新的狀態資料 |
| `instances/<name>/channel.sock` | IPC Unix socket |
| `instances/<name>/transcript-offset` | Transcript monitor 的 byte offset |
| `instances/<name>/access-state.json` | 存取控制狀態 |
| `instances/<name>/memory.db` | 記憶檔的 SQLite 備份 |
| `instances/<name>/output.log` | Claude tmux 輸出擷取 |

## 系統需求

- Node.js >= 20
- tmux
- Claude Code CLI
- Telegram bot token（[@BotFather](https://t.me/BotFather)）
- Groq API key（選用，語音轉文字用）

## 已知問題

- ~~不要在 cmux 裡面跑~~ 已修復：daemon 會設定 `CMUX_CLAUDE_HOOKS_DISABLED=1` 停用 cmux 的 `--settings` 注入
- 全域 `enabledPlugins` 裡有官方 telegram plugin 會造成 409 polling 衝突（daemon 會自動重試）
- 目前只在 macOS 測過

## 授權

MIT
