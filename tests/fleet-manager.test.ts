import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { FleetManager, resolveReplyThreadId } from "../src/fleet-manager.js";
import { TopicCommands } from "../src/topic-commands.js";
import { join, basename } from "node:path";
import { mkdirSync, rmSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import yaml from "js-yaml";

describe("FleetManager", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `ccd-fleet-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("detects stopped instance (no PID)", () => {
    const fm = new FleetManager(tmpDir);
    mkdirSync(join(tmpDir, "instances/test"), { recursive: true });
    expect(fm.getInstanceStatus("test")).toBe("stopped");
  });

  it("detects crashed instance (stale PID)", () => {
    const fm = new FleetManager(tmpDir);
    const dir = join(tmpDir, "instances/test");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, "daemon.pid"), "99999999");
    expect(fm.getInstanceStatus("test")).toBe("crashed");
  });

  it("builds routing table from config", () => {
    const fm = new FleetManager(tmpDir);
    const configPath = join(tmpDir, "fleet.yaml");
    writeFileSync(configPath, `
channel:
  type: telegram
  mode: topic
  bot_token_env: BOT
  group_id: -100
  access:
    mode: locked
    allowed_users: [1]
instances:
  proj-a:
    working_directory: /tmp/a
    topic_id: 42
  proj-b:
    working_directory: /tmp/b
    topic_id: 87
  proj-c:
    working_directory: /tmp/c
`);
    fm.loadConfig(configPath);
    const table = fm.buildRoutingTable();
    expect(table.get("42")).toEqual({ kind: "instance", name: "proj-a" });
    expect(table.get("87")).toEqual({ kind: "instance", name: "proj-b" });
    expect(table.size).toBe(2); // proj-c has no topic_id
  });

  it("marks the General topic as non-probeable in the routing table", () => {
    const fm = new FleetManager(tmpDir);
    const configPath = join(tmpDir, "fleet.yaml");
    writeFileSync(configPath, `
channel:
  type: telegram
  mode: topic
  bot_token_env: BOT
  group_id: -100
  access:
    mode: locked
    allowed_users: [1]
instances:
  general:
    working_directory: /tmp/general
    topic_id: 1
    general_topic: true
`);
    fm.loadConfig(configPath);
    const table = fm.buildRoutingTable();
    expect(table.get("1")).toEqual({ kind: "general", name: "general" });
  });

  it("createForumTopic delegates to adapter.createTopic", async () => {
    const fm = new FleetManager(tmpDir);

    // No adapter set — should throw
    await expect(fm.createForumTopic("my-topic")).rejects.toThrow("Adapter does not support topic creation");

    // Set a mock adapter with createTopic
    fm.adapter = {
      createTopic: async (name: string) => {
        expect(name).toBe("my-topic");
        return 999;
      },
    } as any;

    const threadId = await fm.createForumTopic("my-topic");
    expect(threadId).toBe(999);
  });

  it("does not default replies to thread 1 for the General instance", () => {
    const threadId = resolveReplyThreadId(undefined, {
      working_directory: "/tmp/general",
      topic_id: 1,
      general_topic: true,
      restart_policy: { max_retries: 1, backoff: "linear", reset_after: 1 },
      context_guardian: {
        threshold_percentage: 60,
        max_idle_wait_ms: 300_000,
        completion_timeout_ms: 60_000,
        grace_period_ms: 600_000,
        max_age_hours: 8,
      },
      memory: { auto_summarize: true, watch_memory_dir: true, backup_to_sqlite: true },
      log_level: "info",
    });
    expect(threadId).toBeUndefined();
  });

  it("defaults replies to the instance topic for normal instances", () => {
    const threadId = resolveReplyThreadId(undefined, {
      working_directory: "/tmp/proj",
      topic_id: 42,
      restart_policy: { max_retries: 1, backoff: "linear", reset_after: 1 },
      context_guardian: {
        threshold_percentage: 60,
        max_idle_wait_ms: 300_000,
        completion_timeout_ms: 60_000,
        grace_period_ms: 600_000,
        max_age_hours: 8,
      },
      memory: { auto_summarize: true, watch_memory_dir: true, backup_to_sqlite: true },
      log_level: "info",
    });
    expect(threadId).toBe("42");
  });

  it("saveFleetConfig preserves all optional user-configured fields", () => {
    const fm = new FleetManager(tmpDir);
    const configPath = join(tmpDir, "fleet.yaml");
    writeFileSync(configPath, `
channel:
  type: telegram
  mode: topic
  bot_token_env: BOT
  group_id: -100
  access:
    mode: locked
    allowed_users: [1]
instances:
  my-proj:
    working_directory: /tmp/my-proj
    topic_id: 10
    description: "A test instance"
    tags: [code-reviewer, researcher]
    model: claude-opus-4-6
    model_failover: [sonnet]
    worktree_source: /tmp/source-repo
    backend: claude-code
    skipPermissions: true
    lightweight: true
    memory_directory: /tmp/memory
`);
    fm.loadConfig(configPath);
    fm.saveFleetConfig();

    const saved = yaml.load(readFileSync(configPath, "utf8")) as Record<string, unknown>;
    const inst = (saved.instances as Record<string, unknown>)["my-proj"] as Record<string, unknown>;

    expect(inst.description).toBe("A test instance");
    expect(inst.tags).toEqual(["code-reviewer", "researcher"]);
    expect(inst.model).toBe("claude-opus-4-6");
    expect(inst.model_failover).toEqual(["sonnet"]);
    expect(inst.worktree_source).toBe("/tmp/source-repo");
    expect(inst.backend).toBe("claude-code");
    expect(inst.skipPermissions).toBe(true);
    expect(inst.lightweight).toBe(true);
    expect(inst.memory_directory).toBe("/tmp/memory");
    // Core fields still present
    expect(inst.working_directory).toBe("/tmp/my-proj");
    expect(inst.topic_id).toBe(10);
  });
});

describe("TopicCommands", () => {
  it("handleGeneralCommand returns false for non-commands", async () => {
    const adapter = { sendText: vi.fn() };
    const tc = new TopicCommands({ adapter } as any);
    const result = await tc.handleGeneralCommand({ text: "hello", chatId: "1", messageId: "1", username: "u", userId: "1", timestamp: new Date() } as any);
    expect(result).toBe(false);
  });

  it("ignores topic deletion for the General instance", async () => {
    const logger = { debug: vi.fn(), info: vi.fn() };
    const removeInstance = vi.fn();
    const tc = new TopicCommands({
      logger,
      removeInstance,
      routingTable: new Map([["1", { kind: "general", name: "general" }]]),
      fleetConfig: {
        defaults: {},
        instances: {
          general: {
            working_directory: "/tmp/general",
            topic_id: 1,
            general_topic: true,
            restart_policy: { max_retries: 1, backoff: "linear", reset_after: 1 },
            context_guardian: {
              threshold_percentage: 60,
              max_idle_wait_ms: 300_000,
              completion_timeout_ms: 60_000,
              grace_period_ms: 600_000,
              max_age_hours: 8,
            },
            memory: { auto_summarize: true, watch_memory_dir: true, backup_to_sqlite: true },
            log_level: "info",
          },
        },
      },
    } as any);

    await tc.handleTopicDeleted("1");

    expect(removeInstance).not.toHaveBeenCalled();
    expect(logger.info).not.toHaveBeenCalled();
    expect(logger.debug).toHaveBeenCalled();
  });
});
