# Distribution + Observability Roadmap

**Date:** 2026-03-26
**Status:** Approved
**Context:** CCD's positioning as a Claude Code-specialized headless fleet, differentiating from OpenClaw's breadth-first approach.
**Reviewed by:** ccplugin instance (feedback integrated)

## Strategy

Short-term: deepen the Claude Code fleet moat (vertical depth).
Long-term: expand toward broader competition with OpenClaw.

This spec covers the short-term: make CCD usable by strangers and add fleet intelligence that OpenClaw can't replicate.

## Phase 1: Distribution + Foundation

Goal: a stranger can go from zero to running fleet in < 5 minutes, without risking bill shock.

### 1.1 npm publish + install simplification

- Add `files` field to `package.json` (dist/, templates/, README.md)
- Add `prepublishOnly: "npm run build"` script
- Publish to npm → `npx claude-channel-daemon init` works
- Simplify Quick Start to 3 steps: install, init, start

### 1.2 GitHub Actions CI

Single workflow file:
- On PR: `vitest run` + `tsc --noEmit`
- On tag push (v*): `npm publish`
- No matrix builds, no Docker — one job each

### 1.3 Event log (infrastructure)

Promoted from Phase 2 — this is the foundation all observability features write to.

New `events` table in existing SQLite db:

```sql
CREATE TABLE events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  instance_name TEXT NOT NULL,
  event_type TEXT NOT NULL,
  payload TEXT,              -- JSON
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_events_instance ON events(instance_name, created_at);
CREATE INDEX idx_events_type ON events(event_type, created_at);
```

Event types: `cost_snapshot`, `permission_decision`, `schedule_run`, `context_rotation`, `hang_detected`, `instance_paused`, `instance_resumed`

CLI: `ccd fleet history [--instance <name>] [--type <type>] [--since <date>]`

Why early: cost guard (1.4) and daily summary (2.3) both need a place to write/query events. Building this first avoids retrofitting.

### 1.4 Cost guard

New config in `fleet.yaml`:

```yaml
defaults:
  cost_guard:
    daily_limit_usd: 50
    warn_at_percentage: 80
    timezone: "Asia/Taipei"   # defaults to system timezone
```

Implementation:
- Fleet manager reads `statusline.json` periodically (already watched by context guardian)
- Maintain per-instance daily cost counter, **stored as integer cents** to avoid floating-point drift
- On context rotation: snapshot current statusline cost into events table BEFORE killing the session (statusline may be stale or cleared after kill), then carry over to the accumulator
- Reset at midnight in configured timezone
- At warn threshold: send Telegram notification to instance's topic
- At limit: pause instance + notify + log `instance_paused` event. Resume next day or manual `ccd fleet start <name>`
- Fleet-level aggregate shown in /status

**Cost tracking detail:** `statusline.json`'s `cost.total_cost_usd` is per-session and resets on new spawn. The daemon must maintain:

```
daily_cost_cents = sum(previous_sessions_cost_cents) + current_statusline_cost_cents
```

The context guardian must snapshot cost into the events table when entering ROTATING state (before kill), not after.

### 1.5 Telegram /status command

New command in General topic:

```
🔵 proj-a — idle, ctx 42%, $3.20 today
🟢 proj-b — working, ctx 67%, $8.50 today
⏸ proj-c — paused (cost limit)

Fleet: $11.70 / $50.00 daily
```

Data sources: existing statusline.json + cost counter from 1.4.

### 1.6 Graceful shutdown notification

When `ccd fleet stop` is called:
- Send a "please save your current state" prompt to each active instance
- Wait up to 30s for Claude to finish (same pattern as graceful restart's idle detection)
- Then kill tmux window

Prevents losing in-progress work (e.g., half-written commits).

## Phase 2: Observability

Goal: make fleet operation transparent and controllable from Telegram.

### 2.1 Rate limit-aware scheduling

- Read `rate_limits.five_hour` and `rate_limits.seven_day` from statusline
- When 5hr usage > 85%: defer scheduler triggers (don't drop — queue them)
- Fleet manager maintains a global awareness across instances
- Telegram notification when a schedule is deferred: post to the instance's topic
- Log `schedule_deferred` event

### 2.2 Hang detection

- Add `last_activity_ts` tracking in daemon (transcript monitor already watches output)
- If instance state is "working" but no transcript change for N minutes (configurable, default 15): flag as potentially hung
- **Multi-signal detection:** don't rely on transcript alone. Also check:
  - statusline.json freshness (is it still being updated?)
  - tmux pane alive check (is the process still running?)
  - This avoids false positives when Claude is running long tool calls (tests, builds) where transcript is quiet but work is happening
- Telegram notification to instance's topic with inline buttons: "Force restart / Keep waiting"
- Not auto-restart — user decides
- Log `hang_detected` event

### 2.3 Daily summary

- Configurable time (default: 21:00 in cost_guard.timezone)
- Posted to General topic (single message, full fleet overview)
- Content:
  - Per-instance: message count, schedule runs, cost
  - Fleet totals: cost, context rotations, rate limit status
  - Anomalies highlighted (hangs, cost warnings, incomplete handovers)
- **Data source: events table** (primary) + statusline for live state
- Alerts (cost warnings, hangs) go to individual instance topics in real-time; daily summary is the aggregate view in General

### 2.4 Context rotation quality tracking

- When context guardian enters HANDING_OVER state, start a timer
- After rotation completes, check if `memory/handover.md` exists and is non-empty
- Log `context_rotation` event with payload: `{ handover_status: "complete" | "timeout" | "empty", duration_ms, previous_context_pct }`
- Daily summary flags instances with frequent incomplete handovers

## Dependencies

```
1.1 npm publish ──────────────────────────── (independent)
1.2 GitHub Actions CI ────────────────────── (independent)
1.3 Event log ─────┬──────────────────────── (independent, do first)
1.4 Cost guard ────┤ (writes to event log)
1.5 /status ───────┤ (reads cost data)
1.6 Graceful stop ─┘ (independent)

2.1 Rate limit scheduling ── (needs 1.3 event log)
2.2 Hang detection ───────── (needs 1.3 event log)
2.3 Daily summary ────────── (needs 1.3 event log + 1.4 cost data)
2.4 Rotation quality ─────── (needs 1.3 event log)
```

Recommended build order within Phase 1: 1.3 → (1.1, 1.2, 1.4, 1.5, 1.6 in parallel)

## Non-goals

- Web dashboard (future)
- Multi-channel support (future, long-term)
- Multi-LLM backend (future, long-term)
- Fleet orchestration / task delegation (future, after observability is solid)
