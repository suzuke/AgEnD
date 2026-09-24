# codex-cli 0.156.1 app-server spike — notes

> 歸檔註：設計階段的原始紀錄，內容原樣保存（2026-09-24）。只把絕對路徑換成佔位符：`<scratchpad>` = 當時的暫存目錄、`<v1-repo>` = agend-terminal repo、`<tmp>` = 系統暫存目錄、`~` = 使用者家目錄。文中提到的腳本、log、schema dump 沒有一起歸檔。

Binary: `~/.local/bin/codex` (`codex-cli 0.156.1`)
Scratch root: `<scratchpad>/spike-codex/`
All scripts/logs referenced below live under that root unless stated otherwise.

Reference read (read-only, not modified): `src/transport/codex_app_server.rs`,
`src/transport/registry.rs`, `src/agent/mod.rs` (grep for `codex`).

## Setup

- `codex_home_a/`: isolated `CODEX_HOME` with a copy of `~/.codex/auth.json` (removed
  again at cleanup) and a `config.toml` with `model_reasoning_effort = "low"`,
  `approval_policy = "on-request"`, `sandbox_mode = "workspace-write"`, and
  `[projects."<workspace_a>"] trust_level = "trusted"`. Used for S1-S5, S7.
- `codex_home_fresh/`: isolated `CODEX_HOME`, auth.json only, no config.toml — used
  for S6 (untrusted-directory prompts).
- `workspace_a/`, `workspace_fresh/`: empty cwds under scratch.
- `codexws.py`: hand-rolled WebSocket-over-AF_UNIX JSON-RPC client (the real wire
  protocol per `codex_app_server.rs`: HTTP/1.1 Upgrade handshake, then masked
  text frames carrying `{id,method,params}` / `{method,params}` — no `jsonrpc:"2.0"`
  marker).
- `launch_detached.py`: double-fork + `os.setsid()` daemonizer (macOS has no
  `setsid(1)` binary) — used to start `codex app-server` fully detached from the
  invoking shell (ppid=1, own session/process group, stdio to a log file).
- Protocol discovery used `codex app-server generate-json-schema --experimental -o schema/`
  (official, not guesswork) — `schema/_client_request_methods.txt` lists all 164
  client→server methods; `ServerRequest.json`/`ServerNotification.json` list the
  11 server→client requests and 82 notifications.

## CRITICAL non-obvious finding: AF_UNIX path length vs `--listen`/`--remote`

`codex app-server --listen unix://<path>` does **not** bind a socket at the
literal path you give it if that path is long. It binds the real socket at a
short path `/private/tmp/codex-daemon-<uid>/<sha256-of-the-listen-path>` and
leaves a **filesystem symlink** at your requested path pointing there (a
`symlink()` call has no length limit; `bind()`/`connect()` on `AF_UNIX` do —
~104 bytes on macOS, `sockaddr_un.sun_path`).

Consequence, verified two independent ways:
- Our own Python client got `OSError: AF_UNIX path too long` connecting to the
  literal (long) `--listen` path.
- `codex --remote unix://<same long path>` itself refused with
  `Error: failed to connect to remote app server ...: path must be shorter than SUN_LEN`.

So **any** client (including codex's own TUI/`resume`) must connect to the
*resolved* short path, not the path you asked the server to listen on, once
that path exceeds the AF_UNIX limit. Fix applied in `codexws.py`: `connect()`
resolves `os.path.realpath()` first. Design implication below.

`codex app-server daemon` is a real, documented subsystem (`codex resume --help`
has `--no-daemon`: "Run without the shared background server, even if it is
already running"). Each distinct `--listen` path got its own dedicated process
in our tests (different listen-path strings → different hash → different PID);
we did not find evidence of unrelated app-server instances being silently
shared across different `--listen` values.

## S1 — Reconnect

**Verdict: WORKS.**

- Started detached (`launch_detached.py`, ppid=1, own pgid) app-server on
  `codex_home_a`, `--listen unix://.../s1.sock`.
- Client A: `initialize` → `thread/start` → `turn/start` (`sleep 3 && echo hi;
  reply done`). Captured `turn/started`, then **closed the socket** without
  waiting for completion (`s1_test.py`).
- App-server process stayed alive after the disconnect (confirmed via `ps`).
- Client B (fresh connection, ~6s later): `thread/resume {threadId,
  excludeTurns:true}` succeeded and returned full thread metadata.
  `thread/resume` itself does **not** include turn history (that's what
  `excludeTurns` means).
- The missed turn's outcome (it failed — see model-config note below, not a
  reconnect problem) was fully recoverable via `thread/turns/list
  {threadId}`: full item list, `status`, `error`, `startedAt`/`completedAt`/
  `durationMs` — i.e. **the app-server persists turn completion even with zero
  connected clients**, and a reconnecting client can retrieve it.
- Client B then started a brand-new turn on the resumed thread and received
  the complete live event stream (`turn/started` → `item/started` →
  `item/completed` → `turn/completed`) — confirms **future** notifications
  flow to a reconnecting client, not just history.
- (First run used `model_reasoning_effort="minimal"`, which the configured
  model rejects — HTTP 400 `unsupported_value`. Fixed to `"low"` for all later
  tests. This is a config mistake, not a protocol finding.)

## S2 — TUI-originated turns

**Verdict: WORKS, but only after `thread/resume`.**

- App-server `s2.sock`; TUI attached via `codex --remote unix://<resolved short path>`
  in a private tmux server (`tmux -L spike-codex`).
- **tmux gotcha**: `tmux send-keys -t tui "text" Enter` in one call typed the
  text but did **not** submit it — Codex's composer needs the Enter as a
  *separate* `send-keys` call after a short delay (~0.3-0.5s), otherwise it's
  racing the TUI's own input-processing and the Enter is swallowed as a
  newline into the still-being-typed line. This is exactly the kind of PTY-
  timing fragility the structured-API design is meant to avoid.
- A **passively connected** observer (initialize + `thread/loaded/list` only,
  no `thread/resume`) received only coarse `thread/status/changed`
  (active/idle) notifications for the TUI's thread — **not** `turn/started`/
  `item/*`/`turn/completed`.
- After the observer called `thread/resume {threadId, excludeTurns:true}` on
  the TUI-discovered thread id, a subsequent TUI-typed prompt produced the
  **full** granular event stream on the observer connection: `turn/started`,
  `item/started` (userMessage), `item/completed`, `item/started`
  (agentMessage), streaming `item/agentMessage/delta` tokens, `item/completed`
  (final text), `thread/tokenUsage/updated`, `thread/status/changed`,
  `turn/completed`.
- **Design implication**: a client must call `thread/resume` on a
  TUI-discovered thread id (exactly what v1's `discover_loaded_tui_thread` +
  the `had_thread_id` branch already do) to get full turn-level visibility;
  merely being connected is not enough.

## S3 — Busy levels

**Verdict: WORKS for all three; exact method names below.**

All three tested directly against threads created by our own client (no TUI
needed) on the `s2.sock` app-server (`s3_test.py`, `s3_queue_retest.py`).

1. **Steer** (`turn/steer`, params `{threadId, expectedTurnId, input}`):
   sent mid-turn while a `sleep 4 && echo ...` command was running. Ack:
   `{"result":{"turnId":...}}`. The model's **final reply for that same turn**
   incorporated the steered instruction (`"FIRST STEERED"`) — confirmed
   in-turn visibility.
2. **Plain `turn/start` called again while busy** (no steer, no queue):
   the server did **not** error and did **not** start a second concurrent
   turn. It silently merged the new user message as another item into the
   *already-active* turn (the ack echoed the same, already-in-flight turn id)
   — the model's final answer for that turn changed from what the first
   prompt asked to what the second one asked. I.e. **the server treats a bare
   `turn/start` issued while busy as an implicit steer/coalesce**, not an
   error and not a queue. This means even an imperfect client-side busy
   tracker cannot corrupt state by racing a `turn/start`.
3. **Queue** (`thread/queue/add`, params `{threadId, clientUserMessageId,
   input}`): added while a turn was running; `thread/queue/list {threadId}`
   showed it pending. Once the running turn's `turn/completed` fired, the
   server **automatically** started a new turn for the queued message
   (`turn/started` with a new turn id) with no explicit call needed —
   confirmed end-to-end with distinct "ALPHA" (turn 1) → "BETA" (auto-started
   turn 2) replies (`s3_queue_retest.py`). Calling `thread/queue/start`
   explicitly while auto-dequeue is already racing to fire returns
   `{"error":{"code":-32600,"message":"thread already has an active or
   pending turn"}}` — **do not call `thread/queue/start` manually after
   `thread/queue/add`; just wait for the auto-dequeue.**
4. **Interrupt then fresh turn** (`turn/interrupt`, params `{threadId,
   turnId}`): sent 1s into an 8-second `sleep`. Ack: `{}`. The turn's
   `turn/completed` reported `status: "interrupted"` and the `sleep 8`
   command never finished. A brand-new `turn/start` on the same thread
   immediately afterward ran and completed normally.

## S4 — Resume by explicit id after killing everything

**Verdict: WORKS.**

- Seeded a fact into a TUI-created thread (`s2.sock` / tmux `tui`): "remember
  the secret number 42" → "REMEMBERED". Thread id
  `01a0d1fc-d191-7013-bd2e-d5c32d2b97b3`.
- Killed the TUI tmux session AND the app-server (`kill <pid>`; socket
  self-removed).
- Started a **brand-new** app-server (`s4.sock`, new PID) on the **same**
  `CODEX_HOME`.
- `codex resume <thread-id> --remote unix://<resolved short path>` (explicit
  id, not `--last`) reattached and displayed the **entire prior transcript**
  (all earlier TUI turns) even though nothing was in memory anymore — proves
  it's reading the durable on-disk rollout (`codex_home_a/sessions/.../*.jsonl`),
  not just in-process state.
- Asked "what secret number did I ask you to remember?" → replied **"42"** —
  genuine conversational continuity, not just transcript display.

## S5 — Sandbox + unix socket (network egress via a local socket)

**Verdict: WORKS as designed — the default sandbox BLOCKS it, and answers S7
in the same run.**

- Listener: a short-path (`<tmp>/spike-s5-listener.sock` — the scratch
  path itself was too long for `AF_UNIX bind()`, same limit as above; the
  listener target is still clearly outside `workspace_a`) Python
  `socket.AF_UNIX` server.
- Asked the agent (default `sandbox_mode="workspace-write"`, no bypass flag,
  `approval_policy="on-request"`) to run, verbatim:
  `python3 -c "import socket; s=socket.socket(...); s.connect('<tmp>/spike-s5-listener.sock'); ..."`.
- **First attempt** (inside the sandbox) failed:
  `PermissionError: [Errno 1] Operation not permitted` — the sandbox blocked
  the connect() syscall to a path outside the workspace.
- The app-server then sent a genuine structured approval request (see S7)
  asking to rerun outside the sandbox; our client approved it
  (`{"decision":"accept"}`); the **rerun outside the sandbox succeeded**
  (`CONNECT_OK b'hello-from-listener'`, and the listener's log independently
  confirms `GOT: b'ping-from-agent'`).
- **Conclusion for the `agend` CLI design**: a holder process's local IPC
  socket (e.g. the daemon's control socket) sitting outside the sandboxed
  workspace root will be unreachable from inside a default-sandboxed codex
  turn without an explicit per-command approval grant. Either (a) put the
  socket/endpoint under a writable/workspace root the agent already has, or
  (b) expect/handle an approval round-trip for any command that needs it, or
  (c) use `sandbox_mode="danger-full-access"`/bypass only when the daemon
  already trusts the workspace fully (as v1's `launch_managed_server` already
  does: `approval_policy="never"`, `sandbox_mode="danger-full-access"`).

## S6 — Startup prompts in a never-trusted directory

**Verdict: WORKS (trust prompt observed and reliably avoided via pre-seeded config).**

- Fresh `codex_home_fresh` (auth.json only, no config.toml) + never-used
  `workspace_fresh`, plain `codex` (no `--remote`) in a private tmux pane.
- Startup showed exactly one blocking prompt:
  ```
  Folder access
  <path>
  Trust this folder? Codex can read, edit, and run files here, subject to
  your permission settings. ... Your trust decision will be saved.
  › 1. Trust and continue
    2. Quit
  ```
- Chose "2. Quit" (declined) — process exited cleanly (`EXITCODE=0`); no
  trust entry was written, but codex still bootstrapped the full
  `CODEX_HOME` skeleton (sqlite dbs, `installation_id`, `models_cache.json`,
  `version.json`, `plugins/`, `skills/`, and a `config.toml` containing only
  `[tui] screen_reader_detection_done = true`).
- Edited **only the isolated** `codex_home_fresh/config.toml` (never touched
  `~/.codex/config.toml`) to add:
  ```toml
  [projects."<workspace_fresh path>"]
  trust_level = "trusted"
  ```
  (exact format confirmed by reading, read-only, the real `~/.codex/config.toml`,
  which already has dozens of such entries).
- Relaunching `codex` against the same `CODEX_HOME`/workspace went straight to
  the normal composer — **no trust prompt**. No other blocking prompt
  (update nag, onboarding wizard, telemetry opt-in) appeared in either run;
  only non-blocking informational banners ("Tip: ...", a weekly-rate-limit
  warning). The real `~/.codex/config.toml` also has a working `[notice]
  hide_*_prompt = true` / `[notice.model_migrations]` mechanism for
  suppressing other one-time nudges, confirming config-based suppression is
  the general pattern here, not something special-cased for trust alone.
- No fresh login was required (auth.json copy was sufficient), so nothing
  here is BLOCKED.

## S7 — Structured approvals

**Verdict: WORKS.** (Captured live as part of the S5 run — no bypass flag,
`approval_policy="on-request"`.)

- Method: **`item/commandExecution/requestApproval`** — a genuine
  server→client JSON-RPC *request* (has an `id`, server assigns its own id
  space starting at `0` in our run), not a notification.
- Payload observed:
  ```json
  {
    "id": 0,
    "method": "item/commandExecution/requestApproval",
    "params": {
      "kind": "command",
      "threadId": "...", "turnId": "...", "itemId": "exec-...",
      "startedAtMs": 1790229991393,
      "environmentId": "local",
      "reason": "The sandbox blocked the socket connection. May I rerun your exact command outside the sandbox?",
      "command": "/bin/zsh -lc \"python3 -c ...\""
    }
  }
  ```
- Client answers with a JSON-RPC **response** on the same `id`:
  `{"id": 0, "result": {"decision": "accept"}}`. Full decision enum (from
  `schema/CommandExecutionRequestApprovalResponse.json`,
  `CommandExecutionApprovalDecision`): `accept`, `acceptForSession`,
  `acceptWithExecpolicyAmendment` (+ execpolicy_amendment list),
  `applyNetworkPolicyAmendment`, `decline`, `cancel`.
- There is also a v1/back-compat pair, `execCommandApproval` /
  `ExecCommandApprovalParams`/`Response` (`ReviewDecision` enum: `approved`,
  `approved_execpolicy_amendment`, presumably `denied`/`abort`) in the schema
  dump, but we only ever observed the `item/commandExecution/requestApproval`
  form fire in this v2-protocol server. `item/fileChange/requestApproval` and
  `item/permissions/requestApproval` exist in the schema for the analogous
  file-write / permission-escalation cases but were not exercised here.
- Our client auto-approved and the rerun succeeded — confirms the full
  request→approve→execute round trip works over the structured channel with
  no PTY involvement.

## Cleanup performed

- Killed `codex app-server` PIDs 32931 (S1), 41214 (S2), 53228 (S4) — all via
  plain `kill`; each removed its own socket symlink on clean exit.
- Killed tmux sessions `tui`, `tui4`, `fresh1`, `fresh2`, then
  `tmux -L spike-codex kill-server` (private tmux server on its own socket,
  never touched the default tmux server).
- Removed the `.lock` marker files codex leaves behind under
  `/private/tmp/codex-daemon-<uid>/` for the three hashes we created
  (`2f544f85...`, `7fb3e11a...`, `4ba937bb...`); left all pre-existing,
  unrelated `.lock` files in that shared directory untouched.
- Killed the S5 Python unix-socket listener (it self-exits after one
  connection) and removed `<tmp>/spike-s5-listener.sock`.
- Deleted the copied `auth.json` credentials from both scratch `CODEX_HOME`
  dirs (`codex_home_a/auth.json`, `codex_home_fresh/auth.json`) since they
  were no longer needed.
- Never touched `~/.agend-terminal`, the real daemon, the default tmux
  server, `~/.codex/config.toml`, or any repo file.

## Design implications for the v2 codex driver / holder

1. **Socket path length is a real, load-bearing constraint.** Any locator
   endpoint path the daemon hands to a holder (or that a holder passes to
   `codex app-server --listen` / `--remote`) must either (a) stay under the
   AF_UNIX limit (~100 bytes) end-to-end, or (b) the holder must always
   `readlink`/`realpath` the endpoint before connecting and use the resolved
   short path — exactly like `codex`'s own TUI needs. `AGEND_HOME` paths
   nested under long project-scoped scratch directories (as in this very spike)
   are exactly the shape that breaks this if not handled.
2. **The holder does not need to track busy-state precisely to stay safe.**
   A bare `turn/start` issued while a turn is already running is coalesced by
   the server into the active turn (not an error, not a fork) — so a naive
   "queue" fallback that just calls `turn/start` when it isn't sure is at
   worst a steer, never data corruption. For true "run after current turn
   ends, as its own turn" semantics, use `thread/queue/add` and just wait for
   the automatic dequeue — do not also call `thread/queue/start`.
3. **A reconnecting holder must call `thread/resume` explicitly**, even when
   it already knows the thread id from a TUI-loaded discovery — passive
   connection only yields coarse `thread/status/changed`, not the granular
   `turn/*`/`item/*` stream a holder needs to relay to the daemon.
4. **Turn completion during a client-less window is not lost** — recoverable
   via `thread/turns/list`/`thread/items/list` — so a holder can safely miss
   events while it's reconnecting and backfill afterward.
5. **Approvals need a real handler, not just a bypass toggle.** Under any
   sandbox short of `danger-full-access`, ordinary commands the daemon needs
   (e.g. talking to its own control socket outside the workspace) can trigger
   `item/commandExecution/requestApproval` mid-turn; the holder's structured
   channel must be able to answer it (or the daemon must keep the sandbox
   permissive for its own managed agents, as v1 already does via
   `approval_policy="never"` + `sandbox_mode="danger-full-access"` for
   managed servers).
6. **Never drive the TUI via literal `send-keys "text" Enter` in one call** —
   confirmed race where Enter is swallowed. If a holder ever needs a PTY
   fallback path, separate the Enter into its own delayed call. (This is one
   more argument for why the structured-API-only design in the spec is
   correct: PTY typing is measurably racy even for a human-shaped case like
   this.)
7. Pre-seeding `[projects."<abs-path>"] trust_level = "trusted"` in an
   isolated `CODEX_HOME`'s `config.toml` reliably skips the trust prompt for
   headless/managed agents, with no other blocking prompt encountered at
   0.156.1.
