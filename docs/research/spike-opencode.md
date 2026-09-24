# opencode 1.18.31 spike — AgEnD v2 driver/holder design facts

> 歸檔註：設計階段的原始紀錄，內容原樣保存（2026-09-24）。只把絕對路徑換成佔位符：`<scratchpad>` = 當時的暫存目錄、`<v1-repo>` = agend-terminal repo、`<tmp>` = 系統暫存目錄、`~` = 使用者家目錄。文中提到的腳本、log、schema dump 沒有一起歸檔。

Binary: `/opt/homebrew/bin/opencode` (`opencode --version` -> `1.18.31`)
Scratch root: `<scratchpad>/spike-opencode/`
v1 code read (read-only): `src/transport/opencode_server.rs` (2309 lines), `src/transport/opencode_server/http.rs` (341 lines).

## v1 baseline (from reading the code, before any live test)

- HTTP+SSE only, raw hand-rolled HTTP/1.1 client (`http.rs`), no PTY fallback for opencode.
- `launch_server`: spawns `opencode serve --hostname 127.0.0.1 --port <p>` in its own process
  group (`command.process_group(0)` on unix), `XDG_DATA_HOME` pointed at a per-instance data dir
  that gets `auth.json` copied in from the canonical `~/.local/share/opencode/auth.json`.
  `OPENCODE_DISABLE_AUTOUPDATE=1` + `OPENCODE_CONFIG_CONTENT={"autoupdate":false}` are set on the
  child env.
- Attach args for the TUI: `opencode attach <endpoint> --session <session_id>` (`attach_args`,
  opencode_server.rs:915-931).
- SSE: `GET /event`, decoded by a hand-rolled chunked/SSE decoder (`SseDecoder`/`SseStream`).
  `locator.event_cursor` is persisted but is only a local monotonic counter for bookkeeping — it is
  **never** sent back to the server as a resume point (opencode_server.rs:2103-2107; confirmed live,
  see O1). Reconnection after a drop is a bare fresh `GET /event`.
  Recovery of missed state is done by re-querying REST (`GET /session/:id`,
  `GET /session/:id/message`), not by SSE replay.
- Busy handling is entirely client-side in v1: `deliver_blocking` checks `self.in_flight` and if
  set, **parks** the new delivery in a FIFO queue (`ParkedDelivery`) instead of calling
  `prompt_async`, re-driving it only when a `session.idle`/`session.status:idle` event fires
  (`complete()` -> `redrive_parked()`). Max 3 attempts, then fails closed
  (`MAX_PARKED_REDRIVE_ATTEMPTS`).
- `DeliveryKind::Steer | DeliveryKind::Interrupt` are hard-failed in `deliver_blocking`:
  `"OpenCode has no implicit steer/interrupt operation"` (opencode_server.rs:981-988) — v1 never
  even tries an insert/steer call.
- `normalize_event` recognizes `server.connected`, `message.updated`/`message.part.updated`,
  `session.status`, `session.idle`, `session.error`, `permission.asked` -> `BackendEvent::*`.
- `prompt_body` sends `"model": "<flat string>"` (opencode_server.rs:1116-1124) — **this does not
  match the live OpenAPI schema**, which wants `model: {providerID, modelID}` (confirmed via
  `/doc`). Not one of O1-O6, flagging as a v1 defect / driver-design input, not verified further
  (no delivery-path test was run against v1's own code, only against the raw HTTP API).

## Free model selection

```
$ opencode models   # with XDG_DATA_HOME/XDG_CONFIG_HOME pointed at scratch, auth.json copied in
opencode/big-pickle
opencode/ling-3.0-flash-fin-free
opencode/mimo-v2.6-flash-free
...
opencode/space-bunny-free
opencode-go/...      <- paid (opencode-go/deepseek-v4.1-flash verbose: cost.input=0.15, NOT free)
openrouter/...        <- paid passthrough
```

`opencode models opencode --verbose` shows every model under the **`opencode` provider** (OpenCode
Zen) has `"cost": {"input": 0, "output": 0, "cache": {"read": 0, "write": 0}}`. Chose
**`opencode/space-bunny-free`** (`providerID: opencode`, `id: space-bunny-free`, 1M context) —
explicitly labelled "Free" in both `opencode models` and the TUI's model picker ("Space Bunny Free
· OpenCode Zen — Free"). Sanity check: `opencode run "reply: pong" -m opencode/space-bunny-free`
-> `pong`. All subsequent turns in this spike used this model. `opencode-go/*` (default TUI model,
"DeepSeek V4.1 Flash OpenCode Go") is paid and was switched away from before ever sending a prompt
through it (TUI: ctrl+p -> "Switch model" -> search "Space Bunny" -> select the OpenCode Zen one).

## O1 — Reconnect

Setup: isolated `opencode serve --hostname 127.0.0.1 --port 39271` (own process group via `nohup … &
disown`), `XDG_DATA_HOME`/`XDG_CONFIG_HOME` pointed at scratch, auth.json copied in (never touched
`~/.local/share/opencode`).

1. `POST /session` -> `ses_f2e07aba4ffeLmLgCA56VxqhPO`.
2. SSE client #1 (`curl -sS -N /event`) connected, then `POST /session/:id/prompt_async` with a
   150-word writing prompt.
3. Saw `session.status busy` and `session.updated` over SSE, then **killed the curl SSE client
   mid-turn** (~2s after busy).
4. `ps -p <serve-pid>` still alive; `serve.log` had no crash/error lines.
5. Waited 8s with **zero** SSE listeners attached, then `GET /session/:id` and
   `GET /session/:id/message`: the turn had fully completed
   (`tokens.output: 186`, assistant message present with full 150-word text, `time.completed` set).
   Confirms the turn runs to completion server-side independent of any client connection.
6. Opened a brand-new SSE connection (`curl -N /event`): the **only** event received was a fresh
   `server.connected` — none of the `session.status`/`message.*`/`session.idle` events from the
   turn that ran while disconnected were replayed.
7. `GET /session/status` while idle returned `{}` (a map keyed by session id, populated only for
   busy sessions — matches v1's `session_status_type` lookup code).

**Verdict: WORKS.** `opencode serve` is fully decoupled from any SSE client; a turn started before
a disconnect runs to completion unattended. Missed SSE events during a disconnect window are
**not** replayed (confirmed empirically, matching the task's expectation) — reconnection is a bare
`GET /event` with no cursor/offset parameter available in the API. `GET /session/:id` +
`GET /session/:id/message` (+ `GET /session/status` for the busy/idle map) are sufficient to
reconstruct current state and the full text of a turn that completed unattended.

## O2 — TUI + server

Attached the real `opencode` TUI to the same running server, in a private tmux socket
(`tmux -L spike-opencode`), exactly as v1 does: `opencode attach http://127.0.0.1:39271 --session
<id>` (created the session up front with `model:{id:"space-bunny-free",providerID:"opencode"}` so
the TUI would not default to the paid `opencode-go` model — attach has no `--model` flag).

- Switched model inside the TUI via ctrl+p command palette (see above) to avoid any paid spend.
- With an external `curl -N /event` client already connected, typed a prompt directly into the TUI
  input box and pressed Enter.
- The external SSE client received the **entire** turn in real time: `session.updated`,
  `message.updated` (user msg), `message.part.updated` (the user text part),
  `session.status:busy`, the assistant message's `step-start`/`reasoning`/`text` parts streamed via
  `message.part.delta`, `step-finish`, `session.status:busy` again, then `session.status:idle` +
  `session.idle`. Full raw event stream captured in `sse_o2.log`.

**Verdict: WORKS.** The server-wide `/event` SSE stream is a true broadcast: an external headless
client sees every event of a turn submitted through the attached TUI, including token-level
streaming deltas, with no special subscription needed.

## O3 — Busy levels

Endpoints confirmed from `GET /doc` (OpenAPI): `POST /session/:id/prompt_async`,
`POST /session/:id/abort`, `POST /session/:id/permissions/:permissionID`. **No steer/insert
endpoint exists anywhere in the OpenAPI surface** (grepped the full `/doc` path list) — matches
v1's own conclusion baked into the code.

1. **Queue**: submitted prompt A (long-form writing prompt) to a session, then — while
   `GET /session/status` showed `{"<id>":{"type":"busy"}}` — `POST /session/:id/prompt_async` again
   with prompt B. Both calls returned **`HTTP 204`** (accepted), no rejection at the HTTP layer.
   `GET /session/:id/message` after both turns settled showed **6 messages**: A's user+assistant
   pair completed in full, *then* B's user+assistant pair ran and completed — the server itself
   FIFO-queues a second `prompt_async` call made while busy and runs it after the first turn
   finishes. (v1 does not rely on this — it parks client-side and never actually calls
   `prompt_async` while `in_flight`, so this native server queuing was previously unobserved/unused
   in v1's design.)
2. **Interrupt-then-send**: started a new long turn, then 1s later
   `POST /session/:id/abort {}` -> **`HTTP 200`, body `true`**. `GET /session/status` right after
   returned `{}` (idle immediately). Immediately (no wait) `POST /session/:id/prompt_async` with a
   new short prompt -> `HTTP 204`, and it completed normally (`"post-abort-ok"`). The SSE stream
   showed the aborted assistant message get a `message.updated` event with
   `"error":{"name":"MessageAbortedError","data":{"message":"Aborted"}}` — a clean, structured
   abort signal distinct from `session.error`.
3. **Steer** (insert into the running turn without aborting): **no such endpoint exists**. Confirmed
   both from the full OpenAPI path list and from behavior — the only way to change course mid-turn
   is abort-then-resend (tested above) or let the native queue run the second message after (tested
   above).

**Verdict:**
- Queue: **WORKS** (native server-side FIFO queuing of `prompt_async`, no client parking needed).
- Interrupt-then-send: **WORKS** cleanly (`abort` -> immediately idle -> immediately accepts a new
  prompt; aborted message gets a structured `MessageAbortedError`).
- Steer (insert into a running turn): **DOESN'T EXIST** — no endpoint, confirmed by full route
  listing and by v1's own code comment.

## O4 — Resume by explicit id

1. Recorded session id `ses_f2e067657ffe80ngn3iONgtUqB` (already had 8 prior messages from O2/O3
   testing, including the word "tuiping" established several turns earlier).
2. `tmux kill-session` (kills the TUI) then `kill <serve-pid>` (hard kill of `opencode serve`).
   Confirmed dead: `curl /global/health` -> `Failed to connect`.
3. Restarted `opencode serve` fresh, same `--port`, same `XDG_DATA_HOME` (so it opens the same
   `opencode.db`).
4. `GET /session/:id` and `GET /session/:id/message` for the *exact same session id*: full 10-message
   history intact, `tokens`/`time.created` preserved.
5. Sent one more prompt: *"Earlier in this conversation I asked you to reply with exactly one word.
   What was that word? Answer with just the word."* -> model answered **`tuiping`** — proof the
   model genuinely has the prior context, not just that rows exist in a database.

**Verdict: WORKS.** Session state (and LLM-visible context) survives a hard kill + restart of
`opencode serve` as long as the same data directory (`XDG_DATA_HOME`) is reused and the session is
addressed by its explicit id. No `--continue`/`--session` flag needed on `serve` itself — plain REST
`GET`/`POST` against the known session id is sufficient; `opencode attach --session <id>` would work
the same way for a TUI.

## O5 — Permission requests ("ask")

Project-local config only (`$SCRATCH/project/opencode.json`, **not** the owner's global config):
```json
{ "$schema": "https://opencode.ai/config.json", "permission": { "bash": "ask" } }
```
Verified it loaded via `GET /config?directory=<project>` -> `"permission": {"bash": "ask"}`
(`GET /global/config` does **not** show project-scoped overrides — must query `/config` with the
matching `directory` param).

1. Created a session scoped to that directory, prompted: *"Use your bash/shell tool to run: echo
   hello-from-shell."* The tool call entered `state.status = "running"` and **stayed there** — it
   does not auto-resolve; execution is genuinely gated.
2. `GET /permission?directory=<project>` (a *poll* endpoint, separate from SSE) returned the
   pending request: `{"id":"per_...", "sessionID":"...", "permission":"bash",
   "patterns":["echo hello-from-shell"], "tool":{"messageID":"...","callID":"..."}}`.
3. First trial: the connected `/event` (and `/global/event`) SSE listener did **not** show a
   `permission.asked` event in ~80s of real-time streaming, even though the request was
   demonstrably pending via the poll endpoint. Second trial (fresh session, same setup, different
   shell command): the SSE listener **did** receive
   `{"type":"permission.asked","properties":{"id":"per_...","sessionID":"...","permission":"bash","patterns":["date"],...,"tool":{"messageID":"...","callID":"..."}}}`
   in real time, matching the OpenAPI `EventPermissionAsked` schema exactly.
4. Answered via REST: `POST /session/:id/permissions/:permissionID {"response":"once"}` ->
   `HTTP 200`, body `true`. The tool call's state flipped to `"completed"` with real output
   (`"hello-from-shell\n"` / date output) within ~2s, and `GET /permission` went back to `[]`.

**Verdict: WORKS, with one caveat.** The "ask" permission model genuinely blocks tool execution
until answered, the pending request is fully structured and inspectable via
`GET /permission?directory=...`, and it's answerable purely over REST
(`POST /session/:id/permissions/:permissionID`) with no TUI needed. **But the `permission.asked` SSE
event was missed once out of two trials** by a real-time-connected listener — cause not isolated
(could be a genuine emission gap, a `directory`/`workspace` scoping edge case, or an artifact of
`curl -N` buffering under this harness). Given O1 already establishes SSE has no replay, the
practical implication is the same either way: **a driver must not treat SSE as the sole channel for
permission requests** — poll `GET /permission` (cheap, authoritative, no session/workspace
ambiguity observed) as the source of truth or as a fallback when a turn is `busy` longer than
expected and no `permission.asked` arrived.

Also noted in `/doc`: a parallel, more elaborate "V2" permission surface exists
(`/api/session/:id/permission`, `permission.v2.asked`, `PermissionV2*` schemas) alongside the one
v1 uses (`/session/:id/permissions/:id`, `permission.asked`). Not exercised — v1's chosen surface
works end-to-end as shown above, so there's no forcing reason to move to V2, but it exists and
should be re-checked if the legacy surface is ever deprecated upstream.

## O6 — Startup prompts in a fresh directory

Genuinely fresh isolated environment: fresh `XDG_DATA_HOME` (only `auth.json` copied in, no
`opencode.db`, no prior state) + fresh empty `XDG_CONFIG_HOME`, fresh never-before-opened project
directory. Ran plain `opencode` (default TUI, not `serve`+`attach`) via tmux private socket.

- The TUI came up directly at the main input screen — **no trust dialog, no confirmation prompt, no
  "checking for updates" banner, nothing to dismiss**, even with zero prior state in that directory
  or that data dir.
- Confirmed there is no "trust this folder" concept in opencode at all: `opencode --help` has no
  `--trust`-type flag, and the `Config` OpenAPI schema has no trust-related key — the only
  startup-relevant config key is `autoupdate` (used by v1 via `OPENCODE_CONFIG_CONTENT=
  {"autoupdate":false}` / `OPENCODE_DISABLE_AUTOUPDATE=1`, though this spike found no update prompt
  to suppress in the first place).
- Isolation worked with **zero re-login**: copying the real `~/.local/share/opencode/auth.json`
  into the scratch `XDG_DATA_HOME` was sufficient; the owner's global config/auth files were never
  written to.

**Verdict: WORKS.** No startup prompts exist to avoid in the first place; project-local isolation via
`XDG_DATA_HOME`/`XDG_CONFIG_HOME` plus a copied `auth.json` is sufficient and needs no re-login, no
global config edits.

## Cleanup performed

- `tmux -L spike-opencode kill-server` (private socket only; default tmux server was never touched
  — confirmed not even running) + removed the leftover socket file
  `<tmp>/tmux-<uid>/spike-opencode`.
- Killed both `opencode serve` instances started on `127.0.0.1:39271` (`kill <pid>` then a
  `pkill -f` sweep); confirmed port refuses connections afterward.
- No stray `opencode`/`curl -N` processes remained (`ps aux` swept clean).
- Removed the two copied `auth.json` credential files from the scratch dirs after use (hygiene —
  not required by the task, but they hold live tokens copied from the owner's real account).
- Nothing outside `.../scratchpad/spike-opencode/` was created or modified; `~/.agend-terminal`,
  the real daemon, the default tmux server, and the owner's `~/.config/opencode` /
  `~/.local/share/opencode` were never written to (only read from, to copy `auth.json`).
