# agend-terminal — Feature Inventory (for rewrite keep/drop/improve decisions)

> 歸檔註：設計階段的原始紀錄，內容原樣保存（2026-09-24）。只把絕對路徑換成佔位符：`<scratchpad>` = 當時的暫存目錄、`<v1-repo>` = agend-terminal repo、`<tmp>` = 系統暫存目錄、`~` = 使用者家目錄。文中提到的腳本、log、schema dump 沒有一起歸檔。

Repo: `<v1-repo>`. Read-only investigation, no files modified.

## Methodology

**LOC counting.** A naive "lines before the first `#[cfg(test)]`" script badly
undercounts (and sometimes overcounts) production LOC across this codebase,
because two patterns are common:
1. A small `#[cfg(test)]`-gated helper/const/import appears **early** in a file,
   long before the real trailing `mod tests { ... }` block — the naive script
   truncates there and throws away hundreds/thousands of real production lines
   after it (worst cases found: `app/app_state.rs` 102→1994, `agent/mod.rs`
   37→3132, `main.rs` 23→~2098, `binding.rs` 29→~923, `worktree_pool.rs`
   41→~2473, `token_cost.rs` 255→1061).
2. A file named `review_repro_*.rs` (or similar) is declared via
   `#[cfg(test)] mod x;` **in its parent file**, so the file itself contains no
   `#[cfg(test)]` marker and gets counted as 100% production despite being
   pure test/regression scaffolding (found repeatedly, e.g.
   `app/review_repro_app_tui.rs`, `cleanup_intents/owner_attestation`-adjacent
   files, `api/handlers/set_model_success_3573.rs`).

All numbers below are **hand-corrected**: 9 parallel agents each verified their
assigned files against the real trailing test-module boundary (or the parent's
`mod` gate) and against real call sites, then a 10th gap-fill agent covered
subdirectories the initial flat file lists missed
(`channel/telegram/`, `channel/discord/`, `api/handlers/`, `tray/{autostart,terminal}/`,
a few `transport/` nested files). Duplicate coverage between agents was
reconciled during synthesis (see `§0.1`).

**Verification.** Every one-liner below is checked against a real caller
(MCP dispatch table, CLI enum handler, daemon per-tick registration, or a
direct `grep -rn` for the symbol) — not against the module's own `//!` header
or a `docs/FEATURE-*.md` claim. Places where docs/comments disagree with code
are listed separately in §3. Anything not independently verified is tagged
**未驗證**.

### §0.1 Grand total

**≈182,952 corrected production lines** across `src/` (raw total incl. tests:
494,905 lines, `wc -l`). This supersedes both the naive first-pass estimate
(~146,647) and an intermediate brace-depth-parser estimate (~231,785, which
over-counts because it can't see cross-file `#[cfg(test)] mod x;` gates).

---

## §1. Subsystem overview (16 groups)

| # | Subsystem | Files | Prod LOC | One-line purpose (verified) |
|---|---|---:|---:|---|
| 1 | Daemon core tick loop / supervisor / crash-respawn / restart | 35 | 8,947 | `daemon::mod::run_core` (`daemon/mod.rs:773`) is the main loop; `per_tick::build_default_handlers` (`per_tick/mod.rs:365`) builds the ordered per-tick handler pipeline that does crash/hang/respawn detection every tick. |
| 2 | Daemon: task/board watchdogs (anti-stall, idle, dispatch-idle, hygiene) | 13 | 5,930 | 12 trackers migrated from `supervisor.rs` into `per_tick/supervisor_trackers.rs`, registered in `build_default_handlers` (`per_tick/mod.rs:546-556`); `task_sweep::TaskSweep::spawn` is its own background thread (`daemon/mod.rs:1362`). |
| 3 | Daemon: CI/PR lifecycle tracking (ci_watch, pr_state, reviewer-assignment authority) | 24 | 7,975 | `CiWatchPollHandler`/`PrStateScanHandler`/`AssignmentReconcileHandler` registered at `per_tick/mod.rs:427-437`; `assignment_authority.rs` is the durable crash-safe reviewer-assignment store (122 external references). |
| 4 | Daemon: decision-timeout, notification/discharge ledgers, binding-maintenance, shadow observer, boot/GC sweeps, bootstrap preflight | 73 | 13,918 | Everything else in `daemon/`+`bootstrap/`: decision auto-answer timeouts, inbox/notification dedup ledgers, worktree-maintenance daemons (`auto_release`, `janitor::dispose` as the single worktree-destruction funnel), the `shadow/` state-reconciliation plane, boot-time zombie/orphan sweeps, and `bootstrap::prepare`'s ordered boot sequence (`bootstrap/mod.rs:263`). |
| 5 | Agent/PTY lifecycle + backend abstraction + transport protocols | 47 | 25,520 | PTY spawn/lifecycle (`agent/mod.rs`, `agent_ops.rs`), backend command construction for 6 backends (Claude/Codex/OpenCode/KiroCli/Agy/Grok), typed inject + injection gating, the vterm emulator, output capture, and 3 coexisting transport tiers (`NativeShared` for Codex/OpenCode, `ChannelBridge` for Claude, `LegacyPty` for everything else) — see `transport/registry.rs:25-38`. |
| 6 | MCP tool dispatch + CLI front-end + git-shim/bridge binaries | ~100 | 22,714 | `mcp/registry.rs` pairs all 34 tools with schema+handler+timeout class; `handlers/dispatch.rs::try_dispatch` is the sole dispatch chokepoint; `cli.rs`/`main.rs` hold the full `agend-terminal` command tree; `agend-mcp-bridge` is the stdio↔TCP relay every backend actually spawns as its MCP server; `agend-git`/`agentic-git` are PATH-shim binaries (agend-git is now kill-guard-only). |
| 7 | Fleet config / agent↔worktree binding / teams / git worktree management | 41 | 21,676 | `fleet.yaml` load/merge/resolve/persist; HMAC-signed agent-worktree binding (`binding.rs`, load-bearing for task-completion trust, not just diagnostic); worktree pool provisioning/GC/build-cache; worktree cleanup/occupancy; branch-sweep + cleanup-intents (post-merge branch deletion); admin orphan/zombie recovery; teams (org structure). |
| 8 | TUI (terminal multiplexer: app/render/layout) | ~65 | 18,831 | `Commands::App` → `app::run` → `run_app`'s event loop (`app/mod.rs:154,480-491`) draws via `render::render_with_team` and dispatches key/mouse actions through `keybinds.rs`+`dispatch.rs`; pane layout engine (tabs/splits/tree); command palette; **is always a thin client** (`attached_mode = true` hardcoded, `app/mod.rs:440`) that auto-spawns a detached daemon if none is listening. |
| 9 | Task board + decisions (cross-agent task coordination) | 22 | 14,963 | `task` MCP tool (14 actions) → `tasks::handler::handle` (`tasks/handler.rs:42`) → durable JSONL event log (`task_events.rs`); `decision` MCP tool (6 actions) → `decisions.rs`; ACL/governance/operator-settlement gates who may mutate what. |
| 10 | Diagnostics & operator-facing utility suite (state detection, health engine, quickstart, schedules+schedule_jobs, tray, deployments, skills sync, instruction injection, provider auto-detect, daemon JSON control-plane incl. `api/handlers/*`, OS service registration, e2e verify) | ~55 | 18,493 | PTY-output → 17-variant `AgentState` (`state/mod.rs:36-84`) → `HealthState` recovery engine (`health.rs`); `quickstart` wizard; two-layer schedule system (plain reminders vs. durable daemon-owned jobs); system tray app; batch team `deployment` tool; skills sync to 6 backend dirs; per-backend instruction injection; the daemon's JSON control API (`api/mod.rs` + `api/handlers/*` — team/query/instance/hook_event/messaging/mcp_proxy/external); OS service install; `agend-terminal verify`. |
| 11 | Messaging/inbox abstraction (durable inbox, ledgers, dedup, channel trait layer, operator paging) | 33 | 11,073 | Durable per-agent JSONL inbox with flock-serialized RMW (`inbox/*`); sent/reply audit ledgers; two independent dedup mechanisms (`notification_queue` = draft-state defer, `channel/dedup.rs` = 5s TTL outbound dedup); `Channel` trait + `UxSinkRegistry` fan-out that Telegram/Discord adapters implement against; orchestrator-only operator-paging escape hatch. |
| 12 | Telegram bot client (concrete implementation) | 14 | 3,959 | `teloxide` 0.17 long-polling bot, its own thread+Tokio runtime, wired at boot via `bootstrap::telegram_init::init` (`bootstrap/telegram_init.rs:63`) → `channel::telegram::init_from_config`; forum-topic routing (`topic_registry.rs`, 13+ external call sites). |
| 13 | Discord bot client (concrete implementation) | 8 | 1,173 | `twilight` 0.17 (gateway+http+model) bot, wired at boot via `bootstrap::discord_init::init` (`bootstrap/discord_init.rs:41`) → `channel::discord::init_from_config` → `gateway::start_gateway` opens the live WebSocket shard. |
| 14 | Governance/receipt machinery (claim verification, review/merge receipts, protected-ref gate) | 3 | 1,292 | `claim_verifier.rs` gates `git push` (pre-push hook) by diffing a stated claim against the real `git diff`; `review_receipt.rs`/`merge_receipt.rs` are server-side authority proofs gating message report-authority and task-completion settlement; `protected_refs.rs` (`is_protected_ref`) is consulted by 5+ subsystems before treating a branch as leaseable/watchable. |
| 15 | Runtime-config store + operator-mode authority gate + integrity signing | 5 | 1,075 | `runtime_config.rs` backs `admin config-set`; `operator_mode.rs` is the single daemon API-ingress authority gate (`api/mod.rs:886`), fail-closed to `Away` on any tamper/missing state; `config_integrity.rs`/`integrity_core.rs` HMAC-sign it. |
| 16 | Misc utility long-tail (logging/audit trail, token-cost accounting, fleet/process-state primitives) | ~30 | 5,413 | Rolling daemon-log tracing; append-only audit loggers; lock-depth/flock instrumentation; JSONL log-retention sweeps; FD-limit management; on-demand Claude/Codex token-cost accounting (`token_cost.rs`, 1061 LOC, **zero FEATURE-*.md coverage**); shared primitives (`paths`, `identity`, `error`, `types`, `macros`, etc.), all confirmed to have real external callers. |

**Total: 182,952.**

---

## §2. CLI subcommands (verified directly from `src/main.rs:296-920` clap enum — ground truth, not delegated)

| Command | Purpose |
|---|---|
| `start [--foreground] [--fleet] [--agents ...]` | Start the daemon (detached by default). |
| `attach <name>` | Attach to one agent's terminal (raw passthrough via `BridgeClient`, bypasses the TUI entirely). |
| `restart-probe` (hidden) | Read-only preflight probe before an owner-restart. |
| `hook-event` (hidden) | Backend lifecycle-hook reporter, always exits 0. |
| `channel-bridge` (hidden) | MCP subprocess for Claude Code's research-preview channel. |
| `tool` (hidden) | Generic MCP tool front end (`list`/`schema` discovery + direct call). |
| `inject <name> <text>` | Send input to an agent's PTY. |
| `list` / `ls` / `status` [--json] [--detailed] | List running agents. |
| `connect <name> --backend <b>` | Connect an external agent to the daemon. |
| `app [--fleet]` | Launch the TUI (always thin-client, see §1 row 8). |
| `stop [--no-wait] [--timeout]` | Stop the daemon, wait for exit by default. |
| `kill <name>` | Kill an agent. |
| `mode <active\|away\|sleep>` | Set operator availability mode. |
| `admin <subcommand>` | Admin utilities (below). |
| `capture backend\|promote` | Capture/promote agent output for fixture recording. |
| `verify [--json] [--backend] [--quick]` | Run end-to-end verification suite. |
| `service install\|uninstall\|status` | Manage the OS-level service registration. |
| `doctor [protocol\|providers\|topics\|orphans]` | Health/diagnostics checks. |
| `skills add\|remove\|list\|update\|install` | Manage shared cross-backend agent skills. |
| `tray` (feature-gated) | Launch the system tray app. |
| `quickstart [--unattended]` | Interactive first-time setup. |
| `bugreport` | Generate a bug report bundle. |
| `completions <shell>` | Generate shell completions. |
| `verify-push --base --head [--claim...]` | Verify a push claim against the actual diff (pre-push hook gate). |

**`admin` subcommands**: `resolve-job-recovery`, `task-settlement-preview`, `task-settlement-apply`, `cleanup-branches`, `cleanup-zombies`, `task-sweep-config` (moved from MCP, zero-calls), `gc-dry-run` (moved from MCP), `recover-worktree`, `tokens` (moved from MCP), `watchdog` (moved from MCP), `config-set` (moved from MCP's `config` tool's `set` action).

---

## §3. MCP tools — all 34 (verified directly from `src/mcp/registry.rs` + `src/mcp/tools.rs` `def_*` schema strings — ground truth)

`docs/MCP-TOOLS.md` / `.zh-TW.md` both declare 34 tools and are **mechanically kept in sync** by a compiled test (`registry.rs:897-922`, `docs_match_registry_tool_set`) that fails CI on drift — confirmed zero drift today.

| Tool | Purpose |
|---|---|
| `reply` | Reply to the user via the active channel; supports timeout+default-action operator decisions. |
| `operator_page` | Orchestrator-only: page the operator's Telegram directly, independent of channel binding, rate-limited 3/hr. |
| `download_attachment` | Download a Telegram multimedia attachment by file_id. |
| `send` | Send/broadcast a message to instance(s); replaces 4 legacy single-purpose tools. |
| `inbox` | Drain/lookup/thread/clear/confirm pending inbox messages. |
| `list_instances` | List active agent instances (or one in detail). |
| `create_instance` | Create agent instance(s), homogeneous or heterogeneous teams. |
| `delete_instance` | Stop and remove an instance. |
| `start_instance` | Start a stopped instance. |
| `restart_instance` | Kill+restart (resume or fresh mode). |
| `set_model` | Persist an instance's model/tier/effort intent. |
| `bind_topic` | Retrofit a deferred Telegram topic binding onto an instance. |
| `interrupt` | Send ESC to an agent's PTY to interrupt the current turn. |
| `set_metadata` | Set display name / description. |
| `set_waiting_on` | Declare what an instance is currently blocked waiting for. |
| `move_pane` | Move an instance's TUI pane into a different tab. |
| `pane_snapshot` | Read visible PTY scrollback text. |
| `instance` | Folded read-only alias for per-name instance queries. |
| `decision` | Manage decisions: post/list/get/update/answer/archive_batch. |
| `task` | Manage task board: 14 actions (create/list/get/claim/done/... /orphan_reconcile_apply). |
| `restart_daemon` | Request a graceful self-respawning daemon restart. |
| `team` | Manage teams: create/delete/list/update. |
| `schedule` | Manage reminders + daemon-owned jobs: create/list/update/delete/runs/complete/deliver/resolve_recovery. |
| `deployment` | Manage batch multi-agent deployments: deploy/teardown/list. |
| `ci` | CI watching/handoff: watch/unwatch/status/defer/ack_handoff. |
| `health` | Report/clear an agent's health/blocked-reason state. |
| `config` | Runtime config: get/list only (set moved to CLI). |
| `repo` | Worktree lifecycle: checkout/release/cleanup_init_commits/cleanup_merged_branches/merge. |
| `bind_self` | Bind the calling agent to a worktree on a named branch. |
| `release_worktree` | Release a daemon-managed worktree and clear its binding. |
| `binding_state` | Structured daemon-side bind-state diagnostic report. |
| `revoke_review_assignment` | Revoke a specific reviewer assignment. |
| `correct_review_class` | Operator-only audited correction of a PR's review class. |
| `usage_limit_takeover` | Operator-only: validate+prepare a usage-limit-episode takeover. |

**Removed from MCP surface (zero calls in 20 days, now CLI-admin-only, confirmed on both sides of the move):** `task_sweep_config`, `gc_dry_run`, `tokens`, `watchdog`, `config`'s `set` action. Evidence: `main.rs:625-734` doc comments + absence from `registry.rs:350-583`'s `ALL_TOOLS`, corroborated by `tools.rs:922-949`'s tool-count-changelog test.

---

## §4. Docs vs. code mismatches (file:line evidence; "claim" = doc, "actual" = code)

1. **TUI mode model is stale.** `docs/FEATURE-tui.md` (Owned vs Attached mode, operator-selectable) vs `src/app/mod.rs:440` (`let attached_mode = true;`, unconditional, no branch — commit `8e7f1203` "make TUI a permanent thin client (#3344)"). Doc's "stops daemon on exit" claim for owned mode is also false under current code (no teardown call found).
2. **`kill` does not set `restarting`.** `docs/FEATURE-agent-interaction.md:201` claims kill marks state `restarting`; actual code path (`api/handlers/instance.rs:27` → `agent/crash_disposition.rs:384-389` → `state/mod.rs:2266-2268`) sets `AgentState::Crashed` — same funnel as a real PTY crash.
3. **CLI `--backend` help text has drifted.** `main.rs:429` lists only `claude, kiro-cli, codex, opencode, agy`; `backend.rs:138` also parses a fully real `grok` backend (and `Shell`/`Raw(_)`), and the doc's own example uses `--backend grok`.
4. **`AgentState`/`HealthState` tables in `docs/FEATURE-health.md` are stale/incomplete.** Doc lists `Ready`/`Thinking`/`ToolUse` (none exist — `Active` replaced Thinking+ToolUse per `state/mod.rs:47`) and omits 8 real variants (`GitConflict`, `ContextFull`, `ServerRateLimit`, `UsageLimit`, `AuthError`, `ApiError`, `ModelUnsupported`, `Restarting`) plus 2 real `HealthState` variants (`Absent`, `Unhealthy`).
5. **`docs/FEATURE-dispatch-idle.md` documents a different module than its similarly-named sibling.** It correctly documents `daemon/dispatch_idle/{mod,team_nudge}.rs`, but `src/dispatch_tracking.rs` is a completely separate 15/30-min warn/ask/orphan sweep with **no FEATURE doc of its own anywhere**.
6. **`docs/FEATURE-task-board.md`** never mentions `board_sweep`/`board_unretire`/`orphan_reconcile_{preview,apply}` — 4 live MCP actions, ~1,600 LOC.
7. **`docs/FEATURE-decisions.md`** documents only 4 of 6 live decision actions (missing `get`, `archive_batch`); its own "Source Pointers" section never cites the actual daemon-side timeout files (`daemon/decision_timeout.rs`, `daemon/decision_board_timeout.rs`).
8. **`docs/FEATURE-fleet.md:461-467`** lists 4 daemon-managed fields; `fleet/merge.rs:11` actually classifies 5 (`created_by` undocumented).
9. **`docs/FEATURE-channels.md`'s topic-deleted self-heal** documents only the recreate+retry path (`reply.rs:147-160`, live); a second function `send_reply` (`reply.rs:14`) implements delete-the-instance semantics and its own doc comment falsely claims MCP-reply-tool reachability — its only real caller anywhere is a unit test.
10. **`docs/FEATURE-communication.md`'s "Idempotent Retry" section** actually describes `api/request_dedup.rs` (UUIDv4 request_id dedup), not `inbox/idempotent.rs` (a narrower, unrelated producer-replay guard used only by the Claude self-kick watchdog).
11. **Stale internal `//!` comments (not docs/, but the exact same failure mode):** `daemon/discharge_ledger.rs:12-15` and `daemon/channel_reply_discharge.rs:1-8` both say "nothing calls this yet, PR-2's job" — both are now live-called (`inbox/storage.rs:1760`, `reply_ledger.rs:259`). `protected_refs.rs:1-9` and `integrity_core.rs:1-8` both claim `bin/agend-git.rs` still `#[path]`-includes them as single-source-of-truth; that binary is now kill-guard-only, and `vendor/agentic-git` carries independent un-synced copies of both files. `src/skills.rs`'s own header says "5 backends"; the real table has 6 (missing `grok`) — the external `docs/FEATURE-skills.md` is correct here, the internal comment is the stale one.
12. `docs/FEATURE-diagnostics.md` never mentions `doctor protocol`, `doctor providers`, `doctor orphans`, or `agend-terminal verify` at all (coverage gap, not a factual error).

---

## §5. Governance / process-machinery (not core product features — candidates to consolidate or drop in a rewrite)

| Mechanism | What it gates | Evidence |
|---|---|---|
| `claim_verifier.rs` | `git push` claim-vs-diff check; also an MCP dispatch-tree/test-name validation gate | `scripts/hooks/pre-push:104-105`; `mcp/handlers/comms_gates/dispatch.rs:251,253` |
| `review_receipt.rs` | Only the daemon can turn a `CodeReviewRequest` into a validated receipt carrying reviewer authority | `agent_ops/messaging.rs:190-197` |
| `merge_receipt.rs` | Sole task-completion settlement authority after a binding is released | `tasks/mod.rs:76-85,242` |
| `protected_refs.rs` | Blocks leasing/generic-CI-watch on `main`/`master` | `worktree_pool.rs:385`, `mcp/handlers/ci/watch.rs:17` |
| `operator_mode.rs` | Single daemon API-ingress authority gate, fail-closed to `Away` | `api/mod.rs:886` |
| `config_integrity.rs`/`integrity_core.rs` | HMAC-signs `operator-mode.json` against prompt-injected self-tampering (explicitly NOT a multi-user security boundary, per its own threat-model comment) | `operator_mode.rs:156,170` |
| `assignment_authority.rs` (daemon) | Durable, crash-safe, CAS'd reviewer-assignment ownership record | reconciled every tick, `per_tick/mod.rs:437` |
| `discharge_ledger.rs`/`channel_reply_discharge.rs` (daemon) | "Already-triaged" audit records so a poll doesn't re-notify | `inbox/storage.rs:1760`, `reply_ledger.rs:259` |
| `escalation_persist.rs` (daemon) | Persists crash-budget/notify-cooldown state across daemon restarts | keyed store `health_escalation.json` |
| `binding/signature.rs` (HMAC) | Task-completion trust decision, not just a UI diagnostic | `tasks/mod.rs:203` |
| `binding/release_guard.rs`, `rebind_guard.rs`, `review_lease.rs` | Guarded-release transaction / narrow same-agent rebind exception / disposable-review task-id repair | `binding.rs:353`; `agent_ops/messaging.rs:881` |
| `tasks/acl.rs`, `governance.rs`, `operator_settlement.rs` | Who may mutate a task record / review-authority fields / CLI-only settlement retry | `handler.rs:7`; `comms_delegate/mod.rs:172`; `api/mod.rs:896-897` |
| `mcp/handlers/comms_gates/*` | Send/dispatch authorization gates (stale-SHA block, evidence-before-verdict, structural task_id requirement, busy-park redrive) | `comms_gates/mod.rs:16-19,37-38` |
| `mcp/handlers/dispatch_hook/*`, `force_release/*` | Auto-bind+lease+watch_ci-on-dispatch hook; stale-worktree recovery backing `release_worktree(force:true)` | own module headers |
| `agent/sensitive_env.rs` | 20-key deny-list stripping credentials/LD_PRELOAD-class vectors from every spawned agent's env | `agent/mod.rs:1067,327` |
| `api/operator_gate.rs`, `request_dedup.rs`, `api_activity_probe.rs` | API-ingress authority classification; idempotent-retry dedup; out-of-path busy-probe | control-plane plumbing, not user features |
| `cleanup_intents/owner_attestation.rs` | Explicit `Delete:`-prefixed human/agent attestation to settle an unmergeable branch | `cleanup_intents.rs:340` |

**Recommendation for the rewrite**: the ledger-shaped ones (`assignment_authority`, `discharge_ledger`, `channel_reply_discharge`, `escalation_persist`) share one pattern (flock + atomic-write + keyed-JSON) — candidates to consolidate into one generic durable-ledger primitive instead of 5 hand-rolled stores.

---

## §6. Legacy / deprecated / likely-dead — with grep evidence

### Confirmed dead (zero production callers)
- **`src/daemon/dedup_state.rs`** — self-described "Legacy dedup-state directory management" (GC shim for feature #1316, removed long ago). Still called once at boot (`daemon/mod.rs:1217-1218`), so not *unreachable*, but every line exists only to sweep litter from a feature that no longer exists.
- **`src/backend_harness.rs`** — 578 raw lines, but `CapabilityLevel`/`BackendCapability`/`CapabilityMatrix`/`probe_esc_stops_generation` are all test-gated; the only two real production functions (`verify_byte_delivery`, `verify_tcgetpgrp`, ~75 lines) have **zero references anywhere in `src/`**: `grep -rn "verify_byte_delivery\|verify_tcgetpgrp" src --include="*.rs" | grep -v backend_harness.rs` → no output. `mod backend_harness;` is declared in `main.rs:22` but nothing calls into it.
- **`src/binding/unbind_compat.rs`** — self-annotated `#[allow(dead_code)]`; `grep -rn "binding::unbind\b" src/` shows every caller is a test file. Real function `unbind_with_permit` has 7 genuine production callers instead.
- **`src/tasks/acl.rs:41`** `can_mutate_task` — `#[allow(dead_code)]`, superseded by `can_mutate_record`; no non-test callers.
- **`src/deployments.rs:698`** `pub fn deploy` — `#[allow(dead_code)]`, zero callers (`grep -rn "deployments::deploy(" src` outside tests: none); the live entry point is `deploy_with_runtime`.
- **`src/channel/discord/keepalive.rs::start_keepalive`** — fully implemented (30-min anti-auto-archive PATCH loop for Discord threads), exported crate-wide, but `discord/bootstrap.rs`'s only production init path never calls it. Discord threads silently auto-archive after ~60 min despite the feature existing and being unit-tested (`discord/tests.rs:1219` exercises the underlying PATCH helper directly).
- **`src/channel/telegram/reply.rs:14` `send_reply`** — its own doc comment claims "called from MCP reply tool," but the real MCP-reply path is `adapter.rs:403`; `send_reply`'s only actual caller anywhere is a `#[cfg(test)]` unit test (`channel/contract.rs:267`).

### Test-only files miscounted as production by naive LOC scripts (not features at all)
`src/tasks/routing_red_2760.rs` (496 lines, `#[cfg(test)]`-gated in parent), `src/tasks/settlement_diagnostic_3584.rs` (687 lines, same pattern), `src/app/review_repro_app_tui.rs` + `src/app/overlay/review_repro_app_tui.rs`, `src/binding/review_repro_agent_binding.rs`, `src/worktree_pool/review_repro_worktree_git.rs`, `src/worktree_cleanup/review_repro_worktree_git.rs`, `src/channel/telegram/reply/review_repro_channel.rs`, `src/api/handlers/set_model_success_3573.rs`, `src/state/review_repro_state_capture.rs`, `src/channel/contract.rs` (ships in `src/` for a build-target reason, but zero non-test callers) — all confirmed via `grep -rn <name> src --include="*.rs"` returning only their own declaration + test callers.

### "_legacy"-named but confirmed LIVE (naming trap — do not treat as removable)
- `src/transport/legacy_pty.rs` — the actual **default** transport for Grok/KiroCli/Agy/Shell/Raw backends, plus an explicit Claude-Code opt-out (`AGEND_TRANSPORT_MODE=legacy_pty`). "Legacy" describes the raw-PTY mechanism, not lifecycle status.
- `src/worktree_pool/legacy_release.rs` — live in the release hot path (`worktree_pool.rs:1061,1110,1977-2005`); "legacy" means old on-disk directory layout, not dead code.
- `src/deployments.rs`'s `spawn_instances_legacy`/`create_deployment_team_legacy`/`delete_instances_legacy` — a genuinely live *alternate transport* for standalone MCP-bridge callers without direct daemon-registry access (`mcp/handlers/dispatch.rs:516-517`).
- `Backend::Gemini`→`Backend::Agy` rename (#1580, gemini-cli sunset) — clean historical removal, zero lingering references.

### One-off scaffolding wearing a permanent MCP action (worth flagging, not "dead" but not reusable)
- **`src/tasks/orphan_reconcile.rs`** hardcodes `DECISION_ID = "d-20260922032524703479-91"` and `BOARD_PROJECT = "Hack_agend-terminal"` — this repository's own project board. Every call is fail-closed unless it matches this frozen "seven audited predecessors" set, yet it's listed in `def_task()`'s docstring as a general-purpose action alongside `claim`/`done`.

### Confirmed still-active (checked because the name suggested otherwise)
`worktree_pool/legacy_release.rs`, `transport/legacy_pty.rs` (above); `src/state/*.rs` (confirmed a real PTY-state classifier, not a shadow/rollout-style trap — the actual "shadow/rollout" trap file, `src/daemon/shadow/rollout.rs`, does exist in this codebase, is a state-reconciliation observer not a feature-flag framework, and is inventoried under subsystem #4 above).

---

## §7. Notable single-file findings for the rewrite decision

- **`src/token_cost.rs`** (1,061 LOC, backs `agend-terminal admin tokens`) has **zero FEATURE-*.md coverage** despite being a substantial, real, two-backend (Claude+Codex) feature.
- **`src/tray/icon.rs`** is a literal 6-line placeholder — "real icons... bundled via `include_bytes!` when PLAN task #4 lands" — unfinished stub, not a bug.
- **`src/render/offthread.rs`** is fully wired and tested but gated behind `AGEND_OFFTHREAD_PARSE`, default OFF — a shipped-but-dormant optimization path.
- **`src/api/mod.rs:243` `RestartCapability::App`** — `#[allow(dead_code)]`, reserved for an owner-restart composition root that was never built.
- **`src/dispatch_tracking.rs` vs. `src/daemon/dispatch_idle/*`** — two independently-wired subsystems that share the word "dispatch" and must not be conflated in a rewrite (see §4.5).
- **`src/schedules.rs` vs. `src/schedule_jobs/*`** — confirmed one layered feature (plain reminders vs. durable daemon-owned jobs sharing one store), not duplication.
- Three transport tiers coexist by design (`transport/registry.rs:25-38`): `NativeShared` (Codex/OpenCode), `ChannelBridge` (Claude), `LegacyPty` (Grok/KiroCli/Agy/Shell/Raw + Claude opt-out) — a rewrite should decide whether 3 tiers are still justified.

---

## Source part-files (full per-agent detail, file:line citations, additional sub-cluster tables)

- `part1_daemon.md` — daemon core + bootstrap (8 sub-clusters, 145 files)
- `part2_taskboard.md` — task board + decisions (5 sub-groups, 22 files)
- `part3_tui.md` — TUI (7 sub-groups)
- `part4_agent_backend.md` — agent/PTY/backend/transport (7 sub-groups, 47 files)
- `part5_messaging_channel.md` — messaging/inbox abstraction (6 sub-groups, 33 files)
- `part6_fleet_binding_worktree.md` — fleet/binding/teams/worktree (10 sub-groups, 41 files)
- `part7_state_health_misc_features.md` — state/health/quickstart/schedules/tray/deployments/skills/api (12 groups)
- `part8_mcp_cli_plumbing.md` — MCP registry/handlers/CLI/bin binaries (10 groups)
- `part9_governance_misc.md` — governance receipts + misc utility long-tail (7 groups, 35 files)
- `part10_gapfill.md` — telegram/discord bot clients + api/handlers + tray subdirs + transport leftovers
