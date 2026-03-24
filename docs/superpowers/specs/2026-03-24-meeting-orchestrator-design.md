# Meeting Orchestrator — Multi-Instance Collaboration

## Overview

Two commands — `/meets` for structured debate and `/collab` for collaborative coding — that spawn ephemeral Claude Code instances into a shared Telegram topic. FleetManager acts as the system-level moderator, and a `MeetingOrchestrator` class manages the session flow.

## Modes

### Debate Mode (default)

Multiple instances argue different sides of a topic. Roles auto-assigned by participant count:

| Count | Roles |
|-------|-------|
| 2 | Pro, Con |
| 3 | Pro, Con, Arbiter |
| 4+ | Pro×N, Con×N, Arbiter (odd count gets extra arbiter) |

Working directory: `/tmp` (no codebase needed).

Debate instances run in **lightweight mode**: Daemon skips transcript monitor, context guardian, memory layer, and approval server (since `skipPermissions` is true). Only IPC server and tmux are started.

### Collaboration Mode (`--collab`)

Multiple instances work together on a task in a shared repo. Each instance gets an isolated git worktree. Role assignment happens through instance self-discussion or user direction.

Working directory: per-instance git worktree branching from the target repo. Worktree setup/teardown is handled by `FleetManagerMeetingAPI`, not the orchestrator.

## Architecture

```
User: /meets "topic"
       │
       ▼
┌──────────────────────────────────────────────────┐
│              FleetManager (existing)              │
│                                                   │
│  • Parse /meets command or interactive wizard     │
│  • Create meeting channel (Telegram topic, etc.)  │
│  • spawnEphemeralInstance() × N                   │
│  • Instantiate MeetingOrchestrator, hand off      │
│  • Unified routing: topicId → instance | meeting  │
└──────┬───────────────────────────────────────────┘
       │
       ▼
┌──────────────────────────────────────────────────┐
│         MeetingOrchestrator (new)                 │
│                                                   │
│  • Debate/collab flow control                    │
│  • Turn ordering, prompt composition             │
│  • User intervention handling (absolute priority)│
│  • Dynamic participant management (kick/add)     │
│  • Summary generation on completion              │
│  • Request FM to destroy instances on end        │
└──────┬───────────────────────────────────────────┘
       │  via FleetManagerMeetingAPI
       ▼
┌──────────┐ ┌──────────┐ ┌──────────┐
│ Daemon A │ │ Daemon B │ │ Daemon C │
│ ephemeral│ │ ephemeral│ │ ephemeral│
└──────────┘ └──────────┘ └──────────┘
     Claude Code instances (tmux-backed, via CliBackend)
     Debate: lightweight mode | Collab: full mode
```

## Command Interface

### Interactive Wizard (primary, mobile-friendly)

```
User: /meets

Bot: 📋 建立新會議
     議題是什麼？（請直接輸入）

User: 要不要拆 monorepo

Bot: 模式？
     [💬 辯論]  [🔨 協作]

User: (taps 💬 辯論)

Bot: 幾位參與者？
     [2]  [3]  [4]

User: (taps 3)

Bot: ✅ 會議建立中...

     📋 會議：要不要拆 monorepo？
     參與者：A（正方）、B（反方）、C（仲裁）
     輪次：3 | 指令：/end /more /pause
```

Collaboration mode adds a repo selection step using inline buttons populated from existing fleet instance working directories.

### CLI Shorthand (power users)

```
/meets "topic"                                → debate, 2 participants
/meets -n 3 "topic"                           → debate, 3 participants
/collab --repo ~/app "task"                   → collab mode
/collab -n 3 --repo ~/app "task"              → collab, 3 participants
```

## Instance Naming

- Default: A, B, C, D... (short, mode-agnostic, easy to type)
- Custom: `--names "name1,name2"` overrides default labels
- Users reference instances with `@A`, `@B`, etc. in the topic

## Debate Flow

```
Start meeting
  │
  ▼
[Round 1]
  → Send prompt to Pro (A)
  → A replies → post to topic (labeled "🟢 A（正方）")
  → Send "A's argument + please rebut" to Con (B)
  → B replies → post to topic (labeled "🔴 B（反方）")
  → (if arbiter) Send both sides to Arbiter (C)
  → C replies → post to topic (labeled "⚖️ C（仲裁）")
  │
  ▼
[Round 2..N]
  Repeat. Each round carries previous round summary (not full history)
  to control token consumption.
  │
  ▼
[End] Triggered by:
  • Reaching round limit (default: 3)
  • User sends /end
  │
  ▼
[Summary] Arbiter generates summary (if present); otherwise last speaker does.
         Summary prompt includes all round summaries. → post to topic
  │
  ▼
[Cleanup] Notify FleetManager to destroy all ephemeral instances
```

### Prompt Strategy

- Each round sends "previous round summary + opponent's latest argument" — not full conversation history
- Role assigned at instance spawn time via backend-specific mechanism (abstracted by `FleetManagerMeetingAPI`). Per-round context (opponent's arguments, user instructions) is sent as regular user messages via IPC.
- User free-text in topic is injected as additional context to the next speaker

## Collaboration Flow

```
/collab --repo ~/app -n 3 "Implement OAuth login"
  │
  ▼
FleetManagerMeetingAPI.spawnEphemeralInstance() handles:
  1. git worktree add /tmp/meet-{id}-A -b meet/{id}-A
  2. git worktree add /tmp/meet-{id}-B -b meet/{id}-B
  3. git worktree add /tmp/meet-{id}-C -b meet/{id}-C
  4. Start Daemon in each worktree (full mode)
  │
  ▼
Discussion phase: instances discuss task division in topic
  │
  ▼
Development phase: each works in own worktree
  │
  ▼
End: FleetManagerMeetingAPI.destroyEphemeralInstance() handles:
  - Stop Daemon
  - git worktree remove --force
  - git branch -D meet/{id}-*
  - Report merge status to orchestrator
```

## User Control (Absolute Priority)

User messages in the meeting topic always take highest priority. The orchestrator pauses its current flow to handle user input.

### Command Table

| Input | Behavior |
|-------|----------|
| `/end` | Immediately end, enter summary phase |
| `/more` | +1 round |
| `/more 3` | +3 rounds |
| `/pause` | Pause flow, wait for `/resume` |
| `/resume` | Resume paused flow |
| `/kick A` | Remove instance A — orchestrator calls `fm.destroyEphemeralInstance()`, updates participant list |
| `/add` | Spawn additional instance — orchestrator calls `fm.spawnEphemeralInstance()`, assigns next label |
| `/redirect A "argue from cost perspective"` | Direct instruction to specific instance |
| `@A what about testing?` | Override turn order, A speaks next with this prompt |
| Free text | Appended as additional context to the next speaker's prompt |

### Principles

1. **User message = pause flow** — regardless of what orchestrator is waiting for
2. **User can change rules anytime** — roles, topic, participant count
3. **User can direct-address** — `@A` bypasses orchestrator scheduling, response posted to topic

## Channel Abstraction

MeetingOrchestrator does not assume Telegram. It outputs structured message objects through a `MeetingChannelOutput` — a thin wrapper that binds `chatId`/`threadId` at construction and delegates to the existing `ChannelAdapter`:

```typescript
interface MeetingChannelOutput {
  postMessage(text: string, options?: { label?: string }): Promise<string>
  editMessage(messageId: string, text: string): Promise<void>
}
```

Channel creation/closure is handled by `FleetManagerMeetingAPI`, not the orchestrator.

Orchestrator emits structured data:

```typescript
{ speaker: "A", role: "pro", round: 1, content: "..." }
```

Channel adapter decides rendering. Telegram renders as:

```
🟢 A（正方）：
Monorepo 的部署耦合...
```

Future adapters (Slack, Discord) render in their own native formats.

## MeetingOrchestrator Interface

```typescript
type MeetingRole = "pro" | "con" | "arbiter" | (string & {})  // extensible union

interface MeetingConfig {
  meetingId: string
  topic: string
  mode: "debate" | "collab"
  maxRounds: number          // default: 3
  repo?: string              // collab mode only
}

interface ParticipantConfig {
  label: string              // "A", "B", or custom name
  role: MeetingRole
}

class MeetingOrchestrator {
  constructor(
    config: MeetingConfig,
    fm: FleetManagerMeetingAPI,
    output: MeetingChannelOutput
  )

  /** Boot instances and start the debate/collab flow */
  async start(participants: ParticipantConfig[]): Promise<void>

  /** Handle any user message in the meeting topic (absolute priority) */
  handleUserMessage(msg: InboundMessage): void

  /** Add a participant mid-meeting (/add) */
  async addParticipant(config: ParticipantConfig): Promise<void>

  /** Remove a participant mid-meeting (/kick) */
  async removeParticipant(label: string): Promise<void>

  /** End meeting: summary → cleanup → destroy instances */
  async end(): Promise<void>
}
```

## FleetManager Extensions

### New Methods

```typescript
interface FleetManagerMeetingAPI {
  /** Spawn a temporary instance. Handles worktree setup for collab mode. */
  spawnEphemeralInstance(config: EphemeralInstanceConfig, signal?: AbortSignal): Promise<string>

  /** Destroy a temporary instance. Handles worktree cleanup for collab mode. */
  destroyEphemeralInstance(name: string): Promise<void>

  /** Send message to instance and wait for its reply (see Reply Capture below). */
  sendAndWaitReply(instanceName: string, message: string, timeoutMs?: number): Promise<string>

  /** Create a meeting channel (e.g., Telegram forum topic). */
  createMeetingChannel(title: string): Promise<{ channelId: number }>

  /** Close a meeting channel. */
  closeMeetingChannel(channelId: number): Promise<void>
}
```

Internally, `spawnEphemeralInstance` delegates to existing `startInstance()` and `destroyEphemeralInstance` delegates to existing `stopInstance()`.

### EphemeralInstanceConfig

```typescript
interface EphemeralInstanceConfig {
  systemPrompt: string        // role instructions, delivered via backend-specific mechanism
  workingDirectory: string    // debate: /tmp, collab: worktree path
  lightweight?: boolean       // debate: true (skip context guardian, memory layer, etc.)
  skipPermissions?: boolean   // debate: true
  backend?: string            // defaults to fleet config defaults.backend or "claude-code"
}
```

### `sendAndWaitReply` — Reply Capture Mechanism

This is the most critical new primitive. Today, Claude responds asynchronously via the `reply` tool call, which routes through `fleet_outbound` to Telegram. For meetings, this flow is intercepted:

1. FleetManager sends `fleet_inbound` to the ephemeral instance (same as normal messages)
2. Claude processes the prompt, potentially uses tools, then calls `reply`
3. Daemon emits `fleet_outbound` with `tool=reply`
4. FleetManager checks: is this instance part of an active meeting?
   - **Yes** → resolve the pending `sendAndWaitReply` promise with the reply text. Do NOT post to Telegram.
   - **No** → existing behavior (post to Telegram)
5. Timeout: 120s default, configurable. On timeout, return a timeout error to the orchestrator.

Multiple `reply` calls: concatenate until a 5-second idle period, then resolve. Reply buffer capped at 32KB to prevent unbounded growth.

```
FleetManager.sendAndWaitReply("meet-xyz-A", prompt)
  │
  ├─ send fleet_inbound to Daemon A
  ├─ register pendingMeetingReply["meet-xyz-A"] = { resolve, reject }
  │
  ▼ (async, Claude processes...)

Daemon A fleet_outbound { tool: "reply", args: { text: "..." } }
  │
  ▼
FleetManager.handleOutboundFromInstance("meet-xyz-A", msg)
  ├─ Check: pendingMeetingReply has "meet-xyz-A"?
  ├─ YES → resolve promise with text, don't post to Telegram
  └─ The orchestrator receives the text and posts to topic itself (with formatting)
```

On `MeetingOrchestrator.end()`, all pending `sendAndWaitReply` promises are rejected with `AbortError` to prevent dangling references.

### Approval Strategy for Ephemeral Instances

- **Debate mode**: instances use `skipPermissions: true` (working in /tmp, no risk)
- **Collab mode**: instances use the same approval strategy as normal fleet instances, with approval prompts routed to the meeting topic

### Unified Routing

The existing `routingTable` is extended with a discriminated union instead of a separate map:

```typescript
type RouteTarget =
  | { kind: "instance"; name: string }
  | { kind: "meeting"; orchestrator: MeetingOrchestrator }

routingTable: Map<number, RouteTarget>

handleInboundMessage(msg) {
  const threadId = parseInt(msg.threadId, 10)
  const target = routingTable.get(threadId)
  if (!target) return
  if (target.kind === "meeting") {
    target.orchestrator.handleUserMessage(msg)
  } else {
    // existing instance routing logic
  }
}
```

## Resource Limits

- Maximum 1 active meeting at a time (configurable via fleet.yaml `meetings.maxConcurrent`)
- Maximum 6 participants per meeting (configurable via `meetings.maxParticipants`)
- Attempting to create a meeting while one is active returns an error to the user
- Instances are spawned in parallel with `Promise.all` + `AbortSignal` for cancellation

## Error Handling

- Instance fails to start → post error to topic, continue with remaining instances
- Instance stops responding (timeout 120s) → post timeout notice, skip turn, continue
- All instances fail → end meeting with error summary
- User sends /end during instance boot → AbortSignal cancels remaining spawns, cleanup started instances
- Collab mode: validate `--repo` path is a git repository before spawning

## Backend Prerequisites

The following extensions to existing interfaces are required before meeting functionality can work:

1. **`CliBackendConfig` needs `systemPrompt?: string`** — `buildCommand()` appends the appropriate flag. Supported by Claude Code CLI.
2. **`CliBackendConfig` needs `skipPermissions?: boolean`** — `buildCommand()` appends `--dangerously-skip-permissions` when true.
3. **`Daemon` needs `lightweight?: boolean` config** — when true, skips transcript monitor, context guardian, memory layer, and approval server during startup.
4. **`TelegramAdapter` needs `closeForumTopic(threadId)`** — wraps the Telegram Bot API `closeForumTopic` method.

## Scope Boundaries (not in v1)

- Writing meeting conclusions back to instance memory (future enhancement)
- Meeting templates / presets
- Persistent meeting history (meetings are ephemeral)
- Cross-meeting instance reuse
