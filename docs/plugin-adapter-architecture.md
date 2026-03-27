# Plugin Adapter Architecture

Design specification for external channel adapters in CCD.

## Status

Draft — 2026-03-27

## Overview

CCD supports Telegram as a built-in adapter. This document specifies how third-party adapters (Discord, LINE, Slack, etc.) can be developed as independent npm packages and loaded at runtime.

**Key decisions:**
- Telegram stays built-in (zero-config for the common case)
- Community adapters are separate npm packages (e.g. `ccd-adapter-discord`)
- Factory loads external packages via dynamic `import()`
- No SDK package needed — adapters depend only on the `ChannelAdapter` interface exported from `claude-channel-daemon`

## 1. Plugin Adapter Loading

### fleet.yaml configuration

```yaml
channel:
  type: discord                    # triggers external adapter lookup
  mode: topic                      # "topic" or "dm"
  bot_token_env: CCD_DISCORD_TOKEN
  group_id: "1234567890"           # Discord guild ID (string)
  access:
    mode: locked
    allowed_users: ["987654321"]
  options:                         # adapter-specific config
    category_name: "CCD Agents"
    general_channel_id: "111222333"
```

### Resolution order

When `type` is not `"telegram"`, the factory resolves the adapter in this order:

1. `ccd-adapter-{type}` — canonical package name (e.g. `ccd-adapter-discord`)
2. `{type}` — bare package name (fallback for non-standard naming)

```typescript
// factory.ts — updated
export async function createAdapter(config: ChannelConfig, opts: AdapterOpts): Promise<ChannelAdapter> {
  switch (config.type) {
    case "telegram": {
      const { TelegramAdapter } = await import("./adapters/telegram.js");
      return new TelegramAdapter(opts);
    }
    default: {
      // External adapter — try canonical name, then bare name
      const candidates = [`ccd-adapter-${config.type}`, config.type];
      let mod: { default: AdapterFactory } | undefined;

      for (const pkg of candidates) {
        try {
          mod = await import(pkg);
          break;
        } catch {
          continue;
        }
      }

      if (!mod?.default) {
        throw new Error(
          `Channel adapter "${config.type}" not found. ` +
          `Install it: npm install ccd-adapter-${config.type}`
        );
      }

      return mod.default(config, opts);
    }
  }
}
```

### Security considerations

- `import()` of user-specified package names is inherently trusted — the user controls `fleet.yaml` and has shell access. This is equivalent trust level to `npm install`.
- External adapters run in the same process. A malicious adapter has full access. This matches the trust model of npm dependencies.
- No sandboxing or capability restriction is provided or needed (same trust level as MCP servers configured in `.mcp.json`).

### Version compatibility

External adapters should declare a peer dependency on `claude-channel-daemon`:

```json
{
  "peerDependencies": {
    "claude-channel-daemon": ">=0.3.0"
  }
}
```

The factory does not enforce version checks at runtime. npm's peer dependency resolution handles this at install time.

## 2. Adapter Contract

### What an adapter must export

A **default export** that is an `AdapterFactory` function:

```typescript
import type { ChannelAdapter, ChannelConfig, AdapterOpts } from "claude-channel-daemon";

type AdapterFactory = (config: ChannelConfig, opts: AdapterOpts) => ChannelAdapter;

// adapter's index.ts
const createDiscordAdapter: AdapterFactory = (config, opts) => {
  return new DiscordAdapter(config, opts);
};
export default createDiscordAdapter;
```

### Interface reference

The adapter must implement the full `ChannelAdapter` interface from `claude-channel-daemon/channel/types`:

```typescript
interface ChannelAdapter extends EventEmitter {
  readonly type: string;          // e.g. "discord"
  readonly id: string;            // e.g. "discord-fleet"
  readonly topology: "topics" | "channels" | "flat";

  // Lifecycle
  start(): Promise<void>;
  stop(): Promise<void>;

  // Core messaging
  sendText(chatId: string, text: string, opts?: SendOpts): Promise<SentMessage>;
  sendFile(chatId: string, filePath: string, opts?: SendOpts): Promise<SentMessage>;
  editMessage(chatId: string, messageId: string, text: string): Promise<void>;
  react(chatId: string, messageId: string, emoji: string): Promise<void>;

  // Approval flow
  sendApproval(prompt, callback, signal?, threadId?): Promise<ApprovalHandle>;

  // File handling
  downloadAttachment(fileId: string): Promise<string>;

  // Access control
  handlePairing(chatId: string, userId: string): Promise<string>;
  confirmPairing(code: string): Promise<boolean>;

  // Chat ID management
  setChatId(chatId: string): void;
  getChatId(): string | null;

  // Intent-oriented methods
  promptUser(chatId: string, text: string, choices: Choice[], opts?: SendOpts): Promise<string>;
  notifyAlert(chatId: string, alert: AlertData, opts?: SendOpts): Promise<SentMessage>;

  // Topology-dependent (optional)
  createTopic?(name: string): Promise<number>;
  topicExists?(topicId: number): Promise<boolean>;
}
```

### Events the adapter must emit

| Event | Payload | When |
|-------|---------|------|
| `message` | `InboundMessage` | User sends a message |
| `callback_query` | `{ callbackData, chatId, threadId?, messageId }` | User clicks a button |
| `topic_closed` | `{ chatId, threadId }` | Topic/channel deleted |
| `started` | `username: string` | Bot connected |
| `handler_error` | `Error` | Unhandled error in message processing |

### Lifecycle

1. Constructor receives `ChannelConfig` and `AdapterOpts` (id, botToken, accessManager, inboxDir)
2. `start()` — connect to platform API, begin receiving events
3. Emit `started` when ready
4. Emit `message` for each inbound user message
5. `stop()` — disconnect, clean up resources

### Degradation strategy

Each adapter handles unsupported features internally:

| Feature | If unsupported |
|---------|---------------|
| `react()` | No-op (resolve silently) |
| `editMessage()` | Send new message instead |
| `createTopic()` | Create channel, or throw if flat topology |
| `promptUser()` | Fall back to numbered text list if no buttons |
| Markdown | Strip or convert to plain text |

## 3. Example Package Structure

```
ccd-adapter-discord/
├── package.json
├── tsconfig.json
├── src/
│   ├── index.ts           # default export: AdapterFactory
│   ├── discord-adapter.ts # ChannelAdapter implementation
│   └── utils.ts           # helpers (chunking, emoji mapping, etc.)
├── dist/                  # compiled JS
└── README.md
```

### package.json

```json
{
  "name": "ccd-adapter-discord",
  "version": "0.1.0",
  "type": "module",
  "main": "dist/index.js",
  "types": "dist/index.d.ts",
  "peerDependencies": {
    "claude-channel-daemon": ">=0.3.0"
  },
  "dependencies": {
    "discord.js": "^14.0.0"
  }
}
```

### Minimal index.ts

```typescript
import type { ChannelAdapter } from "claude-channel-daemon/channel/types";
import type { ChannelConfig } from "claude-channel-daemon/types";
import type { AdapterOpts } from "claude-channel-daemon/channel/factory";
import { DiscordAdapter } from "./discord-adapter.js";

export default function createAdapter(config: ChannelConfig, opts: AdapterOpts): ChannelAdapter {
  return new DiscordAdapter(config, opts);
}
```

### Testing

Adapters can be tested independently:

```typescript
import createAdapter from "ccd-adapter-discord";

const adapter = createAdapter(
  { type: "discord", mode: "topic", bot_token_env: "TEST_TOKEN", access: { mode: "locked", allowed_users: [] } },
  { id: "test", botToken: "test-token", accessManager: mockAccessManager, inboxDir: "/tmp/inbox" },
);

await adapter.start();
// ... test message handling ...
await adapter.stop();
```

## 4. Discord Adapter Design

### Design decisions

**Q1: Topology**
Discord uses `topology: "channels"`. Each CCD instance maps to a Discord text channel within a guild category.

**Q2: Mapping**

| CCD concept | Telegram | Discord |
|-------------|----------|---------|
| Group | Telegram Group (group_id) | Guild (guild_id) |
| Topic | Forum Topic (thread_id) | Text Channel (channel_id) |
| General | General Topic (thread 0) | A designated channel |
| Message | Message | Message |
| Button | InlineKeyboard | ActionRow + Button |
| Reaction | setMessageReaction | addReaction |

**Q3: routingTable**
No change needed. Discord channel IDs are snowflake strings, but they can be parsed to numbers for the routing table. Alternatively, the adapter maps channel_id (string) → numeric thread_id internally. This keeps CCD core untouched.

**Q4: MVP scope**

Phase 1 (MVP):
- Connect to Discord gateway
- Receive messages from channels → emit `InboundMessage`
- `sendText()` with chunking at 2000 chars (Discord limit)
- `sendApproval()` with Discord buttons (ActionRow)
- `react()` with Discord emoji
- `editMessage()`
- `createTopic()` → create text channel in category
- `promptUser()` with Discord buttons

Phase 2:
- File upload/download
- Voice transcription integration
- Thread support (Discord threads within channels)

### Discord-specific fleet.yaml options

```yaml
channel:
  type: discord
  mode: topic
  bot_token_env: CCD_DISCORD_TOKEN
  group_id: "GUILD_ID"
  access:
    mode: locked
    allowed_users: ["USER_ID"]
  options:
    category_name: "CCD Agents"      # category to create channels in
    general_channel_id: "CHANNEL_ID"  # channel for /open, /new commands
```

### Implementation sketch

```typescript
import { Client, GatewayIntentBits, ActionRowBuilder, ButtonBuilder, ButtonStyle } from "discord.js";

class DiscordAdapter extends EventEmitter implements ChannelAdapter {
  readonly type = "discord";
  readonly topology = "channels" as const;
  private client: Client;
  private guildId: string;
  private categoryName: string;

  async start() {
    this.client = new Client({
      intents: [
        GatewayIntentBits.Guilds,
        GatewayIntentBits.GuildMessages,
        GatewayIntentBits.MessageContent,
      ],
    });

    this.client.on("messageCreate", (msg) => {
      if (msg.author.bot) return;
      this.emit("message", {
        source: "discord",
        adapterId: this.id,
        chatId: msg.guildId ?? "",
        threadId: msg.channelId,
        messageId: msg.id,
        userId: msg.author.id,
        username: msg.author.username,
        text: msg.content,
        timestamp: msg.createdAt,
      });
    });

    await this.client.login(this.botToken);
    this.emit("started", this.client.user?.username ?? "discord-bot");
  }

  async sendText(chatId: string, text: string, opts?: SendOpts) {
    const channel = await this.client.channels.fetch(opts?.threadId ?? chatId);
    if (!channel?.isTextBased()) throw new Error("Not a text channel");
    // Discord limit: 2000 chars
    const chunks = splitText(text, 2000);
    const first = await channel.send(chunks[0]);
    for (let i = 1; i < chunks.length; i++) {
      await channel.send(chunks[i]);
    }
    return { messageId: first.id, chatId, threadId: opts?.threadId };
  }

  async sendApproval(prompt, callback, signal?, threadId?) {
    const row = new ActionRowBuilder<ButtonBuilder>().addComponents(
      new ButtonBuilder().setCustomId(`approve:${nonce}`).setLabel("Allow").setStyle(ButtonStyle.Success),
      new ButtonBuilder().setCustomId(`deny:${nonce}`).setLabel("Deny").setStyle(ButtonStyle.Danger),
    );
    // ... send message with components, listen for interaction ...
  }

  async createTopic(name: string) {
    const guild = await this.client.guilds.fetch(this.guildId);
    const category = guild.channels.cache.find(c => c.name === this.categoryName);
    const channel = await guild.channels.create({
      name,
      parent: category?.id,
    });
    return parseInt(channel.id); // snowflake → number (may lose precision for >53-bit IDs)
  }
}
```

> **Note on snowflake precision:** Discord snowflake IDs are 64-bit integers. JavaScript `Number` is safe up to 2^53. Current Discord snowflakes are ~18 digits, within safe range until ~2090. If this becomes an issue, `routingTable` should be migrated to `Map<string, RouteTarget>`.

## 5. Types to Export from CCD

For external adapters to `import type` from CCD, the following must be exported from the package's entry point:

```typescript
// src/index.ts (or package.json "exports")
export type {
  ChannelAdapter,
  SendOpts,
  SentMessage,
  InboundMessage,
  Attachment,
  PermissionPrompt,
  ApprovalHandle,
  ApprovalResponse,
  Choice,
  AlertData,
  InstanceStatusData,
  QueuedMessage,
} from "./channel/types.js";

export type { ChannelConfig, AccessConfig } from "./types.js";
export type { AdapterOpts } from "./channel/factory.js";
export type { AdapterFactory } from "./channel/factory.js";
```

## 6. Migration Path

### Phase A (current): factory update
- Update `factory.ts` to try `import(`ccd-adapter-${type}`)` for unknown types
- Export `AdapterFactory` type from factory.ts
- Export channel types from package entry point
- No breaking changes to existing Telegram users

### Phase B: Discord adapter
- Publish `ccd-adapter-discord` to npm
- User: `npm install ccd-adapter-discord`, update `fleet.yaml`
- CCD auto-loads it

### Phase C: Adapter Developer Guide
- Document the contract
- Provide a `ccd-adapter-template` repo with boilerplate
- Add integration test helpers
