# Design: 3 UX Pain Points Fix (v2 — post-review)

## Fix 1: Instructions Change Detection → Force New Session

### Refined Approach

**Key ordering insight from review**: `buildBackendConfig()` reads `this.skipResume` at the top of `trySpawn()`. Hash comparison must happen BEFORE `buildBackendConfig()`, not after `writeConfig()`.

**Best placement**: Inside `spawnClaudeWindow()`, before `trySpawn()`. This is the single entry point for all spawns (initial, crash respawn, context rotation).

**Claude Code exemption**: Claude Code uses `--append-system-prompt-file` which is re-read on every resume. No need to kill its session for instruction changes. Add `instructionsReloadedOnResume` property to `CliBackend` interface.

**Implementation**:

1. Add to `CliBackend` interface (`src/backend/types.ts`):
```typescript
/** Whether this backend re-reads instruction files on --resume. If true, skip hash-based force-new-session. */
readonly instructionsReloadedOnResume?: boolean;
```

2. Set `instructionsReloadedOnResume = true` in `ClaudeCodeBackend` only.

3. In `spawnClaudeWindow()`, before `trySpawn()`:
```typescript
// Detect instructions change → force new session (skip for backends that re-read on resume)
if (!this.skipResume && !this.backend!.instructionsReloadedOnResume) {
  const newInstructions = buildFleetInstructions({
    instanceName: this.name,
    workingDirectory: this.config.working_directory,
    displayName: this.config.display_name,
    description: this.config.description,
    customPrompt: /* resolve systemPrompt same as buildBackendConfig */,
    workflow: /* resolve workflow same as buildBackendConfig */,
    decisions: process.env.AGEND_DECISIONS ? JSON.parse(process.env.AGEND_DECISIONS) : undefined,
  });
  const hashFile = join(this.instanceDir, "instructions-hash");
  const newHash = createHash("md5").update(newInstructions).digest("hex");
  let oldHash = "";
  try { oldHash = readFileSync(hashFile, "utf-8").trim(); } catch {}
  writeFileSync(hashFile, newHash);
  if (oldHash && newHash !== oldHash) {
    this.logger.info("Instructions changed — forcing new session");
    this.skipResume = true;
  }
}
```

**Problem**: This duplicates the instruction-building logic from `buildBackendConfig()`. 

**Cleaner alternative**: Don't rebuild instructions. Instead, compare the hash of what `writeConfig()` will write against what's already on disk. Each backend already writes instruction files:
- Claude Code: `fleet-instructions.md` in instanceDir
- Codex: AGENTS.md marker block in workDir
- Gemini: GEMINI.md marker block in workDir
- Kiro: `.kiro/steering/agend-{name}.md` in workDir

So the simplest approach: **read the existing instruction file before `writeConfig()`, compare with the new instructions from `buildBackendConfig()`, set `skipResume` if different**.

**Final refined implementation** — split `trySpawn()` into two phases:

```typescript
private async trySpawn(): Promise<boolean> {
  const backendConfig = this.buildBackendConfig();
  
  // Detect instructions change → force new session
  // (skip for backends that re-read instructions on resume)
  if (!backendConfig.skipResume && !this.backend!.instructionsReloadedOnResume && backendConfig.instructions) {
    const hashFile = join(this.instanceDir, "instructions-hash");
    const newHash = createHash("md5").update(backendConfig.instructions).digest("hex");
    let oldHash = "";
    try { oldHash = readFileSync(hashFile, "utf-8").trim(); } catch {}
    writeFileSync(hashFile, newHash);
    if (oldHash && newHash !== oldHash) {
      this.logger.info("Instructions changed — forcing new session");
      backendConfig.skipResume = true;
    }
  }
  
  this.backend!.writeConfig(backendConfig);
  // ... rest unchanged
}
```

Wait — `backendConfig.skipResume` is set from `this.skipResume` in `buildBackendConfig()`. Mutating `backendConfig.skipResume` after build is fine because `buildCommand()` reads from the config object, not from `this.skipResume`. Let me verify...

Yes: `buildCommand(config: CliBackendConfig)` reads `config.skipResume`. So mutating `backendConfig.skipResume = true` after `buildBackendConfig()` but before `buildCommand()` (which is called inside `trySpawn` after `writeConfig`) **works correctly**.

This is the cleanest approach:
- No duplication of instruction-building logic
- Hash comparison happens after `buildBackendConfig()` (has instructions) but before `buildCommand()` (reads skipResume)
- `backendConfig` is a local object, safe to mutate
- Only ~8 lines added to `trySpawn()`

### Edge Cases
- First spawn: no old hash → no comparison → normal resume ✅
- Instructions removed: empty instructions → skip hash check (`if backendConfig.instructions`) ✅
- Claude Code: `instructionsReloadedOnResume = true` → skip entirely ✅
- Crash retry in `spawnClaudeWindow()`: second `trySpawn()` call has `this.skipResume = true` already → `backendConfig.skipResume` is already true → hash check skipped ✅

---

## Fix 2: Single Instance Restart Reloads Config

### Refined Approach

**Review finding**: Two endpoints need fixing:
1. `fleet-manager.ts` `/restart/:name` (line 2046)
2. `web-api.ts` `/ui/restart/:name` (line 272)

**Reviewer suggestion**: Add `restartInstance(name)` method to FleetManager to avoid duplication.

**Implementation**:

1. Add method to FleetManager:
```typescript
async restartSingleInstance(name: string): Promise<void> {
  // Reload config to pick up fleet.yaml changes
  if (this.configPath) {
    this.loadConfig(this.configPath);
    this.routing.rebuild(this.fleetConfig!);
  }
  const config = this.fleetConfig?.instances[name];
  if (!config) throw new Error(`Instance not found: ${name}`);
  await this.stopInstance(name);
  const topicMode = this.fleetConfig?.channel?.mode === "topic";
  await this.startInstance(name, config, topicMode ?? false);
}
```

2. Add to `WebApiContext` interface:
```typescript
restartSingleInstance(name: string): Promise<void>;
```

3. Both endpoints call `restartSingleInstance(name)` instead of inline stop+start.

### Edge Cases
- fleet.yaml syntax error: `loadConfig()` throws → catch in caller, return 500 with error message ✅
- Instance removed from fleet.yaml between request and restart: `config` is null → throw "not found" ✅
- Concurrent restarts: `loadConfig` is sync, second call overwrites first, no corruption ✅

---

## Fix 3: Web UI + MCP Schema Missing Fields

### Web UI (`src/ui/dashboard.html`)

Add to `showCreateInstance()` form, after the Branch field:
- **Model**: `<input>` with placeholder "e.g. sonnet, opus, gemini-2.5-pro"
- **System Prompt**: `<textarea>` (multi-line)
- **Tags**: `<input>` with placeholder "comma-separated, e.g. dev, review"

Submit handler additions:
```javascript
if (v("ci-model")) body.model = v("ci-model");
if (v("ci-prompt")) body.systemPrompt = v("ci-prompt");
const tags = v("ci-tags"); if (tags) body.tags = tags.split(",").map(t => t.trim()).filter(Boolean);
```

### MCP Tool Schema (`src/channel/mcp-tools.ts`)

Add to `create_instance` inputSchema properties:
```typescript
tags: {
  type: "array",
  items: { type: "string" },
  description: "Tags for categorization and filtering.",
},
workflow: {
  type: "string",
  description: "Workflow template. 'builtin' (default), 'false' to disable, or custom text.",
},
```

### No backend changes needed
`handleCreate()` already handles all these fields.

---

## Implementation Plan

| Step | File | Lines | Risk |
|------|------|-------|------|
| 1 | `src/backend/types.ts` | +1 (add `instructionsReloadedOnResume` to interface) | None |
| 2 | `src/backend/claude-code.ts` | +1 (set property) | None |
| 3 | `src/daemon.ts` trySpawn() | +8 (hash check) | Low — ordering verified |
| 4 | `src/fleet-manager.ts` | +10 (add `restartSingleInstance`, refactor HTTP handler) | Low |
| 5 | `src/web-api.ts` | +2 (use `restartSingleInstance`, add to interface) | Low |
| 6 | `src/ui/dashboard.html` | +6 (form fields + submit logic) | None — additive |
| 7 | `src/channel/mcp-tools.ts` | +8 (schema additions) | None — additive |

Total: ~36 lines of meaningful code changes.
