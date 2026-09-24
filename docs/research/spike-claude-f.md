# Claude Code 2.1.281 driver follow-up spike — F1-F5

> 歸檔註：設計階段的原始紀錄，內容原樣保存（2026-09-24）。只把絕對路徑換成佔位符：`<scratchpad>` = 當時的暫存目錄、`<v1-repo>` = agend-terminal repo、`<tmp>` = 系統暫存目錄、`~` = 使用者家目錄。文中提到的腳本、log、schema dump 沒有一起歸檔。

Binary: `~/.local/bin/claude` (2.1.281), same as the prior spike.
tmux private socket: `-L spike-claude-f` (killed at end, confirmed "no server running").
Scratch root: `.../scratchpad/spike-claude-f/` (this dir). Harness reused from
`scratchpad/spike-claude/`: `channel_server.py`, `send_msg.py`, `hook.sh` copied
verbatim into `common/`; a new `stop_hook_f4.sh` (queue-file delivery) and
`drive_trial.py` / `drive_f4_trial.py` (tmux automation) were added.

**Setup correction vs the prior spike**: `--mcp-config <file>` with an explicit
path gave `server:spike-channel · no MCP server configured with that name`
(tried both relative and absolute paths). Switched to project-root
auto-discovered `.mcp.json` + bare `--dangerously-load-development-channels
server:spike-channel` (no `--mcp-config` flag) — this is exactly what the
`claude-code-channels-custom-mcp` skill recommends and what the original spike
actually used; it connects cleanly with no extra "Use this MCP server" prompt
(that prompt apparently doesn't fire on this flag path — matches the skill's
"only --dangerously-load-development-channels, no --channels needed" note).

**Harness bug found and fixed early**: sending the busy-task prompt text and
`Enter` as a *single* `tmux send-keys` call, immediately after the ready
banner appeared, silently dropped the keystrokes about half the time (the
freshly created pane's zsh/prompt theme wasn't done settling yet) — the text
sat unsubmitted in the input box and no `UserPromptSubmit` hook fired. Fixed
by (a) a 1s settle delay after `new-session`, (b) splitting the prompt text
and `Enter` into two separate `send-keys` calls with a short delay, (c) a
retype-once fallback if the echoed command doesn't appear after the first
attempt. After the fix, every trial's `UserPromptSubmit`/`PreToolUse` hook
firings confirm real delivery (not just pane-text appearance).

**Baseline control (not asked for by name, but run for scientific honesty)**:
before crediting F1/F2 with "fixing" case (c), ran 3 Esc→channel trials with
*no* CLAUDE.md framing and *no* sender metadata in the message (`proj-baseline`,
message "Once the sleep finishes, separately tell me what N+M is."). Result:
**0/3 acted on the message** — in all three, Claude instead re-ran the
interrupted `sleep 15` (once with explicit Chinese narration "工具呼叫被拒絕了。
我現在重新嘗試執行 sleep" = "the tool call was rejected, retrying sleep"),
completely ignoring the channel content. This is a *different* failure mode
from the original spike's explicit "external data, waiting for the real user"
refusal text, but the same practical outcome (message dropped). Confirms the
harness reproduces case (c)'s unreliability and that F1/F2's fix below is a
real effect, not a byproduct of some other setup change.

## F1 — Source framing via CLAUDE.md

**WORKS, 3/3.**

`proj-f1/CLAUDE.md` (full text in `common/CLAUDE.f1.md`): states messages via
the `agend` channel come from "the user's own team members or the daemon
itself" and should be acted on directly, "including immediately after you
have just interrupted or been interrupted from a previous tool call."
Message sent (plain, no per-message metadata): "Once you are free, please
compute N+M and reply with just the number."

| Trial | Sent after Esc | Reply |
|---|---|---|
| f1-t1 | 14+9 | `⏺ 23` (correct) |
| f1-t2 | 21+16 | `⏺ 37` (correct) |
| f1-t3 | 33+8 | `⏺ 41` (correct) |

Raw excerpt (f1-t1, `captures/f1-t1.txt:16-25`):
```
⏺ Bash(sleep 15)
  ⎿  Interrupted · What should Claude do instead?

← spike-channel: Once you are free, please compute 14+9 and reply with just …

⏺ 23
✻ Cogitated for 2s · done 2:48 PM
```
No refusal language ("external data", "waiting for the actual user") appeared
in any of the 3 trials.

## F2 — Sender metadata inside the message body

**WORKS both ways, 3/3 each.**

Message format: `"from: reviewer-2 (your team) . task T-NN . request: once
you are free, compute N+M and reply with just the number."`

- **F2 alone** (`proj-f2-nof1`, no CLAUDE.md at all): 3/3 correct answers
  (12+15→27, 19+23→42, 25+17→42). Excerpt (`captures/f2a-t1.txt:21-23`):
  `← spike-channel: from: reviewer-2 (your team) . task T-45 . request: ...` →
  `⏺ 27`.
- **F1 + F2 combined** (`proj-f2-f1`, CLAUDE.md present + metadata in message):
  3/3 correct (31+9→40, 18+27→45, 29+14→43).

So on this model/version, **either framing alone is already sufficient**;
combining them adds no observable extra reliability (both are 3/3, baseline
is 0/3). This is evidence, not proof, that sender-metadata-in-body is doing
most of the work here — with only 3 trials per arm we can't rule out haiku
being sensitive to trial-order or context-length effects, but the contrast
with the 0/3 baseline is clean.

## F3 — Wait for idle (Stop hook) after Esc before sending

**DOESN'T (as literally specified) / WORKS (as a delay proxy), 3/3 answered.**

Key finding: **pressing Esc to interrupt a running tool call never fires a
Stop hook.** Instrumented `proj-f2-f1`'s Stop hook (generic logger) and
polled `HOOK_LOG` for up to 10s after every Esc across all 3 F3 trials —
`stop_seen_after_esc` was `false` in all 3. Confirmed independently: the
whole session's hook log (`logs/f2f1-hook.log`) shows `Stop` events only
immediately following an actual model reply, never following a bare
interrupt. So "wait for a Stop/idle signal" cannot be implemented after Esc
specifically — there is no such signal in this state; the UI's own
"Interrupted · What should Claude do instead?" text is the only observable
idle marker, and it is available immediately (same time as the immediate-send
variant already used).

Because there was no real signal to gate on, the driver fell back to a fixed
~10s poll-timeout before sending (same combined-framing message as F2+F1).
Result: 3/3 correct (16+13→29, 24+19→43, 37+5→42) — identical success rate to
the "send immediately" F1/F2 trials. **Ordering (immediate vs ~10s later)
made no observable difference once source framing is present**; the original
C2 refusal was not a race/timing artifact, it was the missing-framing problem
F1/F2 already fix.

## F4 — Busy case, held message delivered via Stop hook block+reason

**WORKS, 3/3, no loops.**

Setup (`proj-f4`, `stop_hook_f4.sh`, no channel/MCP involved): asked Claude to
run `sleep 15` to completion (no Esc), wrote the "held" message into a
`QUEUE_FILE` while it was busy, let the turn finish naturally. The Stop hook
reads the queue, and if non-empty emits `{"decision":"block","reason":
"<message>"}` and **truncates the queue file before emitting** (consume-once
guard) so a subsequent Stop (triggered by the block) sees an empty queue and
does not re-block.

| Trial | Held message | Result |
|---|---|---|
| f4-t1 | 12+31 | `Stop hook error: Compute 12+31...` → `⏺ 43` (correct) |
| f4-t2 | 27+15 | → `⏺ 42` (correct) |
| f4-t3 | 33+19 | → `⏺ 52` (correct) |

Raw excerpt (`captures/f4-t1.txt:17-25`):
```
⏺ Bash(sleep 15)
  ⎿  (No output)
⏺ DONE
⏺ Ran 2 stop hooks (ctrl+o to expand)
  ⎿  Stop hook error: Compute 12+31 and reply with just the number.
⏺ 43
```
(The "Stop hook error:" label is cosmetic/misleading — same finding as the
prior spike's C4: Claude Code renders any hook-triggered continuation under
that generic label, it is not a real error.)

**Loop behavior**: `stop_hook_active` was logged on every Stop firing via the
hook's own JSON input. All 3 trials show exactly `["False", "True"]` — the
natural end-of-turn Stop fires with `stop_hook_active: false`, the
hook-triggered continuation's Stop fires with `stop_hook_active: true`, and
because the queue file was already emptied on the first firing, no third
block happened (confirmed: exactly 2 Stop events per trial, never 3+). No
infinite loop was observed or risked. **Recommended guard**: consume the
pending-message queue atomically (clear-before-emit, as done here) rather
than relying solely on `stop_hook_active` — the flag is correctly reported by
Claude Code and matches docs, but the queue-emptiness check is the simpler
and more robust condition (works even if two hooks fire concurrently, and
doesn't require special-casing the second firing). We did not intentionally
test an *unguarded* hook that re-blocks every time (to see a real infinite
loop) — that risks runaway turns against a shared weekly quota with no
additional information value once the guard is shown to work; the
`stop_hook_active` field's documented purpose (skip re-blocking when true) is
confirmed present and correctly set, which is sufficient evidence for the
"how to avoid it" question.

## F5 — Model sensitivity (fleet's actual Claude model)

`grep model ~/.agend-terminal/fleet.yaml` (read-only, not
modified): agents on `backend: claude` use `model: claude-opus-5` and
`model: claude-opus-4-6[1m]`; the global default is `model: null`. Picked
`claude-opus-5` (the plain, non-long-context Claude model actually assigned
to a `backend: claude` agent) and repeated the best-performing recipe (F1+F2
combined framing, `proj-f5`), 2 trials as instructed ("keeping trials few").

**WORKS, 2/2.** `claude-opus-5` is a valid `--model` value in this account
(banner: "Opus 5 with xhigh effort · Claude Max"). Both trials answered
correctly immediately after Esc:
- f5-t1: 15+28 → `⏺ 43` (`captures/f5-t1.txt:21-23`)
- f5-t2: 26+17 → `⏺ 43` (model narrated "I'll run that now." before the Bash
  call, otherwise identical behavior to haiku)

No refusal/hesitation language in either trial. Framing generalizes to the
production model on this small sample.

## Summary table

| Variant | Verdict | Score |
|---|---|---|
| Baseline (no framing, Esc→channel) | DOESN'T | 0/3 |
| F1 (CLAUDE.md framing only) | WORKS | 3/3 |
| F2 alone (sender metadata in message, no F1) | WORKS | 3/3 |
| F1+F2 combined | WORKS | 3/3 |
| F3 (wait-for-idle-hook ordering after Esc) | DOESN'T (no such signal exists) / WORKS as fixed-delay proxy | 3/3 answered, signal never observed in 3/3 |
| F4 (busy case, Stop-hook block+reason delivery) | WORKS, no loops | 3/3 |
| F5 (claude-opus-5, F1+F2 recipe) | WORKS | 2/2 |

## Recommended v2 driver recipe

1. **Idle / immediately-after-interrupt delivery**: ship a short CLAUDE.md (or
   equivalent `--append-system-prompt`) stating that `agend`-channel messages
   are the user's own team/daemon relaying instructions, to be acted on
   directly, including right after an interrupt (F1). This alone is
   sufficient (3/3) and needs no per-message change.
   Optionally *also* prefix each message body with lightweight sender/task
   metadata (`from: ... · task ... · request: ...`, F2) — it is independently
   sufficient (3/3) and gives the daemon a way to convey provenance without
   depending on project-level config being present in every workspace; cheap
   to keep both since combining them cost nothing and added no measured
   downside.
2. **Busy case (b)**: do not try to inject via the channel mid-turn — queue
   the message externally and deliver it through the Stop hook's
   `{"decision":"block","reason":"<message>"}` at the turn's natural end
   (F4). Consume the queued message before emitting (truncate-then-block) so
   the hook-triggered follow-up Stop (which reports `stop_hook_active:true`)
   finds nothing to re-deliver — this is a correctness requirement, not an
   optimization, or repeated messages could double-fire.
3. **Ordering after Esc**: no idle/Stop signal exists after a bare Esc in
   this Claude Code version — don't build v2 logic that waits for one. Once
   source framing (step 1) is present, sending immediately after Esc is as
   reliable as waiting (F3, both 3/3) — the previous PARTIAL result was a
   framing problem, not a race condition.
4. Framing generalizes from Haiku to the production `claude-opus-5` agent
   model on the small F5 sample (2/2); no model-specific tuning observed to
   be necessary, though only 2 trials were run there per the cost budget.

## Nothing BLOCKED in this spike.

All five sub-questions (F1-F5) produced a concrete, evidenced verdict; no
step required an unavailable capability.

## Cleanup performed

- `tmux -L spike-claude-f kill-server` — confirmed after: "no server running
  on <tmp>/tmux-<uid>/spike-claude-f".
- `ps aux` grep for `claude --dangerously-skip-permissions --model
  (haiku|claude-opus-5)` and `channel_server.py`: zero matches after cleanup.
- Removed `<tmp>/scf-{base,f1,f2a,f2b,f4,f5}.sock` and their `-server.log`
  companions (channel-server control sockets; kept out of the scratch dir
  for the same AF_UNIX 104-byte `sun_path` length reason the prior spike
  documented — the scratch path is 143+ bytes, over the macOS limit).
- Verified `git -C <v1-repo> status --short`
  still equals exactly ` M .gitignore`, `?? AGENTS.md`,
  `?? ANALYSIS_AND_IMPROVEMENT_PLAN.md`, `?? docs/ARCHITECTURE-v2.md` (no
  stray files, no fifo/socket left in the repo).
- Left this scratch directory's own artifacts in place (`common/`, `proj-*/`,
  `logs/`, `captures/`, `queue/`) as evidence for this report.
