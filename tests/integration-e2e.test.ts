/**
 * Integration E2E tests — verify the full config → routing → access → dispatch path
 * without needing Telegram, tmux, or real CLI backends.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
import yaml from "js-yaml";
import { buildFleetConfig, type WizardAnswers } from "../src/setup-wizard.js";
import { FleetManager } from "../src/fleet-manager.js";
import { AccessManager } from "../src/channel/access-manager.js";
import { outboundHandlers } from "../src/outbound-handlers.js";

// ── Mock helpers ────────────────────────────────────────────

function mockAdapter() {
  const sent: Array<{ chatId: string; text: string; opts?: unknown }> = [];
  return {
    sent,
    adapter: {
      type: "test",
      id: "test",
      topology: "topics" as const,
      on: vi.fn(),
      once: vi.fn(),
      emit: vi.fn(),
      removeListener: vi.fn(),
      sendText: vi.fn(async (chatId: string, text: string, opts?: unknown) => {
        sent.push({ chatId, text, opts });
        return { messageId: String(sent.length), chatId };
      }),
      sendFile: vi.fn(),
      editMessage: vi.fn(),
      react: vi.fn(),
      sendApproval: vi.fn(),
      downloadAttachment: vi.fn(),
      handlePairing: vi.fn(),
      confirmPairing: vi.fn(),
      setChatId: vi.fn(),
      getChatId: () => "-100",
      promptUser: vi.fn(),
      notifyAlert: vi.fn(),
    },
  };
}

function mockIpc() {
  const messages: unknown[] = [];
  return {
    messages,
    ipc: {
      send: (msg: unknown) => messages.push(msg),
      connected: true,
      on: vi.fn(),
      once: vi.fn(),
      emit: vi.fn(),
      removeListener: vi.fn(),
      connect: vi.fn(),
      close: vi.fn(),
    },
  };
}

const defaultAnswers: WizardAnswers = {
  backend: "claude-code",
  botTokenEnv: "TEST_BOT_TOKEN",
  groupId: -1001234567890,
  channelMode: "topic",
  accessMode: "locked",
  allowedUsers: ["111"],
  projectRoots: [],
  instances: [
    { name: "worker-a", workDir: "/tmp/a", topicId: "42" },
    { name: "worker-b", workDir: "/tmp/b", topicId: "87" },
  ],
  costGuard: { enabled: false },
  dailySummary: { enabled: false },
};

// ── Tests ───────────────────────────────────────────────────

describe("Integration E2E", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `e2e-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  // ── 1. Config roundtrip ──────────────────────────────────

  describe("Config roundtrip: wizard → YAML → runtime", () => {
    it("routing resolves correctly after YAML roundtrip", () => {
      const config = buildFleetConfig(defaultAnswers);
      const yamlStr = yaml.dump(config);
      const configPath = join(tmpDir, "fleet.yaml");
      writeFileSync(configPath, yamlStr);

      const fm = new FleetManager(tmpDir);
      fm.loadConfig(configPath);
      fm.buildRoutingTable();

      expect(fm.routing.resolve("42")).toEqual({ kind: "instance", name: "worker-a" });
      expect(fm.routing.resolve("87")).toEqual({ kind: "instance", name: "worker-b" });
      expect(fm.routing.resolve("999")).toBeUndefined();
    });

    it("access manager works with YAML-roundtripped user IDs", () => {
      // Wizard stores string "111", YAML may parse as number 111
      const config = buildFleetConfig(defaultAnswers);
      const yamlStr = yaml.dump(config);
      const loaded = yaml.load(yamlStr) as Record<string, any>;
      const accessCfg = loaded.channel.access;

      const am = new AccessManager(
        { mode: accessCfg.mode, allowed_users: accessCfg.allowed_users ?? [], max_pending_codes: 3, code_expiry_minutes: 60 },
        join(tmpDir, "access.json"),
      );

      // Telegram sends number
      expect(am.isAllowed(111)).toBe(true);
      // Discord sends string
      expect(am.isAllowed("111")).toBe(true);
      // Unknown user
      expect(am.isAllowed(999)).toBe(false);
    });

    it("backend defaults survive YAML roundtrip", () => {
      const config = buildFleetConfig({ ...defaultAnswers, backend: "codex" });
      const yamlStr = yaml.dump(config);
      const loaded = yaml.load(yamlStr) as Record<string, any>;
      expect(loaded.defaults.backend).toBe("codex");
    });
  });

  // ── 2. Access + Routing ──────────────────────────────────

  describe("Access + Routing: message dispatch", () => {
    it("authorized user message resolves to correct instance", () => {
      const config = buildFleetConfig(defaultAnswers);
      writeFileSync(join(tmpDir, "fleet.yaml"), yaml.dump(config));
      const fm = new FleetManager(tmpDir);
      fm.loadConfig(join(tmpDir, "fleet.yaml"));
      fm.buildRoutingTable();

      const loaded = yaml.load(yaml.dump(config)) as Record<string, any>;
      const accessCfg = loaded.channel.access;
      const am = new AccessManager(
        { mode: accessCfg.mode, allowed_users: accessCfg.allowed_users ?? [], max_pending_codes: 3, code_expiry_minutes: 60 },
        join(tmpDir, "access.json"),
      );

      // Simulate: user 111 sends message in thread 42
      const userId = 111; // Telegram number
      const threadId = "42";

      expect(am.isAllowed(userId)).toBe(true);
      const target = fm.routing.resolve(threadId);
      expect(target).toEqual({ kind: "instance", name: "worker-a" });
    });

    it("unauthorized user is rejected", () => {
      const config = buildFleetConfig(defaultAnswers);
      const loaded = yaml.load(yaml.dump(config)) as Record<string, any>;
      const am = new AccessManager(
        { mode: loaded.channel.access.mode, allowed_users: loaded.channel.access.allowed_users ?? [], max_pending_codes: 3, code_expiry_minutes: 60 },
        join(tmpDir, "access2.json"),
      );

      expect(am.isAllowed(999)).toBe(false);
    });

    it("unknown thread returns undefined (unbound topic)", () => {
      const config = buildFleetConfig(defaultAnswers);
      writeFileSync(join(tmpDir, "fleet.yaml"), yaml.dump(config));
      const fm = new FleetManager(tmpDir);
      fm.loadConfig(join(tmpDir, "fleet.yaml"));
      fm.buildRoutingTable();

      expect(fm.routing.resolve("9999")).toBeUndefined();
    });
  });

  // ── 3. Outbound dispatch ─────────────────────────────────

  describe("Outbound dispatch: send_to_instance E2E", () => {
    it("send_to_instance delivers message to target IPC", async () => {
      const { adapter } = mockAdapter();
      const senderIpc = mockIpc();
      const targetIpc = mockIpc();

      const ctx = {
        fleetConfig: { instances: { sender: { working_directory: "/tmp/s" }, target: { working_directory: "/tmp/t" } } },
        adapter,
        logger: { info: vi.fn(), warn: vi.fn(), debug: vi.fn(), error: vi.fn() },
        routing: { resolve: () => undefined },
        instanceIpcClients: new Map([
          ["sender", senderIpc.ipc],
          ["target", targetIpc.ipc],
        ]),
        lifecycle: { daemons: new Map() },
        sessionRegistry: new Map(),
        eventLog: null,
        lastActivityMs: () => 0,
        startInstance: vi.fn(),
        connectIpcToInstance: vi.fn(),
      } as any;

      const handler = outboundHandlers.get("send_to_instance")!;
      const respond = vi.fn();

      await handler(ctx, {
        instance_name: "target",
        message: "Hello from sender",
      }, respond, {
        instanceName: "sender",
        requestId: 1,
        fleetRequestId: undefined,
        senderSessionName: undefined,
      });

      // Verify target IPC received the message
      expect(targetIpc.messages).toHaveLength(1);
      const delivered = targetIpc.messages[0] as Record<string, unknown>;
      expect(delivered.type).toBe("fleet_inbound");
      expect(delivered.content).toBe("Hello from sender");

      // Verify respond called with success
      expect(respond).toHaveBeenCalledWith(expect.objectContaining({
        sent: true,
        target: "target",
      }));
    });

    it("send_to_instance rejects unknown target", async () => {
      const { adapter } = mockAdapter();
      const senderIpc = mockIpc();

      const ctx = {
        fleetConfig: { instances: {} },
        adapter,
        logger: { info: vi.fn(), warn: vi.fn(), debug: vi.fn(), error: vi.fn() },
        routing: { resolve: () => undefined },
        instanceIpcClients: new Map([["sender", senderIpc.ipc]]),
        lifecycle: { daemons: new Map() },
        sessionRegistry: new Map(),
        eventLog: null,
        lastActivityMs: () => 0,
        startInstance: vi.fn(),
        connectIpcToInstance: vi.fn(),
      } as any;

      const handler = outboundHandlers.get("send_to_instance")!;
      const respond = vi.fn();

      await handler(ctx, {
        instance_name: "nonexistent",
        message: "Hello",
      }, respond, {
        instanceName: "sender",
        requestId: 1,
        fleetRequestId: undefined,
        senderSessionName: undefined,
      });

      // Verify error response
      expect(respond).toHaveBeenCalledWith(null, expect.stringContaining("not found"));
    });
  });
});
