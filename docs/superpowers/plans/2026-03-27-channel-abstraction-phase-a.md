# Channel Abstraction Phase A — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove all direct TelegramAdapter coupling from business logic. After this refactor, fleet-manager.ts, daemon.ts, and topic-commands.ts have zero imports from `channel/adapters/telegram.ts`.

**Architecture:** Add intent-oriented methods (`promptUser`, `notifyAlert`, `reportStatus`) and topology/chatId management to ChannelAdapter interface. Implement them in TelegramAdapter. Create an adapter factory. Refactor all callers to use the interface only. Pure refactor — zero behavior change.

**Tech Stack:** TypeScript, Grammy (stays inside TelegramAdapter only)

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Modify | `src/channel/types.ts` | Add `topology`, `Choice`, `AlertData`, `InstanceStatusData`, intent methods, `chatId` management, `createTopic`/`topicExists` |
| Modify | `src/channel/adapters/telegram.ts` | Implement new interface methods, add `topology` property |
| Create | `src/channel/factory.ts` | `createAdapter(config, opts)` factory function |
| Modify | `src/types.ts` | Change `ChannelConfig.type` from `"telegram"` to `string` |
| Modify | `src/fleet-manager.ts` | Replace all TelegramAdapter casts with interface calls, use factory |
| Modify | `src/daemon.ts` | Use factory instead of direct TelegramAdapter instantiation |
| Modify | `src/topic-commands.ts` | Replace `sendTextWithKeyboard` with `promptUser` |
| Modify | `src/fleet-context.ts` | No changes expected (already uses ChannelAdapter interface) |

---

### Task 1: Extend ChannelAdapter Interface

**Files:**
- Modify: `src/channel/types.ts`
- Modify: `src/types.ts`

- [ ] **Step 1: Add new types and methods to the ChannelAdapter interface**

In `src/channel/types.ts`, add these types before `ChannelAdapter`:

```typescript
export interface Choice {
  id: string;
  label: string;
}

export interface InstanceStatusData {
  name: string;
  status: "running" | "stopped" | "crashed" | "paused";
  contextPct: number | null;
  costCents: number;
}

export interface AlertData {
  type: "hang" | "cost_warn" | "cost_limit" | "schedule_deferred" | "rotation";
  instanceName: string;
  message: string;
  choices?: Choice[];
}
```

Add these to the `ChannelAdapter` interface:

```typescript
readonly topology: "topics" | "channels" | "flat";

// Chat ID management (adapter tracks internally)
setChatId(chatId: string): void;
getChatId(): string | null;

// Intent-oriented high-level methods
promptUser(chatId: string, text: string, choices: Choice[], opts?: SendOpts): Promise<string>;
notifyAlert(chatId: string, alert: AlertData, opts?: SendOpts): Promise<SentMessage>;

// Topology-dependent (optional)
createTopic?(name: string): Promise<number>;
topicExists?(topicId: number): Promise<boolean>;
```

- [ ] **Step 2: Change ChannelConfig.type from literal to string**

In `src/types.ts`, change:

```typescript
// FROM:
type: "telegram";
// TO:
type: string;
```

- [ ] **Step 3: Run TypeScript to see all the type errors (expected)**

Run: `npx tsc --noEmit 2>&1 | head -20`
Expected: Type errors in TelegramAdapter (not implementing new methods). This confirms the interface change propagated.

- [ ] **Step 4: Commit interface changes**

```bash
git add src/channel/types.ts src/types.ts
git commit -m "refactor: extend ChannelAdapter with topology, intent methods, chatId management"
```

---

### Task 2: Implement New Methods in TelegramAdapter

**Files:**
- Modify: `src/channel/adapters/telegram.ts`

- [ ] **Step 1: Add `topology` property**

```typescript
readonly topology = "topics" as const;
```

- [ ] **Step 2: Add `setChatId` / `getChatId`**

These already exist as `setLastChatId`/`getLastChatId` — rename them:

```typescript
setChatId(chatId: string): void { this.lastChatId = chatId; }
getChatId(): string | null { return this.lastChatId; }
```

Keep the old names as aliases temporarily if needed, but the goal is to only use the new names.

- [ ] **Step 3: Implement `promptUser`**

Convert the existing `sendTextWithKeyboard` to implement `promptUser`. The existing method takes a grammy `InlineKeyboard` — the new method takes `Choice[]` and builds the keyboard internally:

```typescript
async promptUser(chatId: string, text: string, choices: Choice[], opts?: SendOpts): Promise<string> {
  const keyboard = new InlineKeyboard();
  for (const choice of choices) {
    keyboard.text(choice.label, choice.id).row();
  }
  const threadId = opts?.threadId;
  const msg = await this.bot.api.sendMessage(Number(chatId), text, {
    message_thread_id: threadId != null ? Number(threadId) : undefined,
    reply_markup: keyboard,
  });
  return String(msg.message_id);
}
```

Note: `promptUser` returns a message ID string. The callback query routing remains in the fleet manager — the adapter just sends the UI.

- [ ] **Step 4: Implement `notifyAlert`**

```typescript
async notifyAlert(chatId: string, alert: AlertData, opts?: SendOpts): Promise<SentMessage> {
  const threadId = opts?.threadId;
  if (alert.choices && alert.choices.length > 0) {
    const keyboard = new InlineKeyboard();
    for (const choice of alert.choices) {
      keyboard.text(choice.label, choice.id);
    }
    const msg = await this.bot.api.sendMessage(Number(chatId), alert.message, {
      message_thread_id: threadId != null ? Number(threadId) : undefined,
      reply_markup: keyboard,
    });
    return { messageId: String(msg.message_id), chatId, threadId };
  }
  return this.sendText(chatId, alert.message, opts);
}
```

- [ ] **Step 5: Implement `createTopic` and `topicExists`**

```typescript
async createTopic(name: string): Promise<number> {
  const chatId = this.getChatId();
  if (!chatId) throw new Error("No chat ID set — cannot create topic");
  const res = await this.bot.api.createForumTopic(Number(chatId), name);
  return res.message_thread_id;
}

async topicExists(topicId: number): Promise<boolean> {
  const chatId = this.getChatId();
  if (!chatId) return false;
  try {
    const msg = await this.bot.api.sendMessage(Number(chatId), "\u200B", {
      message_thread_id: topicId,
    });
    await this.bot.api.deleteMessage(Number(chatId), msg.message_id).catch(() => {});
    return true;
  } catch (err: unknown) {
    const errMsg = String(err);
    if (errMsg.includes("thread not found") || errMsg.includes("TOPIC_ID_INVALID")) {
      return false;
    }
    throw err;
  }
}
```

- [ ] **Step 6: Verify TypeScript compiles**

Run: `npx tsc --noEmit`
Expected: Should pass (or only errors in callers that still use old methods — those get fixed in later tasks)

- [ ] **Step 7: Run all tests**

Run: `npx vitest run`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add src/channel/adapters/telegram.ts
git commit -m "feat: implement intent-oriented methods in TelegramAdapter"
```

---

### Task 3: Adapter Factory

**Files:**
- Create: `src/channel/factory.ts`

- [ ] **Step 1: Create the factory**

```typescript
// src/channel/factory.ts
import type { ChannelAdapter } from "./types.js";
import type { ChannelConfig } from "../types.js";
import type { AccessManager } from "./access-manager.js";

export interface AdapterOpts {
  id: string;
  botToken: string;
  accessManager: AccessManager;
  inboxDir: string;
}

export async function createAdapter(config: ChannelConfig, opts: AdapterOpts): Promise<ChannelAdapter> {
  switch (config.type) {
    case "telegram": {
      const { TelegramAdapter } = await import("./adapters/telegram.js");
      return new TelegramAdapter(opts);
    }
    default:
      throw new Error(`Unknown channel type: ${config.type}`);
  }
}
```

Uses dynamic import so that adapter-specific dependencies (grammy) are only loaded when needed.

- [ ] **Step 2: Commit**

```bash
git add src/channel/factory.ts
git commit -m "feat: add channel adapter factory"
```

---

### Task 4: Refactor FleetManager — Remove TelegramAdapter Coupling

This is the largest task. There are 4 cast sites to fix.

**Files:**
- Modify: `src/fleet-manager.ts`

- [ ] **Step 1: Replace TelegramAdapter import with factory import**

Remove:
```typescript
import { TelegramAdapter } from "./channel/adapters/telegram.js";
```

Add:
```typescript
import { createAdapter } from "./channel/factory.js";
import type { Choice, AlertData } from "./channel/types.js";
```

- [ ] **Step 2: Replace `new TelegramAdapter(...)` in `startSharedAdapter`**

Change the direct instantiation to use the factory:

```typescript
// FROM:
this.adapter = new TelegramAdapter({
  id: "tg-fleet",
  botToken,
  accessManager,
  inboxDir,
});

// TO:
this.adapter = await createAdapter(channelConfig, {
  id: "fleet",
  botToken,
  accessManager,
  inboxDir,
});
```

- [ ] **Step 3: Replace `setLastChatId` cast**

```typescript
// FROM:
(this.adapter as TelegramAdapter).setLastChatId(String(fleet.channel.group_id));

// TO:
this.adapter.setChatId(String(fleet.channel.group_id));
```

- [ ] **Step 4: Replace `getLastChatId` cast in `handleToolStatusFromInstance`**

```typescript
// FROM:
const chatId = (this.adapter as TelegramAdapter).getLastChatId();

// TO:
const chatId = this.adapter.getChatId();
```

- [ ] **Step 5: Replace topic cleanup poller (`getBot()` usage)**

Replace the `startTopicCleanupPoller` method. Instead of calling `getBot()` to do raw API probing, use the `topicExists` interface method:

```typescript
private startTopicCleanupPoller(): void {
  this.topicCleanupTimer = setInterval(async () => {
    if (!this.fleetConfig?.channel?.group_id || !this.adapter?.topicExists) return;

    for (const [threadId, target] of this.routingTable) {
      try {
        const exists = await this.adapter.topicExists(threadId);
        if (!exists) {
          const targetName = target.kind === "instance" ? target.name : "meeting";
          this.logger.info({ threadId, target: targetName }, "Topic deleted — auto-unbinding");
          await this.topicCommands.handleTopicDeleted(threadId);
        }
      } catch (err) {
        this.logger.debug({ err, threadId }, "Topic existence check failed");
      }
    }
  }, 5 * 60_000);
}
```

- [ ] **Step 6: Replace `sendHangNotification` (sendTextWithKeyboard → notifyAlert)**

```typescript
private async sendHangNotification(instanceName: string): Promise<void> {
  if (!this.adapter) return;
  const groupId = this.fleetConfig?.channel?.group_id;
  if (!groupId) return;
  const threadId = this.fleetConfig?.instances[instanceName]?.topic_id;

  await this.adapter.notifyAlert(String(groupId), {
    type: "hang",
    instanceName,
    message: `⚠️ ${instanceName} appears hung (no activity for 15+ minutes)`,
    choices: [
      { id: `hang:restart:${instanceName}`, label: "🔄 Force restart" },
      { id: `hang:wait:${instanceName}`, label: "⏳ Keep waiting" },
    ],
  }, {
    threadId: threadId != null ? String(threadId) : undefined,
  }).catch(e => this.logger.debug({ err: e }, "Failed to send hang notification"));
}
```

- [ ] **Step 7: Replace `createForumTopic` method**

The `FleetManager.createForumTopic` method currently makes raw API calls. Replace it to delegate to the adapter:

```typescript
async createForumTopic(topicName: string): Promise<number> {
  if (!this.adapter?.createTopic) {
    throw new Error("Adapter does not support topic creation");
  }
  return this.adapter.createTopic(topicName);
}
```

- [ ] **Step 8: Verify no TelegramAdapter imports remain**

Run: `grep -n "TelegramAdapter" src/fleet-manager.ts`
Expected: Zero matches

- [ ] **Step 9: Run all tests**

Run: `npx vitest run`
Expected: All tests pass

- [ ] **Step 10: Commit**

```bash
git add src/fleet-manager.ts
git commit -m "refactor: remove all TelegramAdapter coupling from fleet-manager"
```

---

### Task 5: Refactor TopicCommands — Remove TelegramAdapter Coupling

**Files:**
- Modify: `src/topic-commands.ts`

- [ ] **Step 1: Replace `sendTextWithKeyboard` with `promptUser`**

In `sendOpenKeyboard`, replace the keyboard building + `sendTextWithKeyboard` call with `promptUser`:

```typescript
// Build choices from directories
const choices: Choice[] = [];
for (let i = 0; i < pageDirs.length; i++) {
  const idx = pageStart + i;
  choices.push({ id: `cmd_open:${sessionId}:${idx}`, label: `📁 ${basename(pageDirs[i])}` });
}

if (page > 0) choices.push({ id: `cmd_open:${sessionId}:page:${page - 1}`, label: "⬅️ Prev" });
if (hasMore) choices.push({ id: `cmd_open:${sessionId}:page:${page + 1}`, label: "➡️ Next" });
choices.push({ id: `cmd_open:${sessionId}:cancel`, label: "❌ Cancel" });

const headerText = page === 0 ? "📂 Select a project:" : `📂 Projects (page ${page + 1}):`;
await this.ctx.adapter!.promptUser(chatId, headerText, choices);
```

- [ ] **Step 2: Remove TelegramAdapter import**

Remove:
```typescript
import { TelegramAdapter } from "./channel/adapters/telegram.js";
```

Add (if not already):
```typescript
import type { Choice } from "./channel/types.js";
```

- [ ] **Step 3: Verify no TelegramAdapter references remain**

Run: `grep -n "TelegramAdapter" src/topic-commands.ts`
Expected: Zero matches

- [ ] **Step 4: Run all tests**

Run: `npx vitest run`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/topic-commands.ts
git commit -m "refactor: remove TelegramAdapter coupling from topic-commands"
```

---

### Task 6: Refactor Daemon — Use Factory

**Files:**
- Modify: `src/daemon.ts`

- [ ] **Step 1: Replace direct TelegramAdapter import with factory**

Remove:
```typescript
import { TelegramAdapter } from "./channel/adapters/telegram.js";
```

Add:
```typescript
import { createAdapter } from "./channel/factory.js";
```

- [ ] **Step 2: Replace `new TelegramAdapter(...)` with `createAdapter`**

In the DM mode adapter creation block:

```typescript
// FROM:
this.adapter = new TelegramAdapter({
  id: `tg-${this.name}`,
  botToken,
  accessManager,
  inboxDir,
});

// TO:
this.adapter = await createAdapter(this.config.channel!, {
  id: `dm-${this.name}`,
  botToken,
  accessManager,
  inboxDir,
});
```

Note: `createAdapter` is async (dynamic import), but `start()` is already async so this is fine.

- [ ] **Step 3: Verify no TelegramAdapter references remain**

Run: `grep -n "TelegramAdapter" src/daemon.ts`
Expected: Zero matches

- [ ] **Step 4: Run all tests**

Run: `npx vitest run`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/daemon.ts
git commit -m "refactor: use adapter factory in daemon instead of direct TelegramAdapter"
```

---

### Task 7: Final Verification

- [ ] **Step 1: Verify zero TelegramAdapter imports in business logic**

Run: `grep -rn "from.*telegram" src/*.ts src/channel/*.ts | grep -v "channel/adapters/" | grep -v "channel/factory."`
Expected: Zero matches (only adapters/ and factory should reference telegram)

- [ ] **Step 2: Run full test suite**

Run: `npx vitest run`
Expected: All tests pass

- [ ] **Step 3: TypeScript strict check**

Run: `npx tsc --noEmit`
Expected: No errors

- [ ] **Step 4: Build**

Run: `npm run build`
Expected: Succeeds

- [ ] **Step 5: Commit (if any final fixups needed)**

---

## Build Order

```
Task 1 (Interface) ──── must be first
Task 2 (TelegramAdapter) ──── depends on Task 1
Task 3 (Factory) ──── depends on Task 2
Task 4 (FleetManager refactor) ──── depends on Tasks 2+3
Task 5 (TopicCommands refactor) ──── depends on Task 2
Task 6 (Daemon refactor) ──── depends on Task 3
Task 7 (Verification) ──── after all
```

Tasks 4, 5, 6 can be done in any order after Task 3. Recommended: 1 → 2 → 3 → 4 → 5 → 6 → 7 (sequential, as each builds on the last).
