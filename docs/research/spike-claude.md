# Claude Code 2.1.281 driver spike — full notes

> 歸檔註：設計階段的原始紀錄，內容原樣保存（2026-09-24）。只把絕對路徑換成佔位符：`<scratchpad>` = 當時的暫存目錄、`<v1-repo>` = agend-terminal repo、`<tmp>` = 系統暫存目錄、`~` = 使用者家目錄。文中提到的腳本、log、schema dump 沒有一起歸檔。

Binary under test: `~/.local/bin/claude` -> `~/.local/share/claude/versions/2.1.281`
tmux private socket: `-L spike-claude` (killed at end, confirmed `no server running`).
Scratch root: `<scratchpad>/spike-claude/`
(One scope slip: the minimal channel server's control socket, its log, the hook log, and a
stop-hook marker were created directly under `<tmp>/spike-*` instead of inside the scratch dir.
All four were deleted at cleanup time — see Cleanup section.)

Reference reading (v1, read-only): `src/transport/claude_channel.rs` (full file, MCP stdio
channel bridge: HTTP webhook -> `notifications/claude/channel` JSON-RPC notification, content
wrapped as `<channel source="..." chat_id="..." delivery_id="..." sender_id="...">…</channel>`),
`src/backend.rs:400-920` (Claude preset: `--dangerously-skip-permissions`, resume via
`--continue`/`ContinueInCwd`, dismiss patterns for the trust/dev-channel prompts,
`spawn_flags()` which adds `--mcp-config mcp-config.json` and, only when
`mcpServers.agend-claude-channel` exists, `--dangerously-load-development-channels
server:agend-claude-channel`), `src/mcp_config.rs:270-350` (`upsert_state_hooks`: v1 already
wires SessionStart/UserPromptSubmit/PreToolUse/PostToolUse/Stop/Notification/
PermissionRequest/StopFailure/PreCompact/SessionEnd to `agend-terminal hook-event --instance
<name>`, async, timeout 10s, always exit 0 — observe-only).

Skill loaded: `claude-code-channels-custom-mcp` (gave the exact `.mcp.json` + `--dangerously-
load-development-channels server:<name>` recipe, saved a lot of trial and error).

## Setup built

- `project/channel_server.py`: minimal stdio MCP server. Answers `initialize` with
  `capabilities.experimental["claude/channel"]`, replies to `tools/list`/`ping`, and opens a
  Unix control socket (`CHANNEL_SOCK`) so a separate script can push a
  `notifications/claude/channel` JSON-RPC notification into the running Claude process's
  stdin at any time — mirrors v1's HTTP-webhook-to-stdio bridge exactly, minus the durable
  receipt log.
- `send_msg.py`: connects to that control socket, sends `{"content", "chat_id"}`.
- `project/hook.sh`: appends one JSON line (`{event, time, payload}`) per firing to
  `$HOOK_LOG`, always exits 0.
- `project/stop_hook.sh`: same, but on the Stop event's first firing (marker-file gated)
  prints `{"decision":"block","reason":"..."}`; on later firings just logs and exits 0.
- `project/.mcp.json`: registers `spike-channel` (python3 channel_server.py).
- `project/.claude/settings.json`: hooks for all 9 events named in v1's `upsert_state_hooks`
  minus SessionEnd/PreCompact detail, plus `permissions.allow: ["Bash(echo *)"]`.

## C1 — Channel delivery while busy

**WORKS (idle) / PARTIAL (busy).**

Idle: `send_msg.py "reply ok"` while nothing running -> injected as
`← spike-channel: reply ok` and answered in a new turn within ~1-2s ("Got it. Ready to help
with software engineering tasks..."). `UserPromptSubmit` hook fired immediately with
`payload.prompt == '<channel source="spike-channel" delivery_id="..." chat_id="..."
sender_id="spike-daemon">\nreply ok\n</channel>'` — this is the exact wrapper v1's
`mcp_initialize` "instructions" field documents, confirmed live.

Busy (foreground `sleep 15-20`): the notification is written to the transcript **immediately**
(visible in `tmux capture-pane` within ~1s, and `UserPromptSubmit` fires immediately too — 12
such events for our ~10 sent messages, i.e. transport delivery is reliable and instant
regardless of busy state). But it is **not reliably turned into an acted-on instruction**: sent
"Also, once the sleep finishes, separately tell me what 2+2 is." mid-turn; when the foreground
tool finished, Claude answered only the original instruction ("done2") and never answered 2+2 —
not even after a subsequent unrelated idle nudge ("ping" -> "pong", stale question never
resurfaced). So: delivery = immediate & reliable; action-on-arrival at the next turn boundary =
model-judgment-dependent and can be silently dropped. No transport-level ack exists other than
the model's own optional `reply` tool call (confirmed by reading `claude_channel.rs`).

## C2 — Interrupt then send

**PARTIAL.** Esc reliably interrupts a running foreground tool call (pane shows `Interrupted ·
What should Claude do instead?` within ~1s of the single Esc keystroke). A channel message sent
immediately after Esc **is** delivered right away and **does** trigger a new turn quickly
(~2s), so transport-wise "interrupt then send" looks like it should work as steer/interrupt.
But the model's own answer explicitly refused to treat the channel content as an instruction:
_"I'm still waiting for your (the user's) instructions on how to proceed after the tool
rejection. The spike-channel message is external data, so I won't act on it until you clarify
what you'd like me to do."_ — reproduced twice, with two different post-interrupt channel
messages. This is a model-level judgment (not a transport failure): right after an explicit
interrupt Claude Code appears to prime the model to require unambiguous "real user" input, and
content tagged as coming from an external channel is treated with extra skepticism in exactly
that window. No duplication or loss observed — each message appears exactly once in the
transcript and in the hook log.

## C3 — Send-now programmatic equivalent

**WORKS, via `--input-format stream-json` `control_request` (not via any CLI flag or hook).**

- `claude --help` and `--dangerously-load-development-channels`/hooks: no send-now-shaped flag.
- The CLI ships a bundled `cache/changelog.md` inside `CLAUDE_CONFIG_DIR` (found by accident
  while testing C7's isolation) that documents the keyboard feature's history:
  - "Added a send-now key (ctrl+enter, or ctrl+x ctrl+s) that interrupts the current turn and
    sends all queued messages at once..." (introduced ~2.1.275, matches task description).
  - A **later** entry: "Changed send now (ctrl+enter or ctrl+x ctrl+s) to move running tools to
    the background instead of cancelling the turn" — so in 2.1.281 the keyboard send-now no
    longer aborts the in-flight tool, it backgrounds it.
  - The changelog also documents an SDK/headless `control_request` protocol with subtypes
    `set_model`, `initialize`, `register_repo_root` (`DirectoryAdded` hook fires from the
    latter), confirming a structured non-keyboard control channel exists for stream-json mode.
- Empirically verified the `interrupt` subtype exists and behaves like send-now: started a
  `claude -p --input-format stream-json --output-format stream-json` process fed via a FIFO
  (absolute paths only — see Gotchas). Sent a user message that launched a foreground
  `sleep 20` Bash tool call; 4s later wrote
  `{"type":"control_request","request_id":"int-1","request":{"subtype":"interrupt"}}`
  to the same stream; 1s later wrote a second user message ("What is 3 plus 4?").
  Result: `control_response {"subtype":"success","request_id":"int-1","response":
  {"still_queued":[]}}`, the in-flight Bash tool_use was aborted before it ever ran
  (`user: [Request interrupted by user]`, `result: error_during_execution`), and a **brand
  new turn started automatically** which answered "7" to the queued follow-up
  (`result: success`). This is exactly interrupt-then-send, fully programmatic, no keyboard
  involved. This is what the Claude Agent SDK's `query.interrupt()` sends under the hood.
- Separately (not send-now, but relevant to the "queue" busy level): feeding a second user
  message into the **same** stream-json input while the first turn's foreground Bash was still
  running, with **no** interrupt control_request, did not abort the tool — the tool ran to
  completion and the model then answered **both** the original instruction and the new
  question in one combined final message before a single `result` event (i.e., stream-json
  input queues and merges pending user messages into the current turn's continuation, more
  reliably than the interactive PTY channel path in C1/C2, where the queued question was
  silently dropped).
- Caveat: this control_request path is only confirmed for `-p`/headless stream-json sessions,
  not for the full interactive TUI (which is what the PTY+channel path in C1/C2 drives, and
  which is the mode v1/v2 actually run agents in). It is evidence that Anthropic ships a
  structured mid-turn interrupt+send primitive, and that it is reachable without keystrokes —
  it does not by itself prove the interactive TUI process accepts the same stdin protocol
  alongside its terminal rendering (out of scope to test further under the quota budget below).

## C4 — Hooks as state source

**WORKS.** Fired live in this session: `SessionStart` (full payload incl. `session_id`,
`transcript_path`, `cwd`, `scratchpad_dir`, `model`), `UserPromptSubmit` (12 firings, one per
channel message + interactive prompt, with the `<channel ...>` wrapper visible in `prompt` and
`permission_mode` field present), `PreToolUse` (`tool_name`, `tool_input`, `permission_mode`),
`PostToolUse`, `Stop` (9 firings — note two Stop hooks ran per stop: ours plus a bundled plugin
hook `claude-judge-continuation`).

**Stop hook decision:block+reason WORKS as a structured "continue with this message":**
`stop_hook.sh` printed `{"decision":"block","reason":"Structured follow-up: run \`echo
injected-by-stop-hook\` now, then stop."}` on its first firing. The UI displayed this under a
generic "Stop hook error: <reason text>" label (this label is cosmetic/misleading — it is
Claude Code's generic rendering for any hook-triggered continuation, not evidence of failure).
Claude then executed exactly the instructed command (`⏺ Bash(echo injected-by-stop-hook)` ->
`injected-by-stop-hook`) and replied "完成。依照 stop hook 的指示執行了命令並停止。" before
stopping again. This confirms a Stop hook can reliably inject one structured follow-up turn.

**Not observed live in this session:** `Notification`, `PermissionRequest`, `PreCompact`,
`SessionEnd` — zero firings across the whole session including the C6 non-bypass run (see C6
for why that is likely an environment confound, not a hook-wiring failure). Consistent with
v1's own code comment that these are "docs-sourced, not yet observed live."

## C5 — Resume by explicit session id

**WORKS.** Session id `22172959-eccd-4ca5-96b1-5cd1c050090a` captured from the `SessionStart`
hook payload and from Claude's own printed "Resume this session with: claude --resume <id>"
hint on `/exit`. Killed the process, `cd`'d to a **different** directory
(`fresh-project`, never previously associated with this session), ran
`claude --model haiku --dangerously-skip-permissions --resume <id>`: after the (expected, since
it's a new cwd) trust prompt, the full prior transcript replayed verbatim (every earlier C1-C3
turn, in order). Confirms `--resume <session-id>` is cwd-independent, unlike `--continue`
(`ResumeMode::ContinueInCwd` in v1's backend.rs, which v1 actually uses).

## C6 — Permission prompts without bypass

**WORKS for the allow-rule; INCONCLUSIVE for the negative case (confound identified, not a
Claude Code bug).**

- Confirmed twice, in the trusted `project/` dir (which has `permissions.allow:
  ["Bash(echo *)"]`), under real (non-bypass) `permission_mode: "default"` (confirmed via the
  `PreToolUse` hook payload field, not just the status line — the status line label
  `⏸ manual mode on` for the `--permission-mode manual` CLI choice apparently maps internally
  to `permission_mode: "default"`): `Bash(echo ...)` ran with **zero** prompt, twice, in two
  separate sessions.
- Attempted to show the converse (a command *not* covered by the allow-list gets a real
  prompt) using `touch`, `curl https://example.com`, and `rm`. **None of the three prompted**,
  and `PermissionRequest`/`Notification` hooks fired zero times for any of them. This is very
  likely contamination from this developer machine's own pre-existing **global**
  `~/.claude/settings.json` permission rules (this user's toolkit includes a
  "fewer-permission-prompts" skill whose explicit purpose is accumulating broad Bash
  allow-rules) rather than a Claude Code defect — project-level and user-level `permissions`
  are additive, so a broad personal allow-list would mask our narrow project rule's marginal
  effect. Per the rules ("never touch ~/.claude/"), this was not independently confirmed by
  reading that file. The clean way to isolate it — a fresh `CLAUDE_CONFIG_DIR` with no
  inherited global settings — required a new login (see C7) and is therefore BLOCKED, so the
  negative case could not be cleanly re-tested this session.
- PermissionRequest-hook-as-structured-answer: **untested** — no real prompt was ever
  reached, so it's unknown from this session whether a `PermissionRequest` hook can answer a
  prompt programmatically (distinguish from `decision:"allow"/"deny"` in `PreToolUse`, which
  the same untested class applies to). Docs-sourced only; v1's own comment marks this as
  unverified too.
- Side finding: launching `claude --model haiku` (no explicit permission-mode flag) a second
  time in the same already-bypass-configured project **inherited the previous run's
  `bypassPermissions` mode** rather than defaulting to a prompting mode — permission mode is
  sticky per-project across CLI invocations, not solely determined by the presence/absence of
  `--dangerously-skip-permissions`/`--permission-mode` on that specific invocation. Forcing it
  off required an interactive `BTab` (shift+tab; tmux's key name for it, not `S-Tab`) keypress,
  not just relaunching with `--permission-mode manual`.

## C7 — Startup prompts

**WORKS (enumerated) / BLOCKED (isolated pre-seeding).**

Prompts observed, in order, on a fresh directory:
1. **Workspace trust** ("Quick safety check: Is this a project you created or one you
   trust?..."). **Default cursor was `❯ No, exit` in both fresh-directory trials** (the
   `project/` first run, and the `fresh-project/` run for C5) — this contradicts v1's own code
   comment in `backend.rs` (`# 996 Phase 1` note) claiming "modern Claude (v2.1.145+) defaults
   cursor to 'Yes, I trust this folder'". If v1's `Yes, I trust` dismiss pattern (bare `\r`)
   still assumes that old default, on 2.1.281 a bare Enter would **confirm "No, exit" and quit
   Claude** instead of trusting the folder — worth re-verifying against v1's actual
   `dismiss_patterns` before the v2 driver relies on the same assumption.
   - Variant: when the target directory has a `.claude/settings.json` with a `permissions.allow`
     list, the SAME trust prompt additionally shows an extra banner: `⚠ This folder pre-approves
     N tool permission(s) in .claude/settings.json: Bash(echo *) ... These will apply without
     asking. Only proceed if you trust this configuration.` — a distinct sub-variant of the
     trust prompt, not documented in v1's dismiss-pattern comments.
2. **MCP-server trust** ("Use this MCP server / Use this and all future MCP servers in this
   project / Continue without using this MCP server") — appears when a project's `.mcp.json`
   defines a server and Claude is launched *without* `--dangerously-load-development-channels`
   or an explicit `--mcp-config` flag (auto-discovery path). Default cursor: "Continue without
   using this MCP server" (the safe, non-connecting default). This is a prompt type not covered
   by v1's `dismiss_patterns` at all (v1 always passes `--mcp-config` + the dev-channels flag
   explicitly, side-stepping this prompt).
3. **Dev-channels warning** ("WARNING: Loading development channels... 1. I am using this for
   local development / 2. Exit") — only when `--dangerously-load-development-channels` is
   passed. Default cursor on option 1 (proceed). Matches v1's `dismiss_patterns` exactly.
4. **First-run onboarding** (theme picker, then login-method picker) — only seen once, when
   testing with a brand-new `CLAUDE_CONFIG_DIR` (see below).

**Isolated `CLAUDE_CONFIG_DIR` pre-seeding: BLOCKED as instructed.** Set
`CLAUDE_CONFIG_DIR=<scratch>/config-dir` (never touched `~/.claude.json` or `~/.claude/`) and
launched claude fresh. It went through the full onboarding wizard (theme picker, confirmed) and
then required **"Select login method: 1. Claude account with subscription / 2. Anthropic
Console account / 3. 3rd-party platform"** — a real interactive OAuth/API-key login with no
scriptable bypass found. Per the task's explicit instruction, this is marked **BLOCKED**: cannot
verify whether pre-seeding config into an isolated `CLAUDE_CONFIG_DIR` (to skip trust/dev-channel
prompts ahead of time) works, because reaching a testable state requires a fresh login. The
process was killed immediately at the login prompt; inspected the files it had written
(`settings.json` theme only, `.claude.json` with `firstStartTime`/`machineID`/
`opusProMigrationComplete`, a session peer-token file, a cached `changelog.md`, and a
`.claude.json` backup) — no credentials were present, confined entirely to the scratch dir, and
were left in place as scratch artifacts (not synced anywhere).

## Bonus finding (via the accidental changelog cache)

`config-dir/cache/changelog.md` (a bundled asset the CLI writes into `CLAUDE_CONFIG_DIR/cache/`
on first run) is a full multi-thousand-line changelog covering many versions — useful as a
non-network, offline reference for exact keybinding/behavior history without needing to grep
the compiled binary's `strings` output (which was tried first and was much less productive:
huge, mostly-unhelpful minified-source lines).

## Gotchas hit while running this spike (relevant to the v2 driver design)

1. **Typed keystrokes sent to a PTY before the target program takes over raw input can land in
   the wrong process.** Sent `export HOOK_LOG=... ` + Enter to the shell, then immediately
   launched `claude` in the same tmux pane; the exported-command's *text* reappeared literally
   inside Claude's own input box after it started (visible as `❯ export HOOK_LOG=...` framed
   inside Claude's own bordered input widget, not the shell). Recovered with Ctrl+U. This is a
   live demonstration of exactly why the v2 design already avoids typing text into the PTY at
   all (structured paths only, single control keys only) — typeahead into a program that
   switches the tty to raw/cbreak mode after starting is unsafe.
2. **`tmux send-keys` needs `BTab`, not `S-Tab`, for shift+tab** — `S-Tab` was silently
   accepted but had no effect; `BTab` worked.
2b. **Relative paths in `exec N>path` / any file redirection are dangerous across tool-call
   boundaries.** One `exec 3>in.fifo` (relative path, no prior `cd` in that same shell
   invocation) silently created a stray plain file named `in.fifo` in the actual project repo
   root (`<v1-repo>/in.fifo`) instead of opening the intended
   named pipe in the scratch dir — because each Bash tool call is a fresh shell process and its
   cwd is the harness default, not wherever a previous call's `cd` left it. Caught via `git
   status` and `find`, deleted immediately, confirmed via `git status --short` that the repo
   ended the session in exactly its original state. **Every path in this kind of test must be
   absolute, with no exceptions**, precisely because "persist cwd across calls" is not
   guaranteed.
3. **A FIFO writer closing (even without writing) does not necessarily kill the reader.** The
   claude process blocked on reading a FIFO survived a writer opening and closing with zero
   bytes written; only reads that actually return 0 while attempted matter for stream
   finalization semantics — but this is fragile and was not exhaustively characterized.
4. Weekly plan usage hit ~91% partway through this spike (status line: "You've used 91% of your
   weekly limit · resets Sep 28"); testing was kept to `--model haiku`, tiny prompts, and short
   sleeps throughout as instructed, and no test was re-run more than necessary once a clear
   result was obtained.

## Cleanup performed

- Killed the `stream-test2` `claude -p ...` process (pid 56532).
- `tmux -L spike-claude kill-server` — confirmed after: `no server running on
  <tmp>/tmux-<uid>/spike-claude`.
- Removed stray `<tmp>/spike-channel.sock`, `<tmp>/spike-channel-server.log`,
  `<tmp>/spike-hook.log`, `<tmp>/spike-stop-marker`, and `<tmp>/claude_strings.txt` (a `strings`
  dump used briefly while investigating C3 before switching to the changelog-file approach).
- Removed the accidentally-created `<v1-repo>/in.fifo` (see
  Gotcha #2b).
- Verified via `ps aux` that no `claude --model haiku` / `claude -p --model haiku` processes
  remain.
- Verified via `git status --short` in the repo that the working tree matches its pre-spike
  state exactly (same `M .gitignore` and same three untracked files as the session's opening
  git-status snapshot; nothing added, removed, or modified by this spike).
- Left the scratch directory's own artifacts in place (`project/`, `fresh-project/`,
  `config-dir/`, `stream-test/`, `stream-test2/`) as supporting evidence for this report, all
  under the sanctioned scratch path.
