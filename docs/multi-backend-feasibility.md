# Multi-Backend Feasibility Study

Date: 2026-03-26

## Summary

Evaluated whether CCD's `CliBackend` abstraction can support non-Claude Code CLI backends (OpenCode, Gemini CLI, Codex CLI). **Conclusion: deferred** — the core inbound push mechanism (`notifications/claude/channel`) is Claude Code proprietary with no standard MCP equivalent.

## Candidates Evaluated

### OpenCode (anomalyco/opencode)

- **Language:** TypeScript (Effect-TS)
- **MCP support:** stdio, SSE, streamable-http — all three transports
- **Config format:** `opencode.json` with `mcp` key, `type: "local"`, `command` as array, `environment` (not `env`)
- **Non-interactive mode:** `opencode run "..."`
- **Session resume:** `--continue` / `--session <id>`
- **Multi-model:** Anthropic, OpenAI, Google, Groq, OpenRouter, local models
- **POC result:** All 10 ccd-channel tools loaded successfully

### Gemini CLI (google-gemini/gemini-cli)

- **MCP support:** stdio (via `gemini mcp add`)
- **Config format:** `.gemini/settings.json` with `mcpServers`, format nearly identical to Claude Code's `.mcp.json`
- **Non-interactive mode:** positional prompt with `-o text`
- **Session resume:** `--resume`
- **POC result:** All 10 ccd-channel tools loaded successfully

### Codex CLI (OpenAI)

- **MCP support:** None
- **Conclusion:** Cannot integrate — no MCP support at all

## The Blocker: Inbound Push

CCD's architecture relies on two MCP communication paths:

1. **Outbound (AI → Telegram):** Standard MCP tools (reply, react, etc.) — works universally
2. **Inbound (Telegram → AI):** `notifications/claude/channel` — Claude Code proprietary

The inbound path is how Telegram messages get pushed into the AI's conversation. Without it, the AI can send messages but never receives them.

### MCP Spec Analysis

Exhaustive review of MCP spec (2025-11-25) for server-to-client push alternatives:

| Mechanism | Pushes to AI conversation? | Unsolicited? | Client support |
|-----------|---------------------------|--------------|----------------|
| `notifications/claude/channel` | Yes | Yes | Claude Code only |
| `notifications/message` (logging) | No (displayed as logs) | Yes | Broad |
| `notifications/resources/updated` | No (client must re-read) | Yes | Very limited |
| `sampling/createMessage` | No (asks LLM to complete) | No (spec: must nest in tool call) | VS Code, Glama only |
| `elicitation/create` | No (gathers user input) | No (same constraint) | VS Code, Cursor, Glama |
| Polling tool (`check_messages`) | Yes | No (AI must call it) | Universal |

**No standard MCP mechanism can replace `notifications/claude/channel`.**

### Polling Tool Workaround

The only cross-client fallback is a `check_messages` tool that the AI calls periodically. Drawbacks:
- Passive — AI must be instructed to poll
- Latency — messages sit until next poll
- Token waste — empty polls consume context

Not pursued due to poor UX compared to native push.

## Architecture Readiness

The `CliBackend` interface (`src/backend/types.ts`) is already well-designed for multi-backend support:

```typescript
interface CliBackend {
  buildCommand(config: CliBackendConfig): string;
  writeConfig(config: CliBackendConfig): void;
  getContextUsage(): number | null;
  getSessionId(): string | null;
  postLaunch?(tmux: TmuxManager, windowId: string): Promise<void>;
  cleanup?(config: CliBackendConfig): void;
}
```

Adding a new backend is ~1-2 hours of implementation. The factory pattern in `src/backend/factory.ts` makes registration trivial. When a standard push mechanism emerges, integration will be straightforward.

## Decision

**Deferred.** Focus remains on Claude Code. Revisit when:
- MCP spec adds a standard server-to-client push/channel mechanism
- OpenCode or Gemini CLI implement `notifications/claude/channel` compatibility
- A polling-based UX becomes acceptable for a specific use case
