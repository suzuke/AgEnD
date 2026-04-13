# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- `replace_instance` MCP tool — atomically replace an instance with a fresh one, collecting handover context from the daemon's ring buffer and injecting it into the new instance via standard message delivery
- Communication efficiency rules in workflow template — no pleasantries, silence = agreement, batch points, one round-trip for reviews
- Webhook notifications for fleet events (rotation, hang, cost alerts)
- HTTP health endpoint (`/health`, `/status`) for external monitoring
- Structured handover template with validation and retry on context rotation
- Permission relay UX improvements (timeout countdown, "Always Allow" persistence, post-decision feedback)
- Topic icon auto-update (running/stopped) + idle archive
- Filter out Telegram service messages (topic rename, pin, etc.) to save tokens

### Changed
- **ContextGuardian simplified to pure monitoring** — removed max_age timer, state machine (NORMAL/RESTARTING/GRACE), and all restart triggers. All CLI backends (Claude Code, Codex, Gemini CLI, OpenCode, Kiro CLI) have built-in auto-compact that handles context limits internally.
- **Crash recovery tries --resume first** — on crash respawn, attempts `--resume` to restore full conversation history before falling back to fresh session + snapshot injection. Saves context when resume succeeds.

### Fixed
- Minimal `claude-settings.json` — only CCD MCP tools in allow list, no longer overrides user's global permission settings

## [1.14.0] - 2026-04-07

### Added
- **Plugin system + Discord adapter extraction** — Discord adapter moved to standalone `agend-plugin-discord` package; factory.ts supports `agend-plugin-{type}` / `agend-adapter-{type}` / bare name conventions; main package exports (`/channel`, `/types`) enable third-party plugins
- **Web UI Phase 2: full control dashboard** — instance stop/start/restart/delete with name confirmation, create instance form (directory optional, backend auto-detect), task board CRUD, schedule management, team management, fleet config editor (form-based with sensitive field masking)
- **Web UI layout: Fleet vs Instance** — sidebar "Fleet" entry for fleet-level tabs (Tasks, Schedules, Teams, Config); instance tabs limited to Chat + Detail; cross-navigation links between fleet and instance views
- **Web UI UX improvements** — toast notifications, loading states, cron human-readable descriptions, larger status dots, empty state guidance, cost labels, website-consistent styling (#2AABEE accent, Inter + JetBrains Mono fonts)
- **Backend auto-detection** — `GET /ui/backends` scans PATH for installed CLIs; Create Instance dropdown shows installed/not-installed status
- **Instance-specific restart** — `agend fleet restart <instance>` via fleet HTTP API (`POST /restart/:name`)
- **Bootstrap install script** — `curl -fsSL https://suzuke.github.io/AgEnD/install.sh | bash`
- **project_roots enforcement** — `create_instance` rejects directories outside configured roots

### Fixed
- **Web UI reply context** — first web message no longer causes "No active chat context"; uses real Telegram group_id/topic_id
- **Web↔Telegram bidirectional sync** — web messages forwarded to Telegram with `🌐` prefix; Telegram messages pushed to Web UI via SSE
- **SSE instant status refresh** — action buttons update immediately after stop/start/restart/delete
- **.env override** — `.env` file values unconditionally override inherited shell environment variables
- **tmux duplicate session race** — `ensureSession()` handles concurrent parallel startup
- **Create Instance form** — directory optional with dynamic topic_name requirement

### Changed
- **discord.js removed from core dependencies** — only needed when `agend-plugin-discord` is installed
- **Web API extracted to `web-api.ts`** — reduces fleet-manager.ts size; all `/ui/*` routes in dedicated module
- **Auth unified** — all Web UI endpoints (including restart) require token authentication

## [1.13.0] - 2026-04-06

### Added
- **Web UI Phase 2: full control dashboard** — create/delete instances, task board CRUD (create, claim, complete), schedule management (create, delete), team management (create with member checkboxes, delete), fleet config viewer (read-only, sanitized)
- **Web UI styling** — aligned with website design: Telegram blue `#2AABEE` accent, Inter + JetBrains Mono fonts, dark theme, rounded cards, toast notifications, loading states
- **Bootstrap install script** — `curl -fsSL https://suzuke.github.io/AgEnD/install.sh | bash` for one-line setup (Node.js via nvm, tmux, agend, backend detection)
- **project_roots enforcement** — `create_instance` rejects directories outside configured `project_roots` boundary
- **Auth unification** — all Web UI endpoints (including restart) require token authentication

### Fixed
- **Web UI reply context** — first message from Web UI no longer causes "No active chat context" error; uses real Telegram group_id/topic_id
- **Instant status refresh** — instance action buttons update immediately after stop/start/restart/delete via SSE
- **Web↔Telegram bidirectional sync** — web messages forwarded to Telegram topic with `🌐` prefix; Telegram messages pushed to Web UI via SSE

### Documentation
- Full documentation audit: 20+ missing features added across all docs
- Website redesigned with Spectra-inspired dark-first design

## [1.12.0] - 2026-04-06

### Added
- **Web UI dashboard** — `agend web` launches browser-based fleet monitoring with live SSE updates and integrated chat UI with bidirectional Telegram sync
- **agend quickstart** — simplified 4-question setup wizard replacing `agend init` as the recommended onboarding path
- **project_roots enforcement** — `create_instance` validates working directory is under configured `project_roots` boundary
- **HTML Chat Export** — `agend export-chat` exports fleet activity as self-contained HTML with date filtering (`--from`, `--to`)
- **Mirror Topic** — `mirror_topic_id` config for observing cross-instance communication in a dedicated topic

### Fixed
- **Parallel startup** — handle tmux duplicate session race when spawning many instances simultaneously
- **.env priority override** — `.env` file values now properly override inherited shell environment variables
- **Web UI chat sync** — bidirectional message sync between Web UI and Telegram

### Documentation
- README revamped with hero section, feature highlights, architecture diagram, and "How it works" flow
- Quick Start updated to use `agend quickstart` command
- Full documentation audit: features.md, cli.md, configuration.md updated with all v1.11.0-v1.12.0 features

## [1.11.0] - 2026-04-05

### Added
- **Kiro CLI backend** — new backend for AWS Kiro CLI (`backend: kiro-cli`). Session resume, MCP config, error patterns, models: auto, claude-sonnet-4.5, claude-haiku-4.5, deepseek-3.2, and more
- **Built-in workflow template** — fleet collaboration workflow auto-injected via MCP instructions. Configurable via `workflow` field in fleet.yaml (`"builtin"`, `"file:path"`, or `false`)
- **Workflow split: coordinator vs executor** — General instance gets full coordinator playbook (Choosing Collaborators, Task Sizing, Delegation Principles, Goal & Decision Management). Other instances get slimmed executor workflow (Communication Rules, Progress Tracking, Context Protection)
- **`create_instance` systemPrompt parameter** — agents can pass custom system prompts when creating instances (inline text only)
- **Fleet ready Telegram notifications** — `startAll` and `restartInstances` send "Fleet ready. N/M instances running." to General topic with failed instance reporting
- **E2E test framework** — 79+ tests running exclusively in Tart VMs. Mock backend with `pty_output` directive for error simulation. T15 workflow template tests, T16 failover cooldown tests
- **Token overhead measurement** — test script (`scripts/measure-token-overhead.sh`) and report. Full profile: +887 tokens (0.44% of 200K context, $0.003/msg)
- **Codex usage limit detection** — "You've hit your usage limit" error pattern (action: pause)
- **MockBackend error patterns** — `MOCK_RATE_LIMIT` and `MOCK_AUTH_ERROR` for E2E testing

### Fixed
- **Crash recovery snapshot restore** — write snapshot on crash detection (not just context rotation); replace single-consume file deletion with in-memory `snapshotConsumed` flag so file persists for daemon restart recovery (#11 related)
- **Codex session resume** — `CodexBackend.buildCommand()` now uses `codex resume <session-id>` when session-id file exists (#11)
- **Rate limit failover loop** — 5-minute cooldown on failover-type PTY errors prevents repeated triggering when error text persists in terminal buffer (#10)
- **PTY error monitor hash dedup** — record pane hash at recovery time; suppress same error on same screen to prevent stale re-detection loops
- **CLI restart wait** — replace fixed 1s delay between bootout/bootstrap with dynamic polling (up to 30s) for process exit. Fixes "Bootstrap failed: Input/output error" with many instances
- **CLI attach interactive selection** — fuzzy match ambiguity now shows numbered menu instead of error
- **CLI logs ANSI cleanup** — enhanced `stripAnsi()` handles cursor movement, DEC private modes, carriage returns, and remaining control characters
- **`reply_to_text` in agent messages** — user reply-to context now included in formatted messages pasted to agent
- **General instructions per-backend** — auto-create writes correct file based on `fleet.defaults.backend` (CLAUDE.md, AGENTS.md, GEMINI.md, .kiro/steering/project.md)
- **General instructions on every start** — `ensureGeneralInstructions()` called on every `startInstance` for general_topic instances, not just auto-create
- **Builtin text English-only** — all system-generated text translated from Chinese to English (schedule notifications, voice message labels, general instructions)
- **General delegation principles** — rewritten for coordinator role: delegate proactively with specific conditions instead of "do it yourself"

### Changed
- Fleet start/restart notifications unified to "Fleet ready. N/M instances running." format, sent to General topic
- `buildDecisionsPrompt()` dead code removed (intentionally disconnected in v1.9.0)
- `getActiveDecisionsForProject()` removed from fleet-manager (dead code)

### Documented
- OpenCode MCP instructions limitation (v1.3.10 doesn't read MCP instructions field)
- Kiro CLI MCP instructions limitation (unverified)
- Token overhead report (EN + zh-TW) with reproducible test script

## [1.10.0] - 2026-04-05

_Intermediate release, changes included in 1.11.0 above._

## [1.9.1] - 2026-04-03

### Fixed
- Session snapshot now injected on health-check respawn — crash/kill recovery also gets context restored
- Snapshot paste includes "do NOT reply" instruction to prevent model from attempting an IPC reply that times out

## [1.9.0] - 2026-04-03

### Breaking Changes
- **System prompt injection replaced with MCP instructions.** Fleet context, custom `systemPrompt`, and collaboration rules are now injected via MCP server instructions instead of CLI `--system-prompt` flags. This change was necessary because:
  - Claude Code: `--system-prompt` was passed a file path as literal text instead of file contents — the fleet prompt was **never correctly injected** since inception
  - Gemini CLI: `GEMINI_SYSTEM_MD` overwrites the built-in system prompt and breaks skills functionality
  - Codex: `.prompt-generated` was dead code — written to disk but never read by the CLI
  - OpenCode: `instructions` array was overwritten instead of appended, breaking existing project instructions
- **Impact on existing setups:**
  - `fleet.yaml` `systemPrompt` field is preserved — it now injects via MCP instructions instead of CLI flags
  - `.prompt-generated`, `system-prompt.md`, `.opencode-instructions.md` files are no longer generated
  - Each CLI's built-in system prompt is no longer overridden or modified
  - Active Decisions are no longer preloaded into the system prompt — use `list_decisions` tool on demand
  - Session snapshots (context rotation) are now delivered as the first inbound message (`[system:session-snapshot]`) instead of being embedded in the system prompt

## [1.8.5] - 2026-04-03

### Fixed
- Unified log and notification format to `sender → receiver: summary` style across all cross-instance messages
- Task/query notifications now show the full message body; report/update notifications show only the summary

## [1.8.4] - 2026-04-03

### Fixed
- Cross-instance notification format: `sender → receiver: summary` for clarity
- General Topic instances no longer receive cross-instance notification posts
- Reduced cross-instance notification noise — sender topic post removed; target notification uses `task_summary` when available

## [1.8.3] - 2026-04-03

### Added
- **Team support** — named groups of instances for targeted broadcasting
  - `create_team` — define a team with members and optional description
  - `list_teams` — list all teams with member details
  - `update_team` — add/remove members or update description
  - `delete_team` — remove a team definition
  - `broadcast` now accepts a `team` parameter to target all members of a named team
  - `teams` section in `fleet.yaml` for persistent team definitions

## [1.8.2] - 2026-04-03

### Added
- `working_directory` is now optional in fleet.yaml — auto-created at `~/.agend/workspaces/<name>` when missing
- `create_instance` `directory` parameter is now optional (auto-workspace created when omitted)

### Fixed
- Context-bound routing now runs before IPC forwarding in topic mode (prevented "chat not found" errors)
- Telegram: `thread_id=1` correctly treated as General Topic (no message thread)
- Scheduler initializes before instances start, so active decisions load correctly on fleet spawn

## [1.8.1] - 2026-04-03

### Added
- `reply`, `react`, `edit_message` are now context-bound — `chat_id` and `thread_id` are no longer required in tool calls; the daemon fills them from the active conversation context
- Backend error pattern detection via PTY monitoring — auto-notify on rate limits, auth errors, and crashes
- Auto-dismiss runtime dialogs (e.g. Codex rate limit model-switch prompts)
- Model failover — auto-switch to backup model on rate limit (statusline + PTY detection)

### Fixed
- Recovery notification sent after PTY error monitor detects and handles an error
- Error monitor false positives reduced; invalid `chat_id` auto-corrected from context

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
