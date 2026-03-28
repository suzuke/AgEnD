# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Webhook notifications for fleet events (rotation, hang, cost alerts)
- HTTP health endpoint (`/health`, `/status`) for external monitoring
- Model failover — auto-switch to backup model on rate limit
- Structured handover template with validation and retry on context rotation
- Permission relay UX improvements (timeout countdown, "Always Allow" persistence, post-decision feedback)
- Topic icon auto-update (running/stopped) + idle archive
- Filter out Telegram service messages (topic rename, pin, etc.) to save tokens

### Fixed
- Minimal `claude-settings.json` — only CCD MCP tools in allow list, no longer overrides user's global permission settings

## [0.3.7] - 2026-03-27

### Added
- `delete_instance` MCP tool for removing instances
- `create_instance --branch` — git worktree support for feature branches
- External adapter plugin loading — community adapters via `npm install ccd-adapter-*`
- Export channel types from package entry point for adapter authors
- Discord adapter (MVP) — connect, send/receive messages, buttons, reactions
- Per-instance restart notifications in Telegram topics after graceful restart

### Fixed
- `start_instance`, `create_instance`, `delete_instance` added to permission allow list
- Worktree instance names use `topic_name` instead of directory basename to avoid Unix socket path overflow (macOS 104-byte limit)
- `create_instance` with branch no longer triggers false `already_exists` on base repo
- postLaunch stability check replaced with 10s grace period
- Restart notification uses `fleetConfig.instances` + IPC push
- Discord adapter TypeScript errors resolved

## [0.3.6] - 2026-03-27

### Fixed
- Prevent MCP server zombie processes on instance restart
- Harden postLaunch auto-confirm against edge cases

## [0.3.5] - 2026-03-26

### Added
- Per-instance model selection via `create_instance(model: "sonnet")`
- Instance `description` field for better discoverability in `list_instances`
- Auto-prune stale external sessions from sessionRegistry (every 5 minutes)
- AgEnD landing page website (Astro + Tailwind, bilingual EN/zh-TW)
- GitHub Actions workflow for website deployment
- Security considerations section in README

### Changed
- Simplify model selection — only configurable via `create_instance`, not per-message
- Use single `query_sessions_response` for session pruning

### Fixed
- Security hardening — 10 vulnerability fixes (path traversal, input validation, etc.)
- Send full cross-instance messages to Telegram instead of 200-char preview truncation
- Remove IPC secret auth — socket `chmod 0o600` is sufficient and simpler

## [0.3.4] - 2026-03-26

### Changed
- Remove slash commands (`/open`, `/new`, `/meets`, `/debate`, `/collab`) — General instance handles project management via `create_instance` / `start_instance`
- Remove dead code: `sendTextWithKeyboard`, `spawnEphemeralInstance`, meeting channel methods

## [0.3.3] - 2026-03-25

### Fixed
- Correct `statusline.sh` → `statusline.js` in test assertion

## [0.3.2] - 2026-03-25

### Added
- Channel adapter factory with dynamic import for future multi-platform support
- Intent-oriented adapter methods: `promptUser`, `notifyAlert`, `createTopic`, `topicExists`
- "Always Allow" button on Telegram permission prompts
- Per-instance `cost_guard` field in InstanceConfig
- `topology` property on ChannelAdapter (`"topics"` | `"channels"` | `"flat"`)

### Changed
- Channel abstraction Phase A — remove all TelegramAdapter coupling from business logic (fleet-manager, daemon, topic-commands now use generic ChannelAdapter interface)
- CLI version reads from package.json instead of hardcoded value
- Schedule subcommands now have `.description()` for help text

### Fixed
- Shell injection in statusline script — replaced bash with Node.js script
- Timezone validation in setup wizard and config (Intl.DateTimeFormat)
- `max_age_hours` default aligned to 8h across setup-wizard, config, and README
- `pino-pretty` moved from devDependencies to dependencies (fixes `npm install -g`)
- `toolStatusLines` cleared on respawn to prevent unbounded growth
- Try-catch for `--config` JSON.parse in daemon-entry
- Dead code `resetToolStatus()` removed
