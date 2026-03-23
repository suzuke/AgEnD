# Topic Command UX Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the manual Topic creation + directory browser flow with `/open` and `/new` commands in the General topic.

**Architecture:** Add a `handleGeneralCommand()` router in FleetManager that intercepts messages from the General topic and dispatches to `handleOpenCommand()` or `handleNewCommand()`. Extract the duplicated bind sequence (config write + start daemon + IPC connect) into a shared `bindAndStart()` method. Remove the old `pendingBindings` state machine and directory browser.

**Tech Stack:** TypeScript, grammY (Telegram Bot API), vitest

**Spec:** `docs/superpowers/specs/2026-03-23-topic-command-ux-design.md`

---

### Task 1: Extract shared `createForumTopic()` method

**Files:**
- Modify: `src/fleet-manager.ts:591-628` (autoCreateTopics)

The existing `autoCreateTopics()` has inline fetch logic for creating Telegram topics. Extract it into a reusable private method.

- [ ] **Step 1: Write the failing test**

```typescript
// tests/fleet-manager.test.ts — add to existing describe block
it("createForumTopic returns topic ID on success", async () => {
  const fm = new FleetManager(tmpDir);
  // We can't easily test the Telegram API call without mocking fetch,
  // so this test verifies the method exists and has the right signature.
  // Integration testing will be done manually.
  expect(typeof (fm as any).createForumTopic).toBe("function");
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: FAIL — `createForumTopic` is not a function (doesn't exist yet)

- [ ] **Step 3: Extract `createForumTopic()` from `autoCreateTopics()`**

In `src/fleet-manager.ts`, add this private method and refactor `autoCreateTopics()` to use it:

```typescript
/** Create a Telegram Forum Topic. Returns the message_thread_id. */
private async createForumTopic(topicName: string): Promise<number> {
  const groupId = this.fleetConfig?.channel?.group_id;
  const botTokenEnv = this.fleetConfig?.channel?.bot_token_env;
  if (!groupId || !botTokenEnv) throw new Error("No group_id or bot_token configured");
  const botToken = process.env[botTokenEnv];
  if (!botToken) throw new Error(`Bot token env var ${botTokenEnv} not set`);

  const res = await fetch(
    `https://api.telegram.org/bot${botToken}/createForumTopic`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ chat_id: groupId, name: topicName }),
    },
  );
  const data = await res.json() as { ok: boolean; result?: { message_thread_id: number }; description?: string };
  if (!data.ok || !data.result) {
    throw new Error(`createForumTopic failed: ${data.description ?? "unknown error"}`);
  }
  return data.result.message_thread_id;
}
```

Then update `autoCreateTopics()` to call `this.createForumTopic(topicName)` instead of the inline fetch.

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/fleet-manager.ts tests/fleet-manager.test.ts
git commit -m "refactor: extract createForumTopic into shared method"
```

---

### Task 2: Extract shared `bindAndStart()` method

**Files:**
- Modify: `src/fleet-manager.ts:738-766` (bind sequence in handleDirectorySelection)
- Modify: `src/fleet-manager.ts:848-868` (bind sequence in handleNewProjectName)

The bind sequence (create instance config → save → update routing table → allocate ports → start → wait → connect IPC) is duplicated. Extract into a shared method.

- [ ] **Step 1: Write the failing test**

```typescript
it("bindAndStart method exists", () => {
  const fm = new FleetManager(tmpDir);
  expect(typeof (fm as any).bindAndStart).toBe("function");
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: FAIL

- [ ] **Step 3: Extract `bindAndStart()` method**

In `src/fleet-manager.ts`, add:

```typescript
/**
 * Create instance config, save fleet.yaml, start daemon, connect IPC.
 * Returns the generated instance name.
 */
private async bindAndStart(dirPath: string, topicId: number): Promise<string> {
  if (!this.fleetConfig) throw new Error("Fleet config not loaded");

  const instanceName = `${sanitizeInstanceName(basename(dirPath))}-t${topicId}`;

  this.fleetConfig.instances[instanceName] = {
    working_directory: dirPath,
    topic_id: topicId,
    restart_policy: this.fleetConfig.defaults.restart_policy ?? DEFAULT_INSTANCE_CONFIG.restart_policy,
    context_guardian: this.fleetConfig.defaults.context_guardian ?? DEFAULT_INSTANCE_CONFIG.context_guardian,
    memory: this.fleetConfig.defaults.memory ?? DEFAULT_INSTANCE_CONFIG.memory,
    log_level: this.fleetConfig.defaults.log_level ?? DEFAULT_INSTANCE_CONFIG.log_level,
  };

  this.saveFleetConfig();
  this.routingTable.set(topicId, instanceName);

  const ports = this.allocatePorts(this.fleetConfig.instances);
  await this.startInstance(instanceName, this.fleetConfig.instances[instanceName], ports[instanceName], true);

  await new Promise(r => setTimeout(r, 5000));
  await this.connectIpcToInstance(instanceName);

  this.logger.info({ instanceName, topicId }, "Topic bound and started");
  return instanceName;
}
```

Then update `handleDirectorySelection()` (lines 738-766) and `handleNewProjectName()` (lines 848-868) to call `this.bindAndStart(dirPath, threadId)` instead of the inline sequence.

- [ ] **Step 4: Run tests**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: PASS (existing tests still pass)

- [ ] **Step 5: Commit**

```bash
git add src/fleet-manager.ts tests/fleet-manager.test.ts
git commit -m "refactor: extract bindAndStart into shared method"
```

---

### Task 3: Add `currentOpenSession` field and `handleGeneralCommand()` router

**Files:**
- Modify: `src/fleet-manager.ts:34-45` (class fields)
- Modify: `src/fleet-manager.ts:321-326` (handleInboundMessage routing)

- [ ] **Step 1: Add the `currentOpenSession` field**

In `src/fleet-manager.ts` class fields (around line 41), add:

```typescript
private currentOpenSession: { id: string; paths: string[] } | null = null;
```

- [ ] **Step 2: Add `isGeneralTopic()` helper**

```typescript
/** Detect if threadId represents the General topic (undefined = General in our adapter) */
private isGeneralTopic(threadId: number | undefined): boolean {
  return threadId == null;
}
```

- [ ] **Step 3: Add `handleGeneralCommand()` router**

```typescript
/** Parse and dispatch commands from the General topic */
private async handleGeneralCommand(msg: InboundMessage): Promise<void> {
  const text = msg.text?.trim();
  if (!text) return;

  if (text === "/open" || text === "/open@" || text.startsWith("/open ") || text.startsWith("/open@")) {
    // Extract keyword: remove /open or /open@botname, take the rest
    const keyword = text.replace(/^\/open(@\S+)?\s*/, "").trim();
    await this.handleOpenCommand(msg, keyword || undefined);
    return;
  }

  if (text === "/new" || text === "/new@" || text.startsWith("/new ") || text.startsWith("/new@")) {
    const name = text.replace(/^\/new(@\S+)?\s*/, "").trim();
    await this.handleNewCommand(msg, name || undefined);
    return;
  }

  // Not a command — ignore silently
}
```

- [ ] **Step 4: Update `handleInboundMessage()` routing**

In `src/fleet-manager.ts:321-326`, change:

```typescript
// OLD:
const threadId = msg.threadId ? parseInt(msg.threadId, 10) : undefined;
if (threadId == null) {
  this.logger.warn({ chatId: msg.chatId }, "Message without threadId — ignoring in topic mode");
  return;
}
```

To:

```typescript
// NEW:
const threadId = msg.threadId ? parseInt(msg.threadId, 10) : undefined;
if (this.isGeneralTopic(threadId)) {
  await this.handleGeneralCommand(msg);
  return;
}
```

- [ ] **Step 5: Run tests**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/fleet-manager.ts
git commit -m "feat: add General topic command router for /open and /new"
```

---

### Task 4: Implement `handleOpenCommand()`

**Files:**
- Modify: `src/fleet-manager.ts`

This is the core `/open` implementation. It lists unbound directories, handles keyword matching, and builds inline keyboards.

- [ ] **Step 1: Write the test for `listUnboundDirectories()`**

```typescript
it("listUnboundDirectories excludes already-bound dirs", () => {
  const fm = new FleetManager(tmpDir);
  const configPath = join(tmpDir, "fleet.yaml");

  // Create some project dirs
  const projectRoot = join(tmpDir, "projects");
  mkdirSync(join(projectRoot, "proj-a"), { recursive: true });
  mkdirSync(join(projectRoot, "proj-b"), { recursive: true });
  mkdirSync(join(projectRoot, "proj-c"), { recursive: true });

  writeFileSync(configPath, `
project_roots:
  - ${projectRoot}
instances:
  proj-a-t42:
    working_directory: ${join(projectRoot, "proj-a")}
    topic_id: 42
`);
  fm.loadConfig(configPath);
  fm.buildRoutingTable();

  const unbound = (fm as any).listUnboundDirectories();
  const names = unbound.map((d: string) => basename(d));
  expect(names).toContain("proj-b");
  expect(names).toContain("proj-c");
  expect(names).not.toContain("proj-a");
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: FAIL

- [ ] **Step 3: Implement `listUnboundDirectories()`**

```typescript
/** List directories from project_roots that are not already bound to an instance */
private listUnboundDirectories(): string[] {
  const boundDirs = new Set(
    Object.values(this.fleetConfig?.instances ?? {}).map(i => i.working_directory),
  );
  return this.listProjectDirectories().filter(d => !boundDirs.has(d));
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: PASS

- [ ] **Step 5: Write test for keyword matching logic**

```typescript
it("filterDirectories: exact match wins over substring", () => {
  const dirs = ["/p/myapp", "/p/myapp-v2", "/p/other"];
  const fm = new FleetManager(tmpDir);

  // Exact match
  const exact = (fm as any).filterDirectories(dirs, "myapp");
  expect(exact).toEqual({ type: "exact", path: "/p/myapp" });

  // Substring only
  const sub = (fm as any).filterDirectories(dirs, "app");
  expect(sub).toEqual({ type: "multiple", paths: ["/p/myapp", "/p/myapp-v2"] });

  // No match
  const none = (fm as any).filterDirectories(dirs, "zzz");
  expect(none).toEqual({ type: "none" });
});
```

- [ ] **Step 6: Implement `filterDirectories()`**

```typescript
/** Match directories by keyword. Exact basename match wins over substring. */
private filterDirectories(
  dirs: string[],
  keyword: string,
): { type: "exact"; path: string } | { type: "multiple"; paths: string[] } | { type: "none" } {
  const kw = keyword.toLowerCase();

  // Check for exact basename match first
  const exactMatches = dirs.filter(d => basename(d).toLowerCase() === kw);
  if (exactMatches.length === 1) {
    return { type: "exact", path: exactMatches[0] };
  }

  // Fall back to substring match
  const subMatches = dirs.filter(d => basename(d).toLowerCase().includes(kw));
  if (subMatches.length === 0) return { type: "none" };
  if (subMatches.length === 1) return { type: "exact", path: subMatches[0] };
  return { type: "multiple", paths: subMatches };
}
```

- [ ] **Step 7: Run tests**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: PASS

- [ ] **Step 8: Implement `handleOpenCommand()`**

```typescript
/** Handle /open command — list or search unbound directories */
private async handleOpenCommand(msg: InboundMessage, keyword?: string): Promise<void> {
  if (!this.adapter || !this.fleetConfig) return;

  const roots = this.getProjectRoots();
  if (roots.length === 0 || (roots.length === 1 && roots[0] === homedir())) {
    await this.adapter.sendText(msg.chatId, "No project roots configured. Run `ccd init` to set up.");
    return;
  }

  const dirs = this.listUnboundDirectories();

  if (keyword) {
    const result = this.filterDirectories(dirs, keyword);
    if (result.type === "none") {
      await this.adapter.sendText(msg.chatId, `No projects found matching "${keyword}".`);
      return;
    }
    if (result.type === "exact") {
      await this.openBindProject(msg.chatId, result.path);
      return;
    }
    // Multiple matches — show keyboard
    await this.sendOpenKeyboard(msg.chatId, result.paths, 0);
    return;
  }

  // No keyword — show full list
  if (dirs.length === 0) {
    await this.adapter.sendText(msg.chatId, "All projects are already bound to topics.");
    return;
  }
  await this.sendOpenKeyboard(msg.chatId, dirs, 0);
}

/** Send paginated inline keyboard for /open */
private async sendOpenKeyboard(chatId: string, dirs: string[], page: number): Promise<void> {
  const sessionId = Math.random().toString(16).slice(2, 10); // 8 hex chars
  this.currentOpenSession = { id: sessionId, paths: dirs };

  const PAGE_SIZE = 5;
  const pageStart = page * PAGE_SIZE;
  const pageDirs = dirs.slice(pageStart, pageStart + PAGE_SIZE);

  const { InlineKeyboard } = await import("grammy");
  const keyboard = new InlineKeyboard();

  for (let i = 0; i < pageDirs.length; i++) {
    const idx = pageStart + i;
    keyboard.text(`📁 ${basename(pageDirs[i])}`, `cmd_open:${sessionId}:${idx}`).row();
  }

  // Pagination
  const hasMore = pageStart + PAGE_SIZE < dirs.length;
  if (page > 0 || hasMore) {
    if (page > 0) keyboard.text("⬅️ Prev", `cmd_open:${sessionId}:page:${page - 1}`);
    if (hasMore) keyboard.text("➡️ Next", `cmd_open:${sessionId}:page:${page + 1}`);
    keyboard.row();
  }

  keyboard.text("❌ Cancel", `cmd_open:${sessionId}:cancel`).row();

  const headerText = page === 0
    ? "📂 Select a project:"
    : `📂 Projects (page ${page + 1}):`;

  const tgAdapter = this.adapter as TelegramAdapter;
  // Intentionally no threadId — keyboard is sent to the General topic
  await tgAdapter.sendTextWithKeyboard(chatId, headerText, keyboard);
}

/** Create topic and bind a project directory (triggered by /open exact match or keyboard selection) */
private async openBindProject(chatId: string, dirPath: string): Promise<void> {
  if (!this.adapter || !this.fleetConfig) return;

  let topicId: number | undefined;
  try {
    const topicName = basename(dirPath);
    topicId = await this.createForumTopic(topicName);
    const instanceName = await this.bindAndStart(dirPath, topicId);

    const tgAdapter = this.adapter as TelegramAdapter;
    await tgAdapter.sendText(
      chatId,
      `✅ Bound to: ${dirPath}\nInstance: ${instanceName}`,
      { threadId: String(topicId) },
    );
  } catch (err) {
    // Rollback: remove partial instance config if bindAndStart failed after topic creation
    if (topicId != null) {
      const partialName = Object.entries(this.fleetConfig.instances)
        .find(([, cfg]) => cfg.topic_id === topicId)?.[0];
      if (partialName) {
        delete this.fleetConfig.instances[partialName];
        this.routingTable.delete(topicId);
        this.saveFleetConfig();
      }
    }
    await this.adapter.sendText(chatId, `❌ Failed to bind: ${(err as Error).message}`);
  }
}
```

- [ ] **Step 9: Run tests**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: PASS

- [ ] **Step 10: Commit**

```bash
git add src/fleet-manager.ts tests/fleet-manager.test.ts
git commit -m "feat: implement /open command with keyword search and pagination"
```

---

### Task 5: Implement `handleNewCommand()`

**Files:**
- Modify: `src/fleet-manager.ts`

- [ ] **Step 1: Write test for name validation**

```typescript
it("validateProjectName rejects invalid names", () => {
  const fm = new FleetManager(tmpDir);
  const validate = (name: string) => (fm as any).validateProjectName(name);
  expect(validate("my-project")).toBe(true);
  expect(validate("")).toBe(false);
  expect(validate("   ")).toBe(false);
  expect(validate("foo/bar")).toBe(false);
  expect(validate("..")).toBe(false);
  expect(validate("-flag")).toBe(false);
  expect(validate("ok-project")).toBe(true);
  expect(validate("中文專案")).toBe(true);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: FAIL

- [ ] **Step 3: Implement `validateProjectName()` and `handleNewCommand()`**

```typescript
/** Validate project name for /new command */
private validateProjectName(name: string): boolean {
  if (!name || !name.trim()) return false;
  if (name.includes("/") || name.includes("..")) return false;
  if (name.startsWith("-")) return false;
  return true;
}

/** Handle /new command — create directory + git init + bind */
private async handleNewCommand(msg: InboundMessage, name?: string): Promise<void> {
  if (!this.adapter || !this.fleetConfig) return;

  if (!name) {
    await this.adapter.sendText(msg.chatId, "Usage: `/new <project-name>`", { format: "markdown" });
    return;
  }

  if (!this.validateProjectName(name)) {
    await this.adapter.sendText(msg.chatId, "Invalid project name. Avoid `/`, `..`, leading `-`, and whitespace-only names.");
    return;
  }

  const roots = this.getProjectRoots();
  if (roots.length === 0 || (roots.length === 1 && roots[0] === homedir())) {
    await this.adapter.sendText(msg.chatId, "No project roots configured. Run `ccd init` to set up.");
    return;
  }

  const projectDir = join(roots[0], name);
  if (existsSync(projectDir)) {
    await this.adapter.sendText(msg.chatId, `Directory "${name}" already exists. Use \`/open ${name}\` instead.`);
    return;
  }

  try {
    // Create directory + git init in parallel with createForumTopic
    const [topicId] = await Promise.all([
      this.createForumTopic(name),
      (async () => {
        mkdirSync(projectDir, { recursive: true });
        try {
          const { execFile } = await import("node:child_process");
          const { promisify } = await import("node:util");
          const exec = promisify(execFile);
          await exec("git", ["init"], { cwd: projectDir });
        } catch {}
      })(),
    ]);

    const instanceName = await this.bindAndStart(projectDir, topicId);

    const tgAdapter = this.adapter as TelegramAdapter;
    await tgAdapter.sendText(
      msg.chatId,
      `✅ Bound to: ${projectDir}\nInstance: ${instanceName}`,
      { threadId: String(topicId) },
    );
  } catch (err) {
    // Rollback: remove created directory
    try {
      if (existsSync(projectDir)) rmSync(projectDir, { recursive: true, force: true });
    } catch {}
    // Rollback: remove partial instance config
    if (this.fleetConfig) {
      const partialName = Object.entries(this.fleetConfig.instances)
        .find(([, cfg]) => cfg.working_directory === projectDir)?.[0];
      if (partialName) {
        const tid = this.fleetConfig.instances[partialName].topic_id;
        delete this.fleetConfig.instances[partialName];
        if (tid != null) this.routingTable.delete(tid);
        this.saveFleetConfig();
      }
    }
    await this.adapter.sendText(msg.chatId, `❌ Failed: ${(err as Error).message}`);
  }
}
```

- [ ] **Step 4: Run tests**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/fleet-manager.ts tests/fleet-manager.test.ts
git commit -m "feat: implement /new command with validation and rollback"
```

---

### Task 6: Update callback query handler for `cmd_open:*`

**Files:**
- Modify: `src/fleet-manager.ts:257-259` (callback_query listener)

- [ ] **Step 1: Replace `handleDirectorySelection` dispatch with new handler**

Change the callback_query listener at line 257-259:

```typescript
// OLD:
this.adapter.on("callback_query", (data) => {
  this.handleDirectorySelection(data);
});

// NEW:
this.adapter.on("callback_query", (data: { callbackData: string; chatId: string; threadId?: string; messageId: string }) => {
  this.handleCallbackQuery(data);
});
```

- [ ] **Step 2: Implement `handleCallbackQuery()`**

```typescript
/** Dispatch callback queries by prefix */
private async handleCallbackQuery(data: { callbackData: string; chatId: string; threadId?: string; messageId: string }): Promise<void> {
  const { callbackData, chatId, messageId } = data;

  if (callbackData.startsWith("cmd_open:")) {
    await this.handleOpenCallback(callbackData, chatId, messageId);
    return;
  }

  // Legacy prefixes — can be removed once old keyboards expire
  // (approval callbacks are handled separately by the approval system)
}

/** Handle callback from /open inline keyboard */
private async handleOpenCallback(callbackData: string, chatId: string, messageId: string): Promise<void> {
  if (!this.adapter) return;

  // Format: cmd_open:<sessionId>:<action>
  const parts = callbackData.split(":");
  const sessionId = parts[1];

  // Validate session
  if (!this.currentOpenSession || this.currentOpenSession.id !== sessionId) {
    await this.adapter.editMessage(chatId, messageId, "This menu has expired. Use /open again.");
    return;
  }

  const action = parts[2];

  // Cancel
  if (action === "cancel") {
    this.currentOpenSession = null;
    await this.adapter.editMessage(chatId, messageId, "Cancelled.");
    return;
  }

  // Pagination: cmd_open:<sessionId>:page:<pageNum>
  if (action === "page") {
    const page = parseInt(parts[3], 10);
    await this.adapter.editMessage(chatId, messageId, "Loading...");
    await this.sendOpenKeyboard(chatId, this.currentOpenSession.paths, page);
    return;
  }

  // Directory selection: cmd_open:<sessionId>:<index>
  const index = parseInt(action, 10);
  if (isNaN(index) || index < 0 || index >= this.currentOpenSession.paths.length) {
    await this.adapter.editMessage(chatId, messageId, "Invalid selection.");
    return;
  }

  const dirPath = this.currentOpenSession.paths[index];
  this.currentOpenSession = null;
  await this.adapter.editMessage(chatId, messageId, `Binding to ${basename(dirPath)}...`);
  await this.openBindProject(chatId, dirPath);
}
```

- [ ] **Step 3: Run tests**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/fleet-manager.ts
git commit -m "feat: add callback query handler for /open keyboard"
```

---

### Task 7: Replace `handleUnboundTopic()` with redirect message

**Files:**
- Modify: `src/fleet-manager.ts:633-685`

- [ ] **Step 1: Replace `handleUnboundTopic()` body**

```typescript
/** Reply with redirect when message arrives in an unbound topic */
private async handleUnboundTopic(msg: InboundMessage, threadId: number): Promise<void> {
  if (!this.adapter) return;
  await this.adapter.sendText(
    msg.chatId,
    "Please use /open or /new in General to bind a project to a topic.",
    { threadId: String(threadId) },
  );
}
```

- [ ] **Step 2: Remove `pendingBindings` field and all references**

Remove from class fields (line 41):
```typescript
// DELETE:
private pendingBindings: Map<number, string> = new Map();
```

Remove from `handleInboundMessage()` (lines 330-334) — the checks for `awaiting_name` and `pendingBindings.has()`. After the routing table miss, just call `handleUnboundTopic()` directly:

```typescript
const instanceName = this.routingTable.get(threadId);
if (!instanceName) {
  this.handleUnboundTopic(msg, threadId);
  return;
}
```

- [ ] **Step 3: Remove old `handleDirectorySelection()` and `handleNewProjectName()`**

Delete the entire `handleDirectorySelection()` method (lines 688-767).
Delete the entire `handleNewProjectName()` method (lines 813-869).
Delete `getRecentlyBoundDirs()` method (lines 898-904) — no longer used.

- [ ] **Step 4: Evaluate `autoCreateTopics()`**

Check if `autoCreateTopics()` is still needed. With the new flow, every instance gets a `topic_id` at creation time via `/open` or `/new`. The only case where `autoCreateTopics()` helps is if someone manually adds an instance to `fleet.yaml` without a `topic_id`. Keep it for now as a safety net but add a comment noting it may be removed later.

- [ ] **Step 5: Run tests**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: PASS (existing tests don't depend on removed methods)

- [ ] **Step 6: Commit**

```bash
git add src/fleet-manager.ts
git commit -m "refactor: replace directory browser with redirect message, remove old binding flow"
```

---

### Task 8: Register bot commands on startup

**Files:**
- Modify: `src/fleet-manager.ts` (in `startSharedAdapter()` or `startAll()`)

- [ ] **Step 1: Add `registerBotCommands()` method**

```typescript
/** Register /open and /new in Telegram command menu */
private async registerBotCommands(): Promise<void> {
  const groupId = this.fleetConfig?.channel?.group_id;
  const botTokenEnv = this.fleetConfig?.channel?.bot_token_env;
  if (!groupId || !botTokenEnv) return;
  const botToken = process.env[botTokenEnv];
  if (!botToken) return;

  try {
    await fetch(
      `https://api.telegram.org/bot${botToken}/setMyCommands`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          commands: [
            { command: "open", description: "Open an existing project" },
            { command: "new", description: "Create a new project" },
          ],
          scope: { type: "chat", chat_id: groupId },
        }),
      },
    );
    this.logger.info("Registered bot commands: /open, /new");
  } catch (err) {
    this.logger.warn({ err }, "Failed to register bot commands (non-fatal)");
  }
}
```

- [ ] **Step 2: Call it during startup**

In `startSharedAdapter()`, after the adapter is created and before `adapter.start()`, add:

```typescript
await this.registerBotCommands();
```

- [ ] **Step 3: Run tests**

Run: `npx vitest run tests/fleet-manager.test.ts`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/fleet-manager.ts
git commit -m "feat: register /open and /new bot commands on startup"
```

---

### Task 9: Manual integration test

**Files:** None (manual testing)

- [ ] **Step 1: Build the project**

Run: `npm run build`
Expected: No TypeScript errors

- [ ] **Step 2: Start the fleet**

Run: `ccd fleet start`
Expected: Fleet starts, bot commands are registered (check logs)

- [ ] **Step 3: Test `/open` in General topic**

In the Telegram group's General topic, send `/open`.
Expected: Inline keyboard with unbound project directories appears.

- [ ] **Step 4: Test `/open <keyword>`**

Send `/open chan` (or partial name of a known project).
Expected: If one exact match → auto-creates topic + binds. If multiple → shows keyboard.

- [ ] **Step 5: Test `/new testproject`**

Send `/new testproject` in General topic.
Expected: Creates directory, creates topic, binds, sends confirmation in new topic.

- [ ] **Step 6: Test unbound topic redirect**

Manually create a topic in Telegram, send a message in it.
Expected: Reply says "Please use /open or /new in General to bind a project to a topic."

- [ ] **Step 7: Test stale keyboard**

Send `/open`, then send `/open` again (invalidating first keyboard). Tap a button on the first keyboard.
Expected: "This menu has expired. Use /open again."

- [ ] **Step 8: Clean up test data**

Delete test topics and remove test instances from `fleet.yaml`.
