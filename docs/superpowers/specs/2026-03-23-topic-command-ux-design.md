# Topic Command UX Design

## Problem

Current topic binding flow requires users to understand Telegram Forum Topics, manually create one, send a message to trigger a directory browser, then select a project. Too many steps, too Telegram-specific.

## Solution

Turn the General topic into a control panel. Users manage project bindings via `/open` and `/new` commands. Topic creation is fully automated by the system.

## Commands

All commands are only valid in the General topic of the Telegram group.

### `/open`

List all directories under configured `project_roots` as an inline keyboard. User taps one to bind.

System then:
1. Calls `createForumTopic` with the directory basename as topic name
2. Creates instance config in `fleet.yaml`
3. Starts the daemon instance
4. Sends confirmation in the new topic

### `/open <keyword>`

Fuzzy search (substring match, case-insensitive) across all directories in `project_roots`.

- **Exact unique match** (keyword equals directory basename exactly, and only one match): auto-bind immediately, no confirmation needed.
- **Multiple matches**: list all matching directories as inline keyboard for user to pick.
- **Zero matches**: reply with "No projects found matching `<keyword>`."

### `/new <name>`

Create a new project from scratch.

1. Validate name (no `/`, `..`, or whitespace-only)
2. Create directory at `project_roots[0]/<name>`
3. Run `git init` in the new directory
4. Call `createForumTopic` with `<name>` as topic name
5. Create instance config + start daemon
6. Send confirmation in the new topic

If `<name>` is omitted, reply: "Usage: `/new <project-name>`"

## Unbound Topic Behavior

When a user sends a message to a manually-created (unbound) topic, the system no longer shows the directory browser. Instead, it replies:

> "Please use /open or /new in General to bind a project to a topic."

This keeps the entry point unified and avoids confusion.

## What Stays the Same

- **Topic deletion auto-unbind**: existing `handleTopicDeleted` + polling cleanup logic unchanged.
- **fleet.yaml instance structure**: no schema changes.
- **IPC, Daemon, message routing**: untouched.
- **DM mode**: unaffected (this only applies to topic mode).

## Implementation Scope

### Modified files

- **`src/fleet-manager.ts`**:
  - Add `handleGeneralCommand()` method to parse `/open` and `/new` from General topic messages
  - Add `handleOpenCommand(keyword?: string)` — directory listing/search + auto-create topic + bind
  - Add `handleNewCommand(name: string)` — create dir + git init + auto-create topic + bind
  - Modify `handleUnboundTopic()` — replace directory browser with redirect message
  - Register Telegram bot commands via `setMyCommands` API on startup

- **`src/channel/adapters/telegram.ts`**:
  - Ensure General topic messages (thread_id = undefined or 0) are routed to FleetManager
  - May need to handle bot command parsing if not already done by grammY

### New behavior in message flow

```
Message in General topic
  → FleetManager.handleInboundMessage()
    → threadId is 0/undefined → handleGeneralCommand()
      → parse /open or /new
      → execute command
      → reply in General topic with result or inline keyboard
```

### Directory listing

Reuse existing `project_roots` scanning logic from `handleUnboundTopic`. The inline keyboard format stays similar but callback data prefix changes to distinguish from the old flow (e.g., `open:<threadId>:<path>` → `cmd_open:<path>`).

### Bot commands registration

On fleet startup, call Telegram `setMyCommands` API to register `/open` and `/new` so they appear in Telegram's command autocomplete menu. Scope to group chat only.

## Edge Cases

- **`/open` when project is already bound**: skip it in the list, or mark it as "(active)" so user knows.
- **`/new` with existing directory name**: error message, don't overwrite.
- **Multiple `project_roots`**: `/open` lists all of them. `/new` uses the first one.
- **General topic messages that aren't commands**: ignore silently (don't reply with the redirect message — that's only for unbound non-General topics).
- **Topic creation API failure**: reply with error in General, don't leave orphan config.
