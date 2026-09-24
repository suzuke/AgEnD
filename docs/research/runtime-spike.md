# agend v2 runtime-layer spike: holder vs tmux vs herdr

> 歸檔註：設計階段的原始紀錄，內容原樣保存（2026-09-24）。只把絕對路徑換成佔位符：`<scratchpad>` = 當時的暫存目錄、`<v1-repo>` = agend-terminal repo、`<tmp>` = 系統暫存目錄、`~` = 使用者家目錄。文中提到的腳本、log、schema dump 沒有一起歸檔。

Date: 2026-09-24. Method: hands-on testing (tmux with isolated `-L agend-spike`
socket; herdr v0.9.1 release binary run headless under sandboxed
`XDG_CONFIG_HOME`/`XDG_STATE_HOME`/`HERDR_SOCKET_PATH`, never touching the
user's live `/opt/homebrew/bin/herdr` install or `~/.config/herdr`) plus
source reading (herdr cloned to `./herdr-src`; v1 at
`<v1-repo>`). All spike processes killed
and scratch sockets removed at the end; verified user's default tmux server
and `~/.config/herdr` untouched.

Prior art found mid-task: `<local-notes>/herdr-vs-agend-terminal-analysis.md`
(2026-07-04, base herdr v0.7.1 under `ogulcancelik/herdr`, license recorded then
as AGPL-3.0-or-later). Current `herdrdev/herdr` v0.9.1 clone confirms
`LICENSE`/`Cargo.toml` = **Apache-2.0** now — the project apparently relicensed
and/or moved orgs between v0.7.1 and v0.9.x. That old doc was about porting
*features* (PTY write-actor, plugin/event-hook, FD-passing hot upgrade) into
v1, not about the runtime-layer build-vs-buy question asked here; it's
consistent with, but not a substitute for, this spike.

## Executive summary

Recommend **herdr as the runtime, used as a plain external server your daemon
is a client of** — with the explicit understanding that herdr does **not**
give crash-survival beyond what a well-run tmux server or a disciplined
custom holder would give. All three options share one hard truth, confirmed
empirically below: **if the persistent PTY-owning process itself is hard-killed,
every hosted agent process dies with it, full stop** — no tool "saves" you from
that. What differs is (a) how battle-tested that persistent process is against
crashing by itself (tmux: decades, extremely mature; herdr: ~6 months, young,
single small team, own crash telemetry unknown; custom holder: whatever v1's
own PTY-hygiene bugs and Windows story). tmux is the safe, boring default:
zero new install for most Unix users, dead-simple scriptable protocol, dead
process/exit-status via `remain-on-exit`, real push notifications via control
mode. herdr is materially better on multi-process-per-agent-unit (a true
second independent PTY pane per workspace, not a backgrounded job hack) and
on screen-text capture ergonomics (`pane read` gives clean plain text
out of the box vs tmux's ANSI-decorated capture-pane), at the cost of a new
external dependency the open-source user must install, a much younger/smaller
project, and Windows support that's explicitly beta. Self-writing a holder is
the only path with zero external dependency but reinvents a nontrivial amount
of code (protocol versioning, reattach, multi-process-per-unit) that v1 has
never had to build because v1 currently kills agents on daemon shutdown by
design.

## Requirement scoring (● = can, ◐ = partial, ○ = cannot / not verified)

| Req | Custom holder | tmux | herdr |
|---|---|---|---|
| R1 daemon restart, agent/PTY survives, daemon reattaches | ◐ not built today (v1 kills agents on shutdown); *architecturally* achievable as a separate long-lived process, same ceiling as the other two (nothing survives a hard-kill of the holder itself) | ● verified: killing the attaching client/control process never touches the tmux server or its panes (session/windows/processes all alive after client death) | ◐ verified client-survives-disconnect **and** verified server-hard-kill kills every hosted PTY + all children instantly, with no orphan/resurrect (fresh server restart recreates the pane *shell* with a new PID, not the old process) — see transcript §T5. Graceful cooperative upgrade exists (`src/server/handoff.rs`, FD-passing, Unix-only) but wasn't independently re-verified here (docs/tests only) |
| R2 second process per agent unit, also daemon-restart-safe | ◐ would need new code: v1's spawn path is 1 PTY = 1 process, no unit grouping today | ● verified: `new-window` in the same session hosts a fully independent second long-lived process (`helper` window), survives exactly like the primary | ● verified: `pane split` creates a second independent PTY (`w1:p2`) in the same tab/workspace with its own env — cleaner unit than tmux's window-in-session (each pane has its own read/process-info identity) |
| R3 get current-screen text for hard-gate classifier | ◐ would reuse v1's existing `VTerm`/`dump_screen` machinery (already proven, not a rewrite) | ● verified: `capture-pane -p` returns clean line-based text instantly | ● verified: `pane read --source recent` returns plain text, no manual ANSI stripping needed — nicer default than tmux |
| R4 subscribe to output/change stream | ◐ not built; would be new code on top of existing PTY read loop | ● verified: control mode (`-C`) emits async `%output %<pane> <bytes>` lines the moment any pane produces output, plus `%session-changed` etc, over the same connection used for commands | ● documented only (not independently exercised): `events.subscribe`/`events.wait` in the JSON schema (src/api/schema.rs) — not run in this spike |
| R5 exit status / exit code | ◐ not built; v1 currently just reports "agent gone" | ● verified: `remain-on-exit` (must be set **globally** via `set-window-option -g`, session-scoped set-option only applies to the already-existing active window — a real gotcha) + `pane_dead_status` gave the exact exit code (42, then 7 for a SIGTERM-trapped exit) | ● documented (`pane.process_info` / `PaneProcessInfo`, src/api/schema/panes.rs) — not independently exercised for a dead process in this spike (only live shell_pid was checked) |
| R6 env injection at spawn | ◐ trivial addition to existing `CommandBuilder` spawn call | ● verified: `new-session -e KEY=VAL` sets session environment; confirmed it **also** propagates to windows created later in the same session | ● verified: `workspace create --env KEY=VAL` env visible via `echo $VAR` inside the pane |
| R7 programmatic API stability from Rust | ◐ full control, but v1's own wire protocol today is a single `u8` version byte with **no negotiation** — any format change is breaking (framing.rs) | ● mature, stable CLI/text protocol; no official Rust client crate, hand-rolled parsing of line/blocks needed, but the format has been unchanged for decades | ● explicit `PROTOCOL_VERSION` (=22) + documented "generation 1 is the compatibility floor, immutable field" policy (src/CLAUDE.md) — best-documented stability story of the three, but no Rust library crate either (`Cargo.toml` ships a binary only) — integration is hand-rolled JSON-lines-over-Unix-socket from Rust either way |
| R8 user can attach/switch panes with the tool's own UI | ◐ none today — would have to build a TUI, which the user has already decided *not* to do for agend itself | ● mature `tmux attach`, huge ecosystem familiarity | ● full custom TUI client (`herdr` bare / `herdr session attach`), actively developed, screenshots in README |
| R9 install burden (macOS + Linux) | ● zero — ships inside agend's own binary | ● `/opt/homebrew/bin/tmux` already present here; broadly preinstalled or one-line package-manager install on both OSes | ◐ not preinstalled; single static-ish binary via `install.sh` (fetches prebuilt, no compile) or `brew`/`mise` — one extra dependency open-source users must add, project is ~6 months old |
| R10 send input capability (resize/signal/keys) — capability only | ◐ trivial via portable-pty (`resize()`, kill via pid) | ● verified: `resize-window`, `send-keys`, and `kill -TERM <pane_pid>` (delivered to and trapped by the pane's process, exit status recorded) all work | ● documented: `pane.send_text`/`send_keys`/`send_input`/`pane.resize` in schema — not independently exercised in this spike beyond `pane run`/`send-keys`/`send-text` CLI wrappers, which **did** work (used to inject env-check + background marker) |

## Test transcripts (abbreviated — full raw output was in the session, key lines kept)

### T1. tmux — env injection + capture-pane
```
tmux -L agend-spike new-session -d -s spike -e AGEND_AGENT_ID=agent1 -e AGEND_TEST=hello "bash -c '...; while true; do date; sleep 1; done'"
$ tmux -L agend-spike show-environment -t spike | grep -i AGEND
AGEND_AGENT_ID=agent1
AGEND_TEST=hello
$ tmux -L agend-spike capture-pane -t spike -p
Thu Sep 24 07:06:55 CST 2026
... (clean per-line text)
```

### T2. tmux — remain-on-exit gotcha + pane_dead_status
```
# session-scoped set-option did NOT apply to a window created afterward:
tmux -L agend-spike set-option -t spike remain-on-exit on
tmux -L agend-spike new-window -t spike -n dying "... exit 42"
$ list-panes -> window "dying" already gone (default kill-on-exit still won)

# global window-option DOES apply to future windows:
tmux -L agend-spike set-window-option -g remain-on-exit on
tmux -L agend-spike new-window -t spike -n dying2 "... exit 42"
$ tmux -L agend-spike list-panes -t spike -a -F '#{window_name} #{pane_dead} #{pane_dead_status}'
dying2 1 42
```

### T3. tmux — control mode (-C) push stream + dead-pane visibility
```
tmux -L agend-spike -C attach-session -t spike   # via a persistent fifo fd, not a blocking pipe
> list-panes -a
%begin ... / %end ...
spike:2.0: [80x24] ... %3 (active) (dead)
> capture-pane -p -t spike:helper
%output %2 helper-tick 1790205178\015\012      <- async push notification, unsolicited
%begin ... helper-tick lines ... %end
```
Killing this `-C` control client (`kill -9`) left the session, all 4
windows, and the helper's tick loop running — verified with `has-session`
and `list-windows` immediately after.

### T4. tmux — signal delivery + custom exit code
```
tmux -L agend-spike new-window -t spike -n sigtest "bash -c 'trap \"echo GOT_SIGTERM; exit 7\" TERM; while true; do sleep 1; done'"
kill -TERM $(tmux -L agend-spike list-panes -t spike:sigtest -F '#{pane_pid}')
$ list-panes -F '#{pane_dead} #{pane_dead_status} #{pane_dead_signal}'
1 7 
```

### T5. herdr — the load-bearing crash test
```
XDG_CONFIG_HOME=.../sandbox/xdg-config XDG_STATE_HOME=.../sandbox/xdg-state \
HERDR_SOCKET_PATH=<tmp>/agend-herdr-spike/herdr.sock ./herdr-bin server &
herdr workspace create --label spike --env AGEND_AGENT_ID=agent1 --env AGEND_TEST=hello
  -> workspace w1, root pane w1:p1, shell_pid 69753
herdr pane run w1:p1 'echo ENV_CHECK $AGEND_AGENT_ID $AGEND_TEST; (while true;do date +%s;sleep 1;done)& echo CHILD_PID=$!'
  -> ENV_CHECK AGEND_AGENT_ID=agent1 AGEND_TEST=hello ; CHILD_PID=85023
ps -p 69753,85023   # both alive, zsh(69753) is a DIRECT CHILD of the herdr server pid

kill -9 <herdr server pid>
ps -p 69753,85023   # -> BOTH GONE. No orphan, no survivor.

# fresh server, same XDG state dirs:
./herdr-bin server &
herdr pane list      # -> w1:p1 REAPPEARS (layout/workspace metadata persisted to disk)
herdr pane process-info --pane w1:p1
  -> shell_pid 8875  # <- a brand-new process, NOT 69753
herdr pane read w1:p1 --source recent  # -> blank fresh prompt, no CHILD_PID history
```
Conclusion: herdr persists **layout** (workspaces/tabs/panes skeleton) across
a hard server kill, and respawns a fresh default shell into each slot — it
does **not** persist or reattach the live agent process. This directly
qualifies the "daemon restart" framing in herdr's own README/tests
(`detach_reattach.rs`, `host_shutdown.rs`): those tests exercise a *client*
disconnecting from a *still-running* server, not the server process itself
dying. For agend's actual requirement, this is fine **only if** agend's own
daemon-restart path is a different OS process from the herdr server and never
signals it — i.e., herdr must be run as an independent, rarely-touched local
service, not spawned-and-owned per daemon lifecycle.

### T6. herdr — second independent pane (R2)
```
herdr pane split w1:p1 --direction right --env AGEND_ROLE=helper
  -> {"pane_id":"w1:p2", "tab_id":"w1:t1", ...}
herdr pane list  # -> w1:p1 and w1:p2 both listed, independent terminal_id each
```

## v1 codebase evidence (self-written holder cost estimate)

- v1 already uses `portable-pty = "0.9"` (vanilla, unpatched) for PTY spawn —
  `src/agent/mod.rs:12-13`, `native_pty_system().openpty()` around line
  1478-1486. PTY-syscall-specific code is small (~30 LOC); the bulk of
  `src/agent/mod.rs` (3161 lines) is product logic (crash detection, dev
  modal, dismissal), not PTY plumbing.
- No raw ring-buffer replay of PTY bytes exists today — v1 sidesteps the
  "reattach mid-escape-sequence" problem entirely by reconstructing terminal
  state from a parsed `VTerm` and re-emitting a clean `dump_screen()` ANSI
  dump on reattach, rather than replaying a byte buffer. No comments/commits
  indicate this has ever bitten the project. **This pattern is reusable for
  a v2 holder** and avoids the hardest part of "write your own ring buffer."
- v1 does **not** survive daemon restarts today: `src/daemon/mod.rs:1609-1660`
  `shutdown_sequence` explicitly drains the whole agent registry and
  terminates every agent (SIGTERM + grace + SIGKILL) on daemon shutdown by
  design. Building R1 into a custom holder means adding an entire new
  decoupled-process architecture v1 has never needed.
- Daemon↔bridge wire protocol (`src/framing.rs`) is `[tag:u8][len:u32][payload]`
  with a single `PROTOCOL_VERSION=1` constant and **no negotiation** — any
  format change today is a breaking change. A v2 holder aiming for R7-grade
  stability would need real version-negotiation design, which doesn't exist
  as a template to copy from v1.
- Rough LOC estimate for an MVP custom holder covering R1+R2+R4+R5+R6+R10
  (reusing portable-pty + the existing VTerm dump-on-reattach pattern, adding
  a new decoupled process, a versioned protocol, and multi-process-per-unit
  tracking): **~1,500-2,500 LOC of new Rust, plus a roughly comparable or
  larger amount of test code** given this repo's test culture (~4935 existing
  tests) — this is an estimate reasoned from the pieces enumerated above, not
  a measured figure.

## Risks / open questions (not fully resolved by this spike)

- herdr R4 (`events.subscribe`) and R5/R10 dead-pane and send-input paths
  were confirmed **from source/schema and CLI help only**, not independently
  round-tripped against a live dead process in this session — worth a
  follow-up spike before committing.
- herdr's graceful FD-passing hot-upgrade (`src/server/handoff.rs`) was not
  independently exercised here (relied on the earlier analysis doc + this
  session's Explore agent's source citations) — if agend ever needs herdr's
  OWN binary to upgrade without dropping panes, that path should be tested
  directly, and note it is Unix-only with no Windows fallback per source.
  agend's CI targets `windows-latest`, so herdr's Windows story (`beta`,
  per README) is a real open risk for that platform if R8 (user-facing
  attach) is wanted there too.
- herdr is ~6 months old under active, fast-moving development (per the
  July analysis and the September CHANGELOG cadence observed); its own
  crash frequency in the wild is unknown to this spike.
- The comparison assumes agend would run herdr/tmux as an **externally owned,
  long-lived local service** the daemon merely connects to — if agend's
  design instead wants to spawn-and-own that process per daemon lifecycle,
  none of R1's "survives restart" evidence applies, for any of the three
  options.
