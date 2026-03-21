# claude-channel-daemon

A reliable daemon wrapper for [Claude Code](https://docs.anthropic.com/en/docs/claude-code) Channels. Runs Claude Code CLI as a long-lived background service with automatic session management, context window rotation, and memory backup.

[中文版 README](README.zh-TW.md)

> **⚠️ Security Notice:** This daemon runs Claude Code with `bypassPermissions` mode. All permission decisions are delegated to a PreToolUse hook backed by a remote approval server (Telegram inline buttons). If the approval server is unreachable, **all tool calls are denied** until it recovers. The built-in deny list blocks known destructive commands (`rm -rf /`, `git push --force`, etc.), but this setup fundamentally trusts the approval server for access control. **Use at your own risk.** Review the [Permissions](#permissions) section before deploying.

## Why

Claude Code's Telegram plugin requires an active CLI session — close the terminal and the bot dies. This daemon solves that by:

- Running Claude Code in the background via `node-pty`
- Automatically restarting on crashes with exponential backoff
- Rotating sessions when context usage gets too high
- Backing up memory to SQLite
- Installing as a system service (launchd / systemd)

## Quick Start

```bash
# Clone and install
git clone https://github.com/suzuke/claude-channel-daemon.git
cd claude-channel-daemon
npm install

# Interactive setup
npx tsx src/cli.ts init

# Start the daemon
npx tsx src/cli.ts start
```

## CLI Commands

```
claude-channel-daemon start    Start the daemon
claude-channel-daemon stop     Stop the daemon
claude-channel-daemon status   Show running status
claude-channel-daemon logs     Show daemon logs (-n lines, -f follow)
claude-channel-daemon install  Install as system service
claude-channel-daemon uninstall Remove system service
claude-channel-daemon init     Interactive setup wizard
```

## Architecture

```
┌─────────────────────────────────────────────┐
│              claude-channel-daemon           │
│                                             │
│  ┌─────────────────┐  ┌──────────────────┐  │
│  │ Process Manager  │  │ Context Guardian │  │
│  │ (node-pty)       │  │ (rotation)       │  │
│  └────────┬─────────┘  └────────┬─────────┘  │
│           │                      │            │
│  ┌────────┴─────────┐  ┌────────┴─────────┐  │
│  │  Memory Layer     │  │   Service        │  │
│  │  (SQLite backup)  │  │   (launchd/      │  │
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

Spawns Claude Code via `node-pty` with channel mode enabled. Handles session persistence (resume via UUID), graceful shutdown (`/exit`), and automatic restarts with configurable backoff.

### Context Guardian

Monitors context window usage via Claude Code's status line JSON. Triggers session rotation when usage exceeds the configured threshold or max session age. Supports three strategies: `status_line`, `timer`, or `hybrid`.

### Memory Layer

Watches Claude's memory directory with chokidar and backs up files to SQLite for persistence across session rotations.

### Service Installer

Generates and installs system service files — launchd plist for macOS, systemd unit for Linux. Starts automatically on boot.

## Configuration

Config file: `~/.claude-channel-daemon/config.yaml`

```yaml
channel_plugin: telegram@claude-plugins-official
working_directory: /path/to/your/project

restart_policy:
  max_retries: 10
  backoff: exponential  # or linear
  reset_after: 300      # seconds of stability before resetting retry counter

context_guardian:
  threshold_percentage: 80  # rotate when context reaches this %
  max_age_hours: 4          # max session age before rotation
  strategy: hybrid          # status_line | timer | hybrid

memory:
  auto_summarize: true
  watch_memory_dir: true
  backup_to_sqlite: true

log_level: info  # debug | info | warn | error
```

## Data Directory

All state is stored in `~/.claude-channel-daemon/`:

| File | Purpose |
|------|---------|
| `config.yaml` | Main configuration |
| `daemon.pid` | Process ID (while running) |
| `session-id` | Saved UUID for session resume |
| `statusline.json` | Current context/cost status |
| `claude-settings.json` | Injected Claude Code settings |
| `memory.db` | SQLite memory backup |
| `.env` | Telegram bot token |

## Permissions

**This daemon uses `bypassPermissions` mode.** Claude Code's built-in permission prompts are completely disabled. All access control is handled by a three-layer system:

1. **PreToolUse hook** — Every tool call is POSTed to the Telegram plugin's approval server (`127.0.0.1:18321`). The server decides allow/deny based on danger detection.

2. **Danger detection** — The approval server uses regex patterns to classify operations:
   - **Safe** (auto-approved): read-only operations, safe bash commands, web searches
   - **Dangerous** (requires Telegram approval): `rm`, `sudo`, `git push`, `chmod`, sensitive file paths (`.env`, `.claude/settings.json`)
   - **Hardcoded deny list**: `rm -rf /`, `git push --force`, `git reset --hard`, `dd`, `mkfs`

3. **Health check + fail-safe** — The daemon pings the approval server every 30 seconds. If it's unreachable:
   - All tool calls are **denied** (not allowed)
   - Warning logged every 60 seconds until recovery

**Why `bypassPermissions`?** Claude Code has internal protected paths (e.g., `~/.claude/skills/`) that trigger terminal permission prompts even when the PreToolUse hook returns "allow". In a headless daemon, these prompts block indefinitely with no way to respond. `bypassPermissions` prevents this by delegating all decisions to the hook layer.

**Risk:** If someone gains access to `127.0.0.1:18321`, they can approve arbitrary operations. The server only listens on localhost and validates against the Telegram access allowlist.

## Requirements

- Node.js >= 20
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) CLI installed
- Telegram bot token (created via [@BotFather](https://t.me/BotFather))

## License

MIT
