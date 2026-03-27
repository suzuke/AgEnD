# Feature Roadmap

Prioritized feature list with implementation plans. Updated 2026-03-28.

## Tier 1 — Immediate

### 1. Per-message permission escalation (! prefix)

**Goal:** Let users bypass permission prompts for a single message by prefixing with `!`. Solves the "agent stuck on permission prompt at 3am" problem.

**Changed files:**
- `src/daemon.ts` — handlePermissionRequest()
- `src/fleet-manager.ts` — handleInboundMessage()

**Steps:**
1. In fleet-manager `handleInboundMessage()` (~L548): detect `!` prefix on `msg.text`, strip it, set a flag `autoApprove = true` on the IPC fleet_inbound meta
2. In daemon `pushChannelMessage()`: store autoApprove flag as `this.autoApproveNext = true`
3. In daemon `handlePermissionRequest()` (~L598): if `this.autoApproveNext` is true, respond with `behavior: "allow"` immediately without forwarding to Telegram. Reset flag after first use (or after a timeout)
4. Add safety: autoApproveNext expires after 60 seconds or after N approvals (prevent runaway)

**Estimate:** 2-3 hours

**Testing:** Send `! rm test.txt` in Telegram → verify no permission prompt appears → verify the command executes → verify next message without `!` triggers normal permission flow

**Risks:**
- Must not persist across context rotations (flag is in-memory, resets on respawn)
- Consider limiting to 1 approval or adding a max count per `!` message

---

### 2. Topic icon + auto-archive

**Goal:** Visual feedback in Telegram — running instances get a colored icon, stopped instances get archived.

**Changed files:**
- `src/fleet-manager.ts` — startInstance(), stopInstance()
- `src/topic-commands.ts` — handleTopicDeleted(), bindAndStart()

**Steps:**
1. After `startInstance()` succeeds: call Telegram API `editForumTopic` to set icon_custom_emoji_id (green circle or similar)
2. After `stopInstance()` or when instance crashes: call `editForumTopic` to change icon (red/grey)
3. When instance is idle for extended period: call `closeForumTopic` to archive
4. When instance receives a message in archived topic: call `reopenForumTopic` before routing

Helper method in TelegramAdapter:
```typescript
async setTopicIcon(topicId: number, emojiId: string): Promise<void>;
async archiveTopic(topicId: number): Promise<void>;
async unarchiveTopic(topicId: number): Promise<void>;
```

**Estimate:** 3-4 hours

**Testing:** Start/stop instances → verify topic icon changes in Telegram. Archive an instance → send a message → verify it re-opens.

**Risks:**
- Telegram custom emoji IDs are opaque strings — need to find valid emoji IDs or use built-in icon colors
- Rate limiting on editForumTopic if many instances start/stop rapidly

---

### 3. Permission relay improvements

**Goal:** Better UX for permission prompts — show timeout countdown, support batch approve, allow "approve all for this tool".

**Changed files:**
- `src/channel/adapters/telegram.ts` — sendApproval()
- `src/daemon.ts` — handlePermissionRequest(), requestApprovalViaIpc()
- `src/fleet-manager.ts` — handleApprovalFromInstance()

**Steps:**
1. **Timeout display:** Edit the approval message every 30s to show remaining time: "⏱ 90s remaining" → "⏱ 60s" → etc. Use `editMessage` on the approval message.
2. **"Always allow" button:** Already exists in the interface (`approve_always`). Wire it to add the tool to instance's runtime allow list (in-memory Map). Future permission_requests for the same tool auto-approve.
3. **Tool name in notification:** Show the full tool name + input preview in the Telegram message so user can make informed decisions.
4. **Post-decision feedback:** After approve/deny, edit the message to show the decision (currently just removes buttons — add "Approved by @user" text).

**Estimate:** 1 day

**Testing:** Trigger permission prompt → verify countdown updates → click "Always Allow" → verify next same-tool call auto-approves → verify different tool still prompts

**Risks:**
- "Always allow" runtime list must reset on context rotation (security)
- Editing messages too frequently may hit Telegram rate limits (debounce to 30s)

---

## Tier 2 — Near-term

### 4. Model failover

**Goal:** When primary model hits rate limit, automatically switch to a fallback model on next context rotation.

**Changed files:**
- `src/types.ts` — InstanceConfig: add `model_failover?: string[]`
- `src/context-guardian.ts` — emit rate limit info with status_update
- `src/daemon.ts` — spawnClaudeWindow(): select model based on rate limit state
- `src/backend/claude-code.ts` — buildCommand(): use model parameter

**Steps:**
1. Add `model_failover` config: `["opus", "sonnet"]` — ordered preference list
2. In context-guardian `readAndCheck()`: already emits `status_update` with rate_limits. Add a `rate_limit_exceeded` event when 5h rate > 90%
3. In daemon: listen for `rate_limit_exceeded`, set `this.failoverModelIndex` to next model in list
4. In `spawnClaudeWindow()`: pass current model to buildCommand
5. In claude-code.ts `buildCommand()`: add `--model` flag if model is specified
6. On successful startup with failover model, notify Telegram: "Switched to sonnet due to rate limit"
7. Reset failover index on next successful rotation with primary model

**Estimate:** 1 day

**Testing:** Simulate rate limit > 90% → trigger rotation → verify Claude restarts with fallback model → verify notification sent → verify it switches back when rate limit recovers

**Risks:**
- Claude Code `--model` flag availability — need to verify it exists
- Failover should not persist across fleet restarts (use in-memory state)

---

### 5. Webhook notifications

**Goal:** Push events (task completion, errors, cost alerts) to external endpoints.

**Changed files:**
- `src/types.ts` — FleetDefaults: add `webhooks?: WebhookConfig[]`
- `src/fleet-manager.ts` — new WebhookEmitter class or method
- `src/event-log.ts` — hook into event recording

**Steps:**
1. Config in fleet.yaml:
   ```yaml
   defaults:
     webhooks:
       - url: https://hooks.slack.com/...
         events: ["rotation", "hang", "cost_warn", "schedule_done"]
       - url: https://custom.endpoint/ccd
         events: ["*"]
   ```
2. Create `src/webhook-emitter.ts`: simple class that takes event + payload → POST to matching URLs
3. In fleet-manager: after each eventLog.record(), also call webhookEmitter.emit()
4. Include retry logic: 1 retry after 5s on failure, then drop
5. Include basic auth support: `headers` field in config

**Estimate:** 1 day

**Testing:** Configure a webhook to httpbin.org or requestbin → trigger events → verify POST arrives with correct payload

**Risks:**
- Webhook endpoints could be slow — use fire-and-forget with short timeout (5s)
- Don't block main event loop waiting for webhook response
- Sensitive data in payloads — cost info, instance names. Document what's sent.

---

### 6. Output secret filtering

**Goal:** Prevent Claude from accidentally sending API keys or tokens to Telegram.

**Changed files:**
- `src/channel/tool-router.ts` — add redaction middleware before sendText
- New: `src/channel/secret-filter.ts`

**Steps:**
1. Create `secret-filter.ts` with patterns:
   - Known prefixes: `sk-`, `ghp_`, `ghu_`, `AKIA`, `xoxb-`, `xoxp-`, `Bearer ey`
   - High-entropy strings: 32+ chars of `[A-Za-z0-9+/=]` with Shannon entropy > 4.5
   - .env format: `KEY=value` lines where key contains `TOKEN`, `SECRET`, `KEY`, `PASSWORD`
2. `redactSecrets(text: string): string` — replaces matches with `[REDACTED: <type>]`
3. In tool-router.ts `case "reply"`: run `redactSecrets(args.text)` before passing to adapter
4. Add allowlist in config: `secret_filter.allowlist: ["base64_ok_pattern"]`
5. Log redactions at warn level for audit

**Estimate:** 1-2 days

**Testing:** Send a message containing `sk-abc123...` → verify it appears as `[REDACTED: api_key]` in Telegram → verify normal text passes through unchanged → verify base64 encoded data is not over-redacted

**Risks:**
- **False positives are the main risk.** Long hex strings (git SHAs), base64 data, JWT tokens in examples could all be redacted. Need tuning.
- Start with only known-prefix patterns (low false positive rate), add entropy detection later

---

## Tier 3 — Improvements

### 7. Service message filtering

**Goal:** Filter out Telegram system events (topic rename, pin, member join) that waste Claude's context window.

**Changed files:**
- `src/channel/adapters/telegram.ts` — message handler

**Steps:**
1. In TelegramAdapter's Grammy message handler: check if `msg.text` is undefined/empty AND message has service properties (`new_chat_title`, `pinned_message`, `new_chat_members`, etc.)
2. If service message → skip emit, don't forward to daemon
3. Optionally log at debug level for visibility

**Estimate:** 1 hour

**Testing:** Rename a topic → verify no empty message reaches Claude. Send a real message → verify it still goes through.

**Risks:** None — purely additive filter with clear criteria.

---

### 8. Instance health endpoint

**Goal:** Simple HTTP endpoint for external monitoring (uptime robot, etc.).

**Changed files:**
- `src/fleet-manager.ts` — add HTTP server in startAll()

**Steps:**
1. In `startAll()`: start a minimal HTTP server on configurable port (default 9100)
2. `GET /health` → `200 { status: "ok", instances: N, uptime: Xs }`
3. `GET /status` → `200 { instances: [{ name, status, context_pct, cost_today }] }`
4. No authentication needed (localhost only by default). Add `bind: "127.0.0.1"` config.
5. Config:
   ```yaml
   defaults:
     health_endpoint:
       enabled: true
       port: 9100
       bind: "127.0.0.1"
   ```

**Estimate:** 3-4 hours

**Testing:** Start fleet → `curl localhost:9100/health` → verify JSON response → stop an instance → verify status reflects change

**Risks:**
- Keep it minimal — no express dependency, use Node.js built-in `http.createServer`
- Don't expose to network unless explicitly configured

---

### 9. Graceful handover improvements

**Goal:** More reliable context handover with structured template and validation.

**Changed files:**
- `src/daemon.ts` — handover prompt in guardian "request_handover" handler
- `src/context-guardian.ts` — handover validation

**Steps:**
1. Improve the handover prompt to request structured sections:
   ```
   Save handover state. Use this exact structure:
   ## Active Work
   ## Pending Decisions
   ## Key Context
   ```
2. After handover timeout: read `memory/handover.md`, validate it has the expected sections
3. If validation fails: retry once with a more explicit prompt
4. On new session startup: inject handover.md content into the first channel message as context
5. Track handover quality metrics in eventLog (sections present, word count)

**Estimate:** 3-4 hours

**Testing:** Trigger context rotation → verify handover.md has structured sections → verify new session receives handover context → verify retry works when first attempt fails

**Risks:**
- Claude might not follow the exact template — use fuzzy matching for section headers
- Don't block rotation on handover quality — proceed after 2 attempts regardless
