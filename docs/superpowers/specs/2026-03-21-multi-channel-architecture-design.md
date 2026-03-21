# Multi-Channel Architecture Design

## Problem

claude-channel-daemon 目前硬綁官方 `telegram@claude-plugins-official` plugin，存在三個問題：

1. **多專案需求** — 想用多個 Telegram bot 對應不同專案，各自獨立運行
2. **官方 plugin 風險** — 官方升級可能破壞流程，且分散在不同 plugin 中無法統一掌控
3. **多平台擴展** — 未來要接 Discord 等其他 channel，需要統一抽象層

## Architecture Overview

```
┌─────────────────────────────────────────────┐
│  Fleet Manager (ccd fleet start/stop/status) │
│  讀取 fleet.yaml，spawn N 個 daemon process  │
└──────┬──────────┬──────────┬────────────────┘
       ▼          ▼          ▼
   [daemon-A] [daemon-B] [daemon-C]  ← 各自獨立 process
```

每個 daemon instance 內部：

```
┌──────────────────────────────────────────┐
│           Channel Adapters               │
│  ┌──────────┐ ┌──────────┐ ┌─────────┐  │
│  │ Telegram  │ │ Telegram │ │ Discord │  │
│  │ (bot A)   │ │ (bot B)  │ │ (未來)  │  │
│  └─────┬─────┘ └────┬─────┘ └────┬────┘  │
│        └──────┬──────┘────────────┘       │
│               ▼                           │
│        Message Bus                        │
│    (匯流 inbound / 路由 outbound /        │
│     approval race)                        │
│               ▼                           │
│        MCP Channel Server (local plugin)  │
│    (reply / react / edit / download)      │
│               ▼                           │
│        Daemon Core                        │
│    ┌──────────────────────────┐           │
│    │ Approval Server (HTTP)   │           │
│    │ PTY Detector             │           │
│    │ → messageBus.requestApproval()       │
│    ├──────────────────────────┤           │
│    │ Process Manager (node-pty)│          │
│    │ Context Guardian          │          │
│    │ Memory Layer → SQLite     │          │
│    └──────────────────────────┘           │
└──────────────────────────────────────────┘
```

## Decision: Multi-instance Model

每個 daemon instance = 1 個專案 + 1~N 個 channel adapter + 1 個 Claude session。

採用 **多 process** 而非單 process 管多專案：
- Claude Code 一次只能跑一個 session，多專案必然是多個 `claude` process
- 故障隔離 — 一個專案掛了不影響其他
- Fleet manager 只是啟停和監控的薄 wrapper

## Channel Abstraction Layer

### ChannelAdapter Interface

```typescript
interface ChannelAdapter {
  readonly type: string;         // "telegram" | "discord" | ...
  readonly id: string;           // unique adapter ID within instance

  // Lifecycle
  start(): Promise<void>;
  stop(): Promise<void>;

  // Inbound — adapter 收到訊息時 emit
  on(event: 'message', handler: (msg: InboundMessage) => void): void;

  // Outbound
  sendText(chatId: string, text: string, opts?: SendOpts): Promise<SentMessage>;
  sendFile(chatId: string, filePath: string): Promise<SentMessage>;
  editMessage(chatId: string, messageId: string, text: string): Promise<void>;
  react(chatId: string, messageId: string, emoji: string): Promise<void>;

  // Approval — 送審批按鈕，回傳 cancel handle
  sendApproval(chatId: string, prompt: string,
    callback: (decision: 'approve' | 'deny') => void,
    signal?: AbortSignal): ApprovalHandle;

  // Attachment
  downloadAttachment(fileId: string): Promise<string>;

  // Access control
  handlePairing(chatId: string, userId: string): Promise<string>;
  confirmPairing(code: string): Promise<boolean>;
}

// sendApproval 回傳的 handle，用於取消
interface ApprovalHandle {
  cancel(): void;  // 撤回審批按鈕，late clicks 靜默忽略
}

interface SendOpts {
  replyTo?: string;         // message ID to reply to
  format?: 'text' | 'markdown';
  chunkLimit?: number;      // max chars per message (default 4096 for Telegram)
}

interface SentMessage {
  messageId: string;
  chatId: string;
}

interface OutboundMessage {
  text?: string;
  filePath?: string;
  replyTo?: string;
  format?: 'text' | 'markdown';
}
```

### InboundMessage

```typescript
interface InboundMessage {
  source: string;              // adapter type
  adapterId: string;           // which adapter instance
  chatId: string;
  messageId: string;
  userId: string;
  username: string;
  text: string;
  timestamp: Date;
  attachments?: Attachment[];
  replyTo?: string;
}
```

### Attachment

```typescript
interface Attachment {
  kind: 'photo' | 'document' | 'audio' | 'voice' | 'video' | 'sticker';
  fileId: string;
  localPath?: string;          // photo/voice 自動下載
  mime?: string;
  size?: number;
  filename?: string;
  transcription?: string;      // voice → STT result
}
```

Multimedia strategy:
- **Photo** — adapter 自動下載到 inbox，`localPath` 直接可用
- **Voice** — 自動下載 + 語音轉文字（Groq/Whisper），結果放 `transcription`
- **Document/Video/Audio** — 不自動下載，Claude 需要時透過 `download_attachment` 拉取
- **Sticker** — 下載 webp 或轉 description

### MessageBus

匯流多個 adapter 的 inbound，路由 outbound，處理 approval race。

```typescript
class MessageBus {
  private adapters: ChannelAdapter[] = [];

  register(adapter: ChannelAdapter): void;
  unregister(adapterId: string): void;

  // Inbound: 所有 adapter 的訊息匯流
  on(event: 'message', handler: (msg: InboundMessage) => void): void;

  // Outbound: 指定 target adapter+chat 或 broadcast
  send(target: Target, msg: OutboundMessage): Promise<void>;

  // Approval: 發到所有 channel，先回的算數
  requestApproval(prompt: string): Promise<ApprovalResponse>;
}

interface ApprovalResponse {
  decision: 'approve' | 'deny';
  respondedBy: { channelType: string; userId: string };
}

interface Target {
  adapterId?: string;          // 指定 adapter，省略則 broadcast
  chatId: string;
}
```

**Approval race 機制：**
1. `requestApproval` 建立一個 `AbortController`
2. 同時呼叫所有 adapter 的 `sendApproval()`，傳入 `AbortSignal`
3. 任一 adapter 的 callback 觸發 → resolve Promise + abort 其餘
4. 被 abort 的 adapter 呼叫自身 `ApprovalHandle.cancel()` 撤回按鈕
5. Abort 後的 late click 靜默忽略（adapter 檢查 signal.aborted）
6. Timeout 2 分鐘 → abort all + auto-deny

## Built-in MCP Channel Server

Daemon 內建 MCP server，取代官方 telegram plugin。

### 連接機制：Local Plugin via --plugin-dir

Claude Code 的 `--channels` 只接受 `plugin:<name>@<registry>` 格式。解決方案：將 MCP channel server 打包為 local plugin，透過 `--plugin-dir` 載入。

**Plugin 目錄結構（daemon build output 的一部分）：**

```
dist/plugin/ccd-channel/
├── .claude-plugin/
│   └── plugin.json          # { "name": "ccd-channel", "version": "..." }
├── .mcp.json                # MCP server 定義
└── server.js                # MCP channel server entry point
```

`.mcp.json`:
```json
{
  "ccd-channel": {
    "command": "node",
    "args": ["${CLAUDE_PLUGIN_ROOT}/server.js"],
    "env": {
      "CCD_INSTANCE_DIR": "${CCD_INSTANCE_DIR}",
      "CCD_APPROVAL_PORT": "${CCD_APPROVAL_PORT}"
    }
  }
}
```

**Claude Code 啟動指令：**
```bash
claude --plugin-dir dist/plugin \
       --channels plugin:ccd-channel \
       --settings <instance-dir>/claude-settings.json \
       --resume <session-id>
```

`--plugin-dir dist/plugin` 讓 Claude Code 發現 `ccd-channel` 這個 local plugin。
`--channels plugin:ccd-channel` 將它作為 channel server 載入。

**MCP server 與 daemon 的通訊：**

MCP server（`server.js`）作為 Claude Code 的 child process 運行，透過 stdin/stdout 走 MCP protocol。
它需要與 daemon 的 MessageBus 通訊。方案：透過 Unix domain socket 或 localhost TCP：

```
daemon process                    Claude Code process
  │                                   │
  ├── MessageBus                      ├── MCP channel server (server.js)
  │     ↕ (Unix socket)               │     ↕ (MCP stdio)
  │     IPC bridge ←──────────────────┤     Claude
  │                                   │
```

Daemon 啟動時建立 IPC server（Unix socket at `<instance-dir>/channel.sock`）。
MCP server.js 啟動時連接此 socket，雙向傳遞 inbound messages 和 outbound tool calls。

### MCP Tools (channel-agnostic)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `reply` | `chat_id`, `text`, `files?`, `reply_to?` | Send message |
| `react` | `chat_id`, `message_id`, `emoji` | Add reaction |
| `edit_message` | `chat_id`, `message_id`, `text` | Edit message |
| `download_attachment` | `file_id` | Download and return local path |

Tool interface 與官方 plugin 一致 — Claude 端無需學習新 API。底層改為透過 IPC → MessageBus → ChannelAdapter。

**MCP tool 命名：** `mcp__plugin_ccd-channel_ccd-channel__reply` 等。Settings 中的 tool allow-list 需對應更新。

### Inbound Message Injection

Adapter 收到使用者訊息 → MessageBus → IPC → MCP server 透過 channel protocol 推送給 Claude。格式沿用 `<channel source="..." chat_id="..." ...>` tag。

### Approval Integration

- PreToolUse hook POST 到 approval server endpoint
- Approval server 呼叫 `messageBus.requestApproval()` 而非直接呼叫 Telegram API
- PTY prompt detector 也呼叫 `messageBus.requestApproval()`
- 統一的 approval 路徑，消除 daemon/plugin 間的邏輯分裂

## Access Control

每個 adapter 獨立管理自己的 access：

```typescript
interface AccessConfig {
  mode: 'pairing' | 'locked';
  allowed_users: number[];
  max_pending_codes: number;      // default 3
  code_expiry_minutes: number;    // default 60
}
```

State machine:
- `pairing` — 新用戶 DM 時發 pairing code，確認後加入 `allowed_users`
- `locked` — 只有 `allowed_users` 能用，unknown sender 直接丟棄

**Pairing 流程（端到端）：**
1. 新用戶 DM bot → adapter 收到訊息，userId 不在 `allowed_users`
2. Adapter 呼叫 `accessManager.generateCode(userId)` → 產生 6 字元 hex code
3. Bot 回覆用戶：「Pairing code: `A3F7B2`，請在終端機執行 `ccd access <instance> pair A3F7B2`」
4. Max 2 次 pairing 回覆/sender，之後靜默丟棄（防濫用）
5. Code 有效期 60 分鐘，最多 3 個 pending codes
6. Operator 在終端執行 `ccd access <instance> pair A3F7B2`
7. AccessManager 驗證 code → 將 userId 加入 `allowed_users` → 持久化到 access state file
8. Bot 通知用戶：「Paired successfully」

Management:
```
ccd access <instance> lock          # 切到 locked mode
ccd access <instance> unlock        # 切回 pairing mode
ccd access <instance> list          # 列出 allowed_users
ccd access <instance> remove <uid>  # 移除用戶
ccd access <instance> pair <code>   # 確認 pairing
```

Security: channel 內訊息要求改 access 一律拒絕（防 prompt injection）。

## Fleet Management

### Fleet Config: `~/.claude-channel-daemon/fleet.yaml`

```yaml
defaults:
  restart_policy:
    max_retries: 10
    backoff: exponential
    reset_after: 300
  context_guardian:
    threshold_percentage: 80
    max_age_hours: 4
    strategy: hybrid
  memory:
    watch_memory_dir: true
    backup_to_sqlite: true
  log_level: info

instances:
  project-a:
    working_directory: /path/to/project-a
    channels:
      - type: telegram
        bot_token_env: PROJECT_A_BOT_TOKEN
        access:
          mode: pairing
          allowed_users: [123456789]
    # override defaults
    context_guardian:
      threshold_percentage: 60

  project-b:
    working_directory: /path/to/project-b
    channels:
      - type: telegram
        bot_token_env: PROJECT_B_BOT_TOKEN
        access:
          mode: locked
          allowed_users: [123456789, 987654321]
```

**Config merge 規則：**
- `defaults` 中的 object fields（restart_policy, context_guardian, memory）透過 deep merge 與 per-instance override 合併
- `channels` 是 array — **不 merge，直接替換**（每個 instance 定義自己完整的 channels list）
- Bot token 不直接寫 yaml — `bot_token_env` 指向環境變數，安全性更好

**Backward compatibility：** 單機模式 `ccd start` 仍讀取 `config.yaml`（舊格式）。新的 `InstanceConfig` type 同時支援舊的 `channel_plugin: string` 和新的 `channels: ChannelConfig[]`，遷移期間兩者共存。

### Config Types

```typescript
interface FleetConfig {
  defaults: Partial<InstanceConfig>;
  instances: Record<string, InstanceConfig>;
}

interface InstanceConfig {
  working_directory: string;
  channels: ChannelConfig[];
  restart_policy: RestartPolicy;
  context_guardian: ContextGuardianConfig;
  memory: MemoryConfig;
  log_level: string;
  // Deprecated: backward compat with old config.yaml
  channel_plugin?: string;
}

interface ChannelConfig {
  type: 'telegram';             // | 'discord' | ... (future)
  bot_token_env: string;
  access: AccessConfig;
  // Platform-specific options
  options?: Record<string, unknown>;
}
```

### CLI Commands

```
# Fleet management
ccd fleet start                  # Start all instances
ccd fleet stop                   # Stop all instances
ccd fleet start <instance>       # Start single instance
ccd fleet stop <instance>        # Stop single instance
ccd fleet status                 # List all instance states
ccd fleet logs <instance>        # Tail specific instance log

# Single-instance mode (backward compatible)
ccd start                        # Uses config.yaml, old behavior
```

### Process Management

- 每個 instance 是獨立 child process (fork)
- **Instance-scoped data directory**: `~/.claude-channel-daemon/instances/<name>/`
  - `daemon.pid` — process ID
  - `daemon.log` — structured JSON logs
  - `session-id` — Claude session UUID for --resume
  - `statusline.json` — Claude status JSON
  - `claude-settings.json` — per-instance settings (unique approval port, tool allow-list)
  - `channel.sock` — Unix domain socket for MCP ↔ daemon IPC
  - `access/` — per-adapter access state files

**注意：** 現有代碼中 `DATA_DIR`、`SESSION_FILE`、`PID_PATH`、`LOG_PATH` 等全域路徑需重構為接受 instance-scoped 路徑。這影響 `ProcessManager`、`ContextGuardian`、`cli.ts` — 是跨模組的變更。

### Approval Server Port Allocation

每個 instance 需要獨立的 approval server port（PreToolUse hook 透過 HTTP POST 觸發）。

**策略：**
- Fleet config 可選指定 `approval_port`，不指定則自動分配
- 自動分配：base port 18321 + instance index（按 fleet.yaml 中的宣告順序）
- 每個 instance 的 `claude-settings.json` 中的 PreToolUse hook curl 指令使用對應的 port
- 單機模式沿用 18321

```yaml
instances:
  project-a:            # auto: port 18321
    ...
  project-b:            # auto: port 18322
    ...
  project-c:
    approval_port: 19000  # manual override
    ...
```

### Fleet Status Output

```
ccd fleet status

Instance     Status      Uptime    Context   Channel
─────────────────────────────────────────────────────
project-a    running     2h 15m    42%       telegram (bot-a)
project-b    running     0h 30m    12%       telegram (bot-b)
project-c    crashed     -         -         telegram (bot-c)
```

Status 判斷：
- `running` — PID file 存在且 process alive
- `stopped` — PID file 不存在
- `crashed` — PID file 存在但 process dead

### Service Installation

`ccd fleet install` 產生一個 launchd plist / systemd service，執行 `ccd fleet start`。一個 service 管整個 fleet，加新專案只改 `fleet.yaml`。

## Module Structure

```
src/
├── cli.ts                     # CLI entry (ccd start/stop/fleet/access)
├── fleet-manager.ts           # fleet start/stop/status, spawn child processes
├── daemon.ts                  # Single instance main logic
├── process-manager.ts         # PTY management, session resume (existing, adapted)
├── context-guardian.ts        # Context rotation (existing, path-parameterized)
├── memory-layer.ts            # Memory backup (existing, unchanged)
├── db.ts                      # SQLite (existing, unchanged)
├── config.ts                  # Read fleet.yaml + deep merge defaults
├── logger.ts                  # Pino logging (existing, unchanged)
│
├── channel/
│   ├── types.ts               # ChannelAdapter, InboundMessage, Attachment, etc.
│   ├── message-bus.ts         # MessageBus — merge inbound / route outbound / approval race
│   ├── mcp-server.ts          # Built-in MCP server entry (runs as Claude child process)
│   ├── ipc-bridge.ts          # Unix socket IPC between daemon ↔ MCP server
│   ├── access-manager.ts      # Pairing / locked state machine, allowlist
│   └── adapters/
│       └── telegram.ts        # TelegramAdapter implements ChannelAdapter
│
├── approval/
│   ├── approval-server.ts     # HTTP server (PreToolUse hook endpoint)
│   └── pty-detector.ts        # PTY prompt detection (extracted from cli.ts)
│
├── types.ts                   # Global types (FleetConfig, InstanceConfig, ChannelConfig)
│
└── plugin/                    # Local plugin structure (built output)
    └── ccd-channel/
        ├── .claude-plugin/
        │   └── plugin.json
        ├── .mcp.json
        └── server.js          # → compiled from channel/mcp-server.ts
```

### Dependency Changes

- **Add**: `@modelcontextprotocol/sdk` (build MCP server), `grammy` (Telegram Bot API)
- **Remove**: dependency on `telegram@claude-plugins-official` plugin

### Existing Code Impact

| Current File | Change |
|-------------|--------|
| `cli.ts` | Split → `cli.ts` (pure CLI) + `daemon.ts` (instance logic) + `fleet-manager.ts` |
| `process-manager.ts` | Refactor: accept instance-scoped paths, remove Telegram hardcoding, use `messageBus` |
| `context-guardian.ts` | Minor: accept instance-scoped `statusline.json` path |
| `memory-layer.ts` | Unchanged |
| `db.ts` | Unchanged |
| `config.ts` | Major: support `fleet.yaml` + `InstanceConfig` + backward compat with `config.yaml` |
| `types.ts` | Major: add `FleetConfig`, `InstanceConfig`, `ChannelConfig`, deprecate `DaemonConfig.channel_plugin` |
| `setup-wizard.ts` | Adapt for fleet init + channel type selection |

**Cross-cutting concern:** `DATA_DIR` and all derived paths (`SESSION_FILE`, `PID_PATH`, `LOG_PATH`, `STATUSLINE_FILE`, settings file path) must be refactored from global constants to instance-scoped parameters. This affects `ProcessManager`, `ContextGuardian`, `daemon.ts`, and settings file generation.

## Scope

### In Scope (this iteration)
- Channel abstraction layer (interface + MessageBus + IPC bridge)
- Telegram adapter (replaces official plugin)
- Local plugin structure + MCP channel server
- Unified approval system with approval race
- Fleet management (fleet.yaml + CLI)
- Instance-scoped data directories
- Access control with pairing + locked modes
- Multimedia support (photo, voice, document)
- Backward compatible single-instance mode

### Out of Scope (future)
- Discord adapter
- Cross-channel message forwarding
- Adapter hot-plug (runtime add/remove without restart)
- Slack / other platform adapters
