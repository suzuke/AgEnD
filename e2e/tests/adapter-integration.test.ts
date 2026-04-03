/**
 * E2E Test: TelegramAdapter ↔ Mock Telegram Server Integration
 *
 * Tests the real TelegramAdapter connecting to our mock Telegram server.
 * Verifies message flow in both directions:
 * - Inbound: mock server injects message → adapter emits event
 * - Outbound: adapter.sendText() → mock server records the call
 *
 * This is a "real" integration test — no mocks of grammy, it uses the
 * actual TelegramAdapter talking to the mock HTTP server.
 *
 * Note: imports from ../../src are valid because tests run in the same repo.
 * For VM-based E2E tests, the adapter would be tested via SSH + fleet.yaml.
 */
import { describe, it, expect, beforeAll, afterAll, beforeEach, afterEach } from "vitest";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdirSync, rmSync, existsSync } from "node:fs";
import { createTelegramMock, type TelegramMock } from "../mock-servers/telegram-mock.js";
import { waitFor } from "../mock-servers/shared.js";
import { TelegramAdapter } from "../../src/channel/adapters/telegram.js";
import { AccessManager } from "../../src/channel/access-manager.js";
import type { InboundMessage } from "../../src/channel/types.js";

const TELEGRAM_MOCK_PORT = 18445;
const TEST_GROUP_ID = -1001234567890;
const TEST_USER_ID = 111222333;

let telegramMock: TelegramMock;
let testDir: string;

/** Create a TelegramAdapter wired to the mock server. */
function createTestAdapter(allowedUsers: number[] = [TEST_USER_ID]): TelegramAdapter {
  const accessDir = join(testDir, "access");
  mkdirSync(accessDir, { recursive: true });
  const accessManager = new AccessManager(
    { mode: "locked", allowed_users: allowedUsers },
    join(accessDir, "access.json"),
  );
  return new TelegramAdapter({
    id: "test",
    botToken: "123456:FAKE_TOKEN",
    accessManager,
    inboxDir: join(testDir, "inbox"),
    apiRoot: `http://localhost:${TELEGRAM_MOCK_PORT}`,
  });
}

/** Start adapter and wait for grammy to complete its first poll cycle. */
async function startAndWaitReady(adapter: TelegramAdapter): Promise<void> {
  await adapter.start();
  await waitFor(
    () => telegramMock.getCallsFor("getUpdates").length > 0,
    { timeout: 5000, label: "grammy first poll" },
  );
}

describe("TelegramAdapter ↔ Mock Server", () => {
  beforeAll(async () => {
    telegramMock = createTelegramMock({ port: TELEGRAM_MOCK_PORT });
    await telegramMock.start();
  });

  afterAll(async () => {
    await telegramMock.stop();
  });

  beforeEach(() => {
    telegramMock.reset();
    testDir = join(tmpdir(), `agend-e2e-adapter-${Date.now()}-${Math.random().toString(36).slice(2)}`);
    mkdirSync(testDir, { recursive: true });
  });

  afterEach(() => {
    rmSync(testDir, { recursive: true, force: true });
  });

  it("T5: TelegramAdapter starts and connects to mock server", async () => {
    const adapter = createTestAdapter();
    await adapter.start();

    await waitFor(
      () => telegramMock.getCallsFor("getMe").length > 0,
      { timeout: 5000, label: "getMe call" },
    );
    expect(telegramMock.getCallsFor("getMe").length).toBeGreaterThan(0);

    await adapter.stop();
  });

  it("T5: adapter receives injected message from mock server", async () => {
    const adapter = createTestAdapter();
    const receivedMessages: InboundMessage[] = [];
    adapter.on("message", (msg: InboundMessage) => receivedMessages.push(msg));

    await startAndWaitReady(adapter);

    telegramMock.injectMessage({
      text: "Hello from E2E test!",
      chatId: TEST_GROUP_ID,
      userId: TEST_USER_ID,
      username: "testuser",
      threadId: 42,
    });

    await waitFor(
      () => receivedMessages.length > 0,
      { timeout: 10_000, label: "inbound message" },
    );

    expect(receivedMessages).toHaveLength(1);
    expect(receivedMessages[0].text).toBe("Hello from E2E test!");
    expect(receivedMessages[0].source).toBe("telegram");
    expect(receivedMessages[0].chatId).toBe(String(TEST_GROUP_ID));
    expect(receivedMessages[0].threadId).toBe("42");
    expect(receivedMessages[0].username).toBe("testuser");

    await adapter.stop();
  });

  it("T5: adapter ignores messages from non-allowed users", async () => {
    const adapter = createTestAdapter();
    const receivedMessages: InboundMessage[] = [];
    adapter.on("message", (msg: InboundMessage) => receivedMessages.push(msg));

    await startAndWaitReady(adapter);

    const pollCountBefore = telegramMock.getCallsFor("getUpdates").length;

    telegramMock.injectMessage({
      text: "I should be blocked",
      chatId: TEST_GROUP_ID,
      userId: 999999999,  // Not in allowed_users
      username: "hacker",
    });

    // Wait until mock confirms the update was delivered (next getUpdates cycle)
    await waitFor(
      () => telegramMock.getCallsFor("getUpdates").length > pollCountBefore,
      { timeout: 10_000, label: "getUpdates delivery" },
    );

    // Adapter should NOT have emitted the message
    expect(receivedMessages).toHaveLength(0);

    await adapter.stop();
  });

  it("T6: adapter.sendText() sends via mock server", async () => {
    const adapter = createTestAdapter();
    await startAndWaitReady(adapter);

    const result = await adapter.sendText(
      String(TEST_GROUP_ID),
      "Reply from agend!",
      { threadId: "42" },
    );

    expect(result.messageId).toBeDefined();

    const calls = telegramMock.getCallsFor("sendMessage");
    const sendCall = calls.find(c => c.params.text === "Reply from agend!");
    expect(sendCall).toBeDefined();
    expect(sendCall!.params.chat_id).toBe(TEST_GROUP_ID);
    expect(sendCall!.params.message_thread_id).toBe(42);

    await adapter.stop();
  });

  it("T6: adapter.react() sends setMessageReaction via mock", async () => {
    const adapter = createTestAdapter();
    await startAndWaitReady(adapter);

    await adapter.react(String(TEST_GROUP_ID), "123", "👍");

    await waitFor(
      () => telegramMock.getCallsFor("setMessageReaction").length > 0,
      { timeout: 5000, label: "setMessageReaction" },
    );
    expect(telegramMock.getCallsFor("setMessageReaction")).toHaveLength(1);

    await adapter.stop();
  });

  it("T5+T6: full round-trip — inject message → adapter receives → adapter replies", async () => {
    const adapter = createTestAdapter();
    const receivedMessages: InboundMessage[] = [];
    adapter.on("message", (msg: InboundMessage) => receivedMessages.push(msg));

    await startAndWaitReady(adapter);

    // 1. User sends message
    telegramMock.injectMessage({
      text: "What is 2+2?",
      chatId: TEST_GROUP_ID,
      userId: TEST_USER_ID,
      username: "user",
      threadId: 42,
    });

    // 2. Adapter receives it
    await waitFor(
      () => receivedMessages.length > 0,
      { timeout: 10_000, label: "receive message" },
    );

    // 3. Adapter sends reply
    const msg = receivedMessages[0];
    await adapter.sendText(msg.chatId, "The answer is 4", { threadId: msg.threadId });

    // 4. Mock server recorded the reply
    const replies = telegramMock.getCallsFor("sendMessage");
    const reply = replies.find(c => c.params.text === "The answer is 4");
    expect(reply).toBeDefined();
    expect(reply!.params.message_thread_id).toBe(42);

    await adapter.stop();
  });

  it("T3: adapter.createTopic() calls createForumTopic on mock", async () => {
    const adapter = createTestAdapter();
    await startAndWaitReady(adapter);
    adapter.setChatId(String(TEST_GROUP_ID));

    const topicId = await adapter.createTopic("new-test-instance");

    expect(topicId).toBeGreaterThan(0);
    const calls = telegramMock.getCallsFor("createForumTopic");
    expect(calls.length).toBeGreaterThan(0);
    expect(calls[calls.length - 1].params.name).toBe("new-test-instance");

    await adapter.stop();
  });

  it("T6: adapter.downloadAttachment() fetches file via mock", async () => {
    const adapter = createTestAdapter();
    await startAndWaitReady(adapter);

    const localPath = await adapter.downloadAttachment("fake_file_id_123");

    const getFileCalls = telegramMock.getCallsFor("getFile");
    expect(getFileCalls.length).toBeGreaterThan(0);
    expect(existsSync(localPath)).toBe(true);

    await adapter.stop();
  });

  it("T6: adapter.editMessage() calls editMessageText on mock", async () => {
    const adapter = createTestAdapter();
    await startAndWaitReady(adapter);

    await adapter.editMessage(String(TEST_GROUP_ID), "100", "Updated text");

    await waitFor(
      () => telegramMock.getCallsFor("editMessageText").length > 0,
      { timeout: 5000, label: "editMessageText" },
    );

    const calls = telegramMock.getCallsFor("editMessageText");
    expect(calls[0].params.text).toBe("Updated text");

    await adapter.stop();
  });
});
