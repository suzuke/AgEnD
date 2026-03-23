# Scheduler Design Spec

Fleet-level scheduling system for claude-channel-daemon. Users set up schedules via natural language in Telegram topics; Claude calls MCP tools to register cron jobs; Fleet Manager triggers them and injects messages into target instances.

## Architecture

Scheduler is a module inside Fleet Manager (`src/scheduler.ts`), not a separate process. Fleet Manager already owns the routing table and all IPC connections, making it the natural home for scheduling.

```
Fleet Manager
├── Shared Telegram Adapter
├── Routing Table
└── Scheduler
    ├── croner (cron engine, timezone-aware)
    ├── SQLite (scheduler.db, fleet-level)
    └── Trigger → pushChannelMessage to target instance via IPC
```

## Data Model

Fleet-level SQLite at `~/.claude-channel-daemon/scheduler.db`.

```sql
CREATE TABLE schedules (
  id              TEXT PRIMARY KEY,       -- nanoid / crypto.randomUUID
  cron            TEXT NOT NULL,          -- cron expression, e.g. "0 7 * * *"
  message         TEXT NOT NULL,          -- message content injected on trigger
  source          TEXT NOT NULL,          -- instance that created the schedule
  target          TEXT NOT NULL,          -- resolved target instance (never NULL)
  reply_chat_id   TEXT NOT NULL,          -- Telegram chat ID for replies/notifications
  reply_thread_id TEXT,                   -- Telegram thread ID (NULL for DM mode)
  label           TEXT,                   -- human-readable schedule name
  enabled         INTEGER DEFAULT 1,     -- 0/1
  timezone        TEXT DEFAULT 'Asia/Taipei', -- IANA timezone
  created_at      TEXT NOT NULL,          -- ISO 8601
  last_triggered_at TEXT,                 -- ISO 8601
  last_status     TEXT                    -- 'ok' | 'instance_offline' | 'send_failed'
);

CREATE TABLE schedule_runs (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
  triggered_at TEXT NOT NULL,            -- ISO 8601
  status      TEXT NOT NULL,             -- 'delivered' | 'delivered_fallback' | 'retry' | 'instance_offline' | 'channel_dead'
  detail      TEXT                       -- error message if any
);

CREATE INDEX idx_schedule_runs_schedule_id ON schedule_runs(schedule_id);

-- Retention: prune runs older than 30 days on each scheduler init/reload
-- DELETE FROM schedule_runs WHERE triggered_at < datetime('now', '-30 days');
```

Design decisions:
- `target` is always resolved at creation time (NULL input becomes source instance name).
- `reply_chat_id` / `reply_thread_id` are captured from IPC meta at creation time, decoupling from runtime topology.
- `schedule_runs` provides full audit trail without bloating the main table.
- `ON DELETE CASCADE` on schedule_runs so deleting a schedule cleans up history.

## MCP Tools

Four tools exposed to Claude via MCP server:

### `create_schedule`

```typescript
inputSchema: {
  cron:      string,   // required. cron expression
  message:   string,   // required. message injected on trigger
  target?:   string,   // optional. target instance name, defaults to self
  label?:    string,   // optional. human-readable name
  timezone?: string    // optional. IANA timezone, defaults to Asia/Taipei
}
// reply_chat_id, reply_thread_id auto-filled from IPC meta — not exposed to Claude
```

Returns: `{ id, next_trigger }`.

Validation at creation:
- Target instance must exist in fleet config.
- Cron expression must be parseable by croner.
- Schedule count must not exceed limit (default 100, configurable in fleet.yaml).

### `list_schedules`

```typescript
inputSchema: {
  target?: string   // optional. filter by target instance
}
```

Returns array of schedule objects with `id`, `label`, `cron`, `target`, `enabled`, `timezone`, `next_trigger`, `last_triggered_at`, `last_status`.

### `update_schedule`

```typescript
inputSchema: {
  id:        string,   // required
  cron?:     string,
  message?:  string,
  target?:   string,
  label?:    string,
  timezone?: string,
  enabled?:  boolean
}
```

### `delete_schedule`

```typescript
inputSchema: {
  id: string   // required
}
```

## IPC Protocol

Reuses existing line-delimited JSON IPC format.

### Full message flow: Claude → Fleet Manager (CRUD)

Schedule tool calls follow the same path as existing tools (reply, react, etc.) but require request/response semantics:

```
1. Claude calls tool (e.g. create_schedule)
   ↓
2. MCP server (mcp-server.ts) receives CallToolRequest
   → Recognizes schedule tools as a distinct category
   → Wraps as IPC message: { type: "tool_call", tool: "create_schedule", args: {...}, requestId: "..." }
   → Sends via IPC client to daemon
   ↓
3. Daemon (daemon.ts) receives tool_call on IPC server
   → handleToolCall() detects tool name starts with "*_schedule"
   → Forwards to Fleet Manager as: { type: "fleet_schedule_create", payload: {...}, meta: { chat_id, thread_id, instance_name }, requestId: "..." }
   → (Same relay pattern as existing fleet_outbound / fleet_approval_request)
   ↓
4. Fleet Manager receives fleet_schedule_create
   → Calls scheduler.create(payload, meta)
   → Sends response back to daemon: { type: "fleet_schedule_response", requestId: "...", result: { id, next_trigger } }
   ↓
5. Daemon receives fleet_schedule_response
   → Matches requestId → forwards result to MCP server via IPC
   ↓
6. MCP server resolves pending CallToolRequest with the result
   → Claude receives tool response
```

Key implementation notes:
- Daemon must maintain a `pendingRequests: Map<requestId, callback>` for schedule CRUD (similar to how approval requests are tracked).
- MCP server must register the 4 schedule tools in `ListToolsRequestSchema` handler alongside existing tools.
- In `CallToolRequestSchema` handler, schedule tools route to IPC with request/response pattern (unlike reply/react which are fire-and-forget via adapter).

### IPC message types

```
// CRUD: Claude → daemon → Fleet Manager
fleet_schedule_create   { payload: {...}, meta: { chat_id, thread_id, instance_name }, requestId }
fleet_schedule_list     { payload: { target? }, requestId }
fleet_schedule_update   { payload: { id, ...fields }, requestId }
fleet_schedule_delete   { payload: { id }, requestId }

// CRUD response: Fleet Manager → daemon → MCP server
fleet_schedule_response { requestId, result?, error? }
```

### Fleet Manager → Daemon (trigger)

```
fleet_schedule_trigger  { payload: { schedule_id, message, label }, meta: { chat_id, thread_id, user: "scheduler" } }
```

Trigger reuses the same `pushChannelMessage` path as `fleet_inbound`. The daemon treats it identically — difference is `meta.user = "scheduler"` and message prefixed with `[排程任務]`.

## CLI Commands

```bash
ccd schedule list [--target <instance>] [--json]
ccd schedule add --cron "..." --target <instance> --message "..." [--label "..."] [--timezone "..."]
ccd schedule update <id> [--cron "..."] [--message "..."] [--target "..."] [--enabled true/false]
ccd schedule delete <id>
ccd schedule disable <id>
ccd schedule enable <id>
ccd schedule history <id> [--limit 20]
ccd schedule trigger <id>
```

CLI directly reads/writes `scheduler.db`. After write operations, CLI sends SIGHUP to Fleet Manager PID (read from `~/.claude-channel-daemon/fleet.pid`) to trigger schedule reload. If Fleet Manager is not running (no PID file or process not alive), schedules are persisted and loaded on next startup.

Implementation prerequisite: Fleet Manager must write its PID to `fleet.pid` on startup and clean up on shutdown. This file does not exist today — it must be added to `FleetManager.startAll()` (write) and shutdown handler (unlink). The CLI already writes per-instance `daemon.pid` files via a similar pattern.

SQLite concurrency: Both CLI and Fleet Manager may have the DB open simultaneously. The DB must be opened in WAL mode (`PRAGMA journal_mode=WAL`) by both parties to support concurrent readers/writers safely.

## Scheduler Module

```typescript
// src/scheduler.ts
class Scheduler {
  private db: Database;
  private jobs: Map<string, CronJob>;
  private onTrigger: (schedule: Schedule) => void;

  constructor(dbPath: string, onTrigger: callback);

  init(): void;           // create tables + load enabled schedules + register cron jobs
  reload(): void;         // clear all jobs + reload from DB (SIGHUP handler)
  shutdown(): void;       // stop all cron jobs

  create(params): Schedule;
  list(filter?): Schedule[];
  get(id): Schedule | null;
  update(id, params): Schedule;
  delete(id): void;

  trigger(id): void;                                              // manual trigger
  deleteByInstanceOrThread(instanceName: string, threadId: string): number; // topic deletion cleanup

  recordRun(scheduleId, status, detail?): void;
  getRuns(scheduleId, limit): ScheduleRun[];
}
```

Fleet Manager integration:

```typescript
// In FleetManager.startAll()
this.scheduler = new Scheduler(
  path.join(this.baseDir, 'scheduler.db'),
  (schedule) => this.handleScheduleTrigger(schedule)
);
this.scheduler.init();
process.on('SIGHUP', () => this.scheduler.reload());
```

## Trigger Flow

```
cron fires (croner)
  → Scheduler.onTrigger(schedule)
  → FleetManager.handleScheduleTrigger(schedule)  [async, non-blocking — does not delay other cron triggers]
  → Check target instance IPC connection
     ├─ Connected → send fleet_schedule_trigger → recordRun('delivered')
     │   └─ If cross-instance (source ≠ target):
     │        → Send notification to source topic (reply_chat_id/reply_thread_id):
     │          "⏰ 排程「{label}」已觸發，目標實例：{target}"
     └─ Disconnected → retry 3x at 30s intervals (async setTimeout, non-blocking)
        ├─ Retry succeeds → recordRun('delivered')
        └─ All retries fail → recordRun('instance_offline')
           → Send notification to reply_chat_id/reply_thread_id via Telegram API:
             "⏰ 排程「{label}」觸發失敗：實例 {target} 未在線。"
```

## Edge Case Handling

### Creation-time validation

| Case | Handling |
|------|----------|
| Target instance doesn't exist | Reject with error listing valid instance names |
| Invalid cron expression | Reject with parse error |
| NULL target | Resolve to source instance name before storing |
| Schedule count exceeded | Reject with limit error (default 100, configurable) |

### Topology changes

| Case | Handling |
|------|----------|
| Target instance removed from fleet.yaml | Trigger-time detection → notify via reply_thread_id |
| Target instance renamed | Becomes broken ref, shown in `ccd schedule list` |
| Source instance removed | No impact — reply coordinates are stored |
| Topic unbound | IPC still works, reply uses stored thread_id |
| Topic deleted from Telegram | Delete all schedules with matching target or reply_thread_id, notify in General topic |
| Fleet restart | Reload all schedules from SQLite on init |
| Missed triggers during downtime | Skip — do not backfill |

### Trigger-time failures

| Case | Handling |
|------|----------|
| Instance offline | Retry 3x at 30s → notify on failure |
| Instance in context rotation | Retry covers the ~10-30s rotation window |
| Multiple simultaneous triggers for same instance | Messages queue naturally |
| DST transitions | Handled by croner's timezone support |

### Execution & reply

| Case | Handling |
|------|----------|
| Telegram API reply failure (429) | Existing MessageQueue exponential backoff |
| Context near full on schedule arrival | Existing handover mechanism; `[排程任務]` prefix aids handover saliency |
| Reply thread deleted | Fallback to chat_id without thread_id (General topic) |
| Next trigger before previous completes | v1: queue. v2: optional skip_if_busy flag |

### Topic deletion cleanup

Detected via the existing `startTopicCleanupPoller` in Fleet Manager (60s polling interval that checks for deleted topics). Telegram Bot API does not emit a `forum_topic_deleted` event — the poller is the authoritative mechanism.

When the poller detects a deleted topic:
1. Resolve thread_id → instance_name via routing table.
2. Call `scheduler.deleteByInstanceOrThread(instanceName, threadId)`.
3. Send summary notification to General topic.

```sql
DELETE FROM schedules WHERE target = ? OR reply_thread_id = ?
-- First param: instance_name (matches target column)
-- Second param: thread_id as string (matches reply_thread_id column)
```

The method signature is `deleteByInstanceOrThread(instanceName: string, threadId: string): number` to clarify that `target` is matched by instance name, not thread ID.

### Cross-instance scheduling

When instance A creates a schedule targeting instance B:
- Execution happens on B (message pushed to B via IPC)
- B's Claude replies to B's own topic (default routing)
- Scheduler sends a summary notification to A's topic (reply_chat_id/reply_thread_id): "⏰ 排程「{label}」已觸發，結果請見 Topic B"

## Dependencies

| Dependency | Purpose | Notes |
|------------|---------|-------|
| croner | Cron parsing & scheduling | Pure JS, timezone-aware, DST-safe, ~15KB, zero deps |
| better-sqlite3 | Already in project | Shared with existing db.ts |
| nanoid or crypto.randomUUID | Schedule IDs | Already available |

No new heavy dependencies.

## Configuration

```yaml
# fleet.yaml
defaults:
  scheduler:
    max_schedules: 100           # max total schedules
    default_timezone: Asia/Taipei
    retry_count: 3               # retries on trigger failure
    retry_interval_ms: 30000     # 30s between retries
```

## Future Considerations (v2, not in scope)

- `auto_wake`: auto-start instance on trigger if offline
- `skip_if_busy`: skip trigger if target is processing a scheduled task
- `priority` / `authority`: hierarchy for cross-instance scheduling
- Stagger option for multiple simultaneous triggers to same instance
- Integration with sandbox feature (pending from another team)
