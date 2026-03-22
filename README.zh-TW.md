# claude-channel-daemon

讓你的 Claude Code Telegram bot 不用顧著終端機也能一直跑。這個 daemon 把 `claude --channels` 包成背景服務，掛了自動重啟，context 快滿就換新 session，記憶自動備份到 SQLite。

[English README](README.md)

> **⚠️ 注意：** daemon 會預先放行大部分工具，危險操作（rm、git push --force 之類的）會透過 Telegram 按鈕讓你確認。批准 server 連不上的話，所有工具呼叫都會被擋。跑之前先看[權限機制](#權限機制)。

## 為什麼要做這個

Claude Code 的 Telegram plugin 需要一個活著的 CLI session。終端機關掉，bot 就斷了。測試可以這樣搞，正式用不行。

這個 daemon 處理的事：

- 用 `node-pty` 開 pseudo-terminal 在背景跑 Claude Code
- 掛了就自動重啟，間隔越來越長（1 秒、2 秒、4 秒... 最多 60 秒）
- 監控 context window 用量，快滿就砍掉重開
- 記憶檔案有變動就自動備份到 SQLite
- 可以裝成 launchd（macOS）或 systemd（Linux）服務

## 開始用

```bash
git clone https://github.com/suzuke/claude-channel-daemon.git
cd claude-channel-daemon
npm install && npm link

# 互動式設定（bot token、工作目錄、要不要裝服務）
ccd init

# 開跑
ccd start
```

`npm link` 之後就有 `ccd` 指令可以用。

## 指令

```
ccd start       啟動
ccd stop        停止
ccd status      看有沒有在跑
ccd logs        看 log（-n 50 看幾行、-f 即時追蹤）
ccd install     裝成系統服務
ccd uninstall   移除服務
ccd init        互動式設定
```

## 架構

```
┌─────────────────────────────────────────────┐
│              claude-channel-daemon           │
│                                             │
│  ┌─────────────────┐  ┌──────────────────┐  │
│  │ Process Manager  │  │ Context Guardian │  │
│  │ (node-pty)       │  │ (自動換 session) │  │
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

**Process Manager** — 開 PTY 跑 `claude --channels plugin:telegram@...`。掛了就等一下重啟。會記住 session UUID，crash 後可以 `--resume` 接回去。關閉時送 `/exit` 讓 Claude 正常結束。

**Context Guardian** — 讀 Claude Code 的 status line JSON（daemon 啟動時自動注入一個 statusline script）。`context_window.used_percentage` 超過門檻（預設 40%）就砍掉重開新 session。順便拿到 `rate_limits` 跟 `cost`，不花任何 API 額度。

**Memory Layer** — 用 chokidar 盯著 `~/.claude/projects/.../memory/`。記憶檔一有變動就複製到 SQLite 存起來。Session 重啟後 Claude Code 會自己去讀 memory 目錄，不用額外處理。

**Service Installer** — 幫你產生 launchd plist 或 systemd unit 檔，跟你說怎麼啟用。

## 設定

設定檔在 `~/.claude-channel-daemon/config.yaml`：

```yaml
channel_plugin: telegram@claude-plugins-official
working_directory: /path/to/your/project

restart_policy:
  max_retries: 10
  backoff: exponential  # 或 linear
  reset_after: 300      # 穩定跑 5 分鐘後歸零重試計數

context_guardian:
  threshold_percentage: 40  # 到這個 % 就換 session
  max_age_hours: 4          # 最久跑幾小時就強制換
  strategy: hybrid          # status_line | timer | hybrid

memory:
  auto_summarize: true
  watch_memory_dir: true
  backup_to_sqlite: true

log_level: info
```

## 資料目錄

都放在 `~/.claude-channel-daemon/`：

| 檔案 | 幹嘛的 |
|------|--------|
| `config.yaml` | 主設定 |
| `daemon.pid` | PID 檔（跑的時候才有） |
| `daemon.log` | log 輸出（也會印到 stdout） |
| `session-id` | 存的 UUID，給 `--resume` 用 |
| `statusline.json` | Claude Code 最新的狀態資料 |
| `claude-settings.json` | 注入給 Claude session 的設定 |
| `statusline.sh` | 接 status line JSON 的 shell script |
| `memory.db` | 記憶檔的 SQLite 備份 |

## 權限機制

Claude Code 有兩套獨立的權限系統。headless daemon 兩個都要處理，不然 session 會卡住。踩了不少坑才搞清楚。

### 工具層級的權限

Claude Code 在 session 裡第一次用到 Edit、Bash 或 MCP tool 的時候會問你要不要放行。在終端機你可以按 allow，daemon 裡沒人按。

我們在 `claude-settings.json` 裡預先放行所有工具：

```
Read, Edit, Write, Glob, Grep, Bash(*), WebFetch, WebSearch, Agent, Skill,
mcp__plugin_telegram_telegram__reply, react, edit_message
```

這樣工具層級的提示就不會出現了。

### 危險操作攔截（PreToolUse hook）

每個工具呼叫都會先 POST 到 Telegram plugin 內建的 HTTP server（`127.0.0.1:18321/approve`）。

Server 會判斷這個操作危不危險：

| 操作 | 結果 |
|------|------|
| `ls`、`grep`、讀檔 | 自動放行 |
| `rm`、`sudo`、`git push`、`chmod` | 發 Telegram 訊息，有 ✅/❌ 按鈕讓你選 |
| 改 `.env`、`.claude/settings.json` | 一樣，按鈕 |
| `rm -rf /`、`dd`、`mkfs` | 設定檔直接擋，不會到 server |
| Server 連不上 | 擋掉（fail-closed） |

判斷用的是 regex：

```typescript
const DANGEROUS_BASH = [
  /(?:rm|rmdir)\s/i,
  /(?:sudo|kill|killall|pkill)\s/i,
  /git\s+push/i,
  /git\s+reset\s+--hard/i,
  // ...
]
```

### 硬編碼路徑保護（最麻煩的部分）

Claude Code 對 `.git/`、`.claude/`、`.vscode/`、`.idea/` 的寫入有內建保護。就算用 `acceptEdits` 模式、就算 hook 回 allow，它還是會在終端機跳確認。

Daemon 裡這個確認會永遠卡住。所以我們從 PTY 輸出偵測到提示（會出現 "1.Yes 2.Yes,andallow... 3.No"），然後轉發到 Telegram 讓你按按鈕。按了之後 daemon 在 PTY 裡打 "1" 或 "3"。兩分鐘沒按就自動拒絕。

### 為什麼不用 `bypassPermissions`？

試過了。它會連 plugin 都不載入，Telegram plugin 直接不啟動，bot 完全收不到訊息。文件上說 bypass 只是跳過提示，但實際上它也擋了 MCP server 的啟動。看起來是 Claude Code 的 bug。所以改用 `acceptEdits` 模式，剩下的 edge case 用 PTY 偵測處理。

### 整體流程

```
Claude 要用一個工具
    │
    ├─ permissions.allow → 工具在清單裡？→ 是 → 繼續
    │
    ├─ PreToolUse hook → POST 到 approval server
    │   安全操作 → 放行
    │   危險操作 → Telegram 按鈕 → 你決定
    │   server 掛了 → 擋掉
    │
    └─ 硬編碼路徑保護
        PTY 出現確認提示 → 轉發到 Telegram → 你決定
```

## 系統需求

- Node.js >= 20
- Claude Code CLI
- Telegram bot token（[@BotFather](https://t.me/BotFather)）

## 已知問題

- ~~不要在 cmux 裡面跑~~ 已修復：daemon 會設定 `CMUX_CLAUDE_HOOKS_DISABLED=1` 停用 cmux 的 `--settings` 注入
- `bypassPermissions` 模式會讓 plugin 不載入（Claude Code bug）
- 目前只在 macOS 測過

## 授權

MIT
