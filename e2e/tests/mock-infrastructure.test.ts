/**
 * E2E Mock Infrastructure Tests
 *
 * Verifies that the mock Telegram server, mock backend, and test utilities
 * work correctly before using them in real E2E tests.
 */
import { describe, it, expect, beforeAll, afterAll, beforeEach, afterEach } from "vitest";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdirSync, writeFileSync, rmSync, existsSync, readFileSync } from "node:fs";
import yaml from "js-yaml";
import { createTelegramMock, type TelegramMock } from "../mock-servers/telegram-mock.js";
import { waitFor, sleep } from "../mock-servers/shared.js";

const TELEGRAM_MOCK_PORT = 18443; // Use high port to avoid conflicts
const TEST_GROUP_ID = -1001234567890;
const TEST_USER_ID = 111222333;

let telegramMock: TelegramMock;
let testDir: string;

describe("Mock Infrastructure", () => {
  beforeAll(async () => {
    // Start mock Telegram server
    telegramMock = createTelegramMock({ port: TELEGRAM_MOCK_PORT });
    await telegramMock.start();
  });

  afterAll(async () => {
    await telegramMock.stop();
  });

  beforeEach(() => {
    telegramMock.reset();
    testDir = join(tmpdir(), `agend-e2e-${Date.now()}-${Math.random().toString(36).slice(2)}`);
    mkdirSync(testDir, { recursive: true });
    mkdirSync(join(testDir, "instances"), { recursive: true });
  });

  afterEach(() => {
    rmSync(testDir, { recursive: true, force: true });
  });

  it("T1: mock Telegram server responds to getMe", async () => {
    // Verify the mock Telegram API is working
    const res = await fetch(`http://localhost:${TELEGRAM_MOCK_PORT}/bot123:fake/getMe`);
    const data = await res.json() as { ok: boolean; result: { username: string } };

    expect(data.ok).toBe(true);
    expect(data.result.username).toBe("test_bot");
  });

  it("T1: mock Telegram server records sendMessage calls", async () => {
    const res = await fetch(`http://localhost:${TELEGRAM_MOCK_PORT}/bot123:fake/sendMessage`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        chat_id: TEST_GROUP_ID,
        text: "Hello from test",
        message_thread_id: 42,
      }),
    });
    const data = await res.json() as { ok: boolean; result: { message_id: number; text: string } };

    expect(data.ok).toBe(true);
    expect(data.result.message_id).toBeGreaterThan(0);

    const calls = telegramMock.getCallsFor("sendMessage");
    expect(calls).toHaveLength(1);
    expect(calls[0].params.text).toBe("Hello from test");
    expect(calls[0].params.chat_id).toBe(TEST_GROUP_ID);
  });

  it("T1: mock Telegram server delivers injected messages via getUpdates", async () => {
    // Inject a message
    telegramMock.injectMessage({
      text: "Hello bot",
      chatId: TEST_GROUP_ID,
      userId: TEST_USER_ID,
      username: "testuser",
      threadId: 42,
    });

    // Poll for updates (like grammy does)
    const res = await fetch(`http://localhost:${TELEGRAM_MOCK_PORT}/bot123:fake/getUpdates`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ timeout: 1 }),
    });
    const data = await res.json() as { ok: boolean; result: Array<{ update_id: number; message: { text: string } }> };

    expect(data.ok).toBe(true);
    expect(data.result).toHaveLength(1);
    expect(data.result[0].message.text).toBe("Hello bot");
  });

  it("T1: mock Telegram server long-polling returns empty on timeout", async () => {
    // No pending messages — should return empty after timeout
    const start = Date.now();
    const res = await fetch(`http://localhost:${TELEGRAM_MOCK_PORT}/bot123:fake/getUpdates`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ timeout: 1 }),
    });
    const elapsed = Date.now() - start;
    const data = await res.json() as { ok: boolean; result: unknown[] };

    expect(data.ok).toBe(true);
    expect(data.result).toHaveLength(0);
    // Should have waited ~1s (capped at 5s)
    expect(elapsed).toBeGreaterThanOrEqual(800);
  });

  it("T1: mock Telegram server handles createForumTopic", async () => {
    const res = await fetch(`http://localhost:${TELEGRAM_MOCK_PORT}/bot123:fake/createForumTopic`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        chat_id: TEST_GROUP_ID,
        name: "test-instance",
      }),
    });
    const data = await res.json() as { ok: boolean; result: { message_thread_id: number; name: string } };

    expect(data.ok).toBe(true);
    expect(data.result.message_thread_id).toBeGreaterThan(0);
    expect(data.result.name).toBe("test-instance");
  });

  it("T1: control API — send-message and get calls", async () => {
    // Use control API to inject message
    await fetch(`http://localhost:${TELEGRAM_MOCK_PORT}/control/send-message`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        text: "control message",
        chat_id: TEST_GROUP_ID,
        user_id: TEST_USER_ID,
      }),
    });

    // Verify via control API
    const callsRes = await fetch(`http://localhost:${TELEGRAM_MOCK_PORT}/control/calls`);
    const callsData = await callsRes.json() as { ok: boolean; result: unknown[] };
    // No Bot API calls yet (message injected directly to update queue)
    expect(callsData.ok).toBe(true);
  });

  it("T1: control API — reset clears all state", async () => {
    // Add some calls
    await fetch(`http://localhost:${TELEGRAM_MOCK_PORT}/bot123:fake/getMe`);
    expect(telegramMock.getCalls().length).toBeGreaterThan(0);

    // Reset
    telegramMock.reset();
    expect(telegramMock.getCalls()).toHaveLength(0);
  });

  it("T1: fleet.yaml config generates correctly with mock backend", () => {
    const config = {
      channel: {
        type: "telegram",
        mode: "topic",
        bot_token_env: "AGEND_BOT_TOKEN",
        group_id: TEST_GROUP_ID,
        telegram_api_root: `http://localhost:${TELEGRAM_MOCK_PORT}`,
        access: {
          mode: "locked",
          allowed_users: [TEST_USER_ID],
        },
      },
      defaults: {
        backend: "mock",
        model: "mock-model",
        tool_set: "standard",
      },
      instances: {
        alpha: {
          working_directory: join(testDir, "alpha"),
          display_name: "Alpha",
          tags: ["test"],
        },
        beta: {
          working_directory: join(testDir, "beta"),
          display_name: "Beta",
          tags: ["test"],
        },
      },
    };

    const yamlStr = yaml.dump(config);
    const configPath = join(testDir, "fleet.yaml");
    writeFileSync(configPath, yamlStr);

    // Verify it can be parsed back
    const parsed = yaml.load(readFileSync(configPath, "utf-8")) as Record<string, unknown>;
    expect(parsed.channel).toBeDefined();
    expect((parsed.defaults as Record<string, unknown>).backend).toBe("mock");
    expect(Object.keys(parsed.instances as Record<string, unknown>)).toHaveLength(2);
  });

  it("T1: mock backend is registered in factory", async () => {
    const { createBackend } = await import("../../src/backend/factory.js");

    const instanceDir = join(testDir, "instances", "test-instance");
    mkdirSync(instanceDir, { recursive: true });

    const backend = createBackend("mock", instanceDir);
    expect(backend.binaryName).toBe("node");
    expect(backend.getReadyPattern().source).toContain("MOCK_READY");

    // writeConfig should create statusline.json
    backend.writeConfig({
      workingDirectory: join(testDir, "work"),
      instanceDir,
      instanceName: "test-instance",
      mcpServers: {},
    });

    expect(existsSync(join(instanceDir, "statusline.json"))).toBe(true);
    expect(existsSync(join(instanceDir, "mcp-config.json"))).toBe(true);

    // Verify statusline format matches ClaudeCode format
    const statusline = JSON.parse(readFileSync(join(instanceDir, "statusline.json"), "utf-8"));
    expect(statusline.context_window).toHaveProperty("used_percentage");
    expect(statusline.context_window).toHaveProperty("context_window_size");

    // getContextUsage should read from statusline
    expect(backend.getContextUsage()).toBe(0);
  });
});
