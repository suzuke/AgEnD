# Screen fixtures

The files contain only matched prompt text, not complete terminal captures. Codex was
captured from the holder PTY. Claude's prompt lines were copied from the version-pinned
spike notes, so they establish matching text but not exact full-screen rendering or
layout.

| Fixture | Evidence |
|---|---|
| `codex-folder-access.txt` | PTY capture from Codex CLI 0.156.1; `docs/research/spike-codex.md`, S6 |
| `claude-workspace-trust.txt` | Prompt excerpt from Claude Code 2.1.281 spike notes; default selection is described in the notes, not screen text |
| `claude-mcp-trust.txt` | Prompt excerpt from Claude Code 2.1.281 spike notes |

The classifier currently implements startup trust prompts only. Usage limits,
permission, rate-limit, authentication, and context-full patterns are deferred
until holder captures provide exact screen text. Add a rule only with a
corresponding fixture and backend/version evidence.
