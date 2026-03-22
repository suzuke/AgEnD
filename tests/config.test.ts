import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { loadConfig, loadFleetConfig, DEFAULT_CONFIG } from "../src/config.js";
import { writeFileSync, mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("loadConfig", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `ccd-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("returns defaults when no config file exists", () => {
    const config = loadConfig(join(tmpDir, "nonexistent.yaml"));
    expect(config).toEqual(DEFAULT_CONFIG);
  });

  it("merges partial config with defaults", () => {
    const configPath = join(tmpDir, "config.yaml");
    writeFileSync(configPath, "channel_plugin: custom-plugin\n");
    const config = loadConfig(configPath);
    expect(config.channel_plugin).toBe("custom-plugin");
    expect(config.restart_policy.max_retries).toBe(DEFAULT_CONFIG.restart_policy.max_retries);
  });

  it("loads legacy config.yaml and maps to InstanceConfig shape", () => {
    const configPath = join(tmpDir, "config.yaml");
    writeFileSync(configPath, `
channel_plugin: telegram@claude-plugins-official
working_directory: /tmp/legacy
restart_policy:
  max_retries: 5
  backoff: exponential
  reset_after: 300
context_guardian:
  threshold_percentage: 70
  max_idle_wait_ms: 300000
  completion_timeout_ms: 60000
  grace_period_ms: 600000
  max_age_hours: 2
memory:
  auto_summarize: false
  watch_memory_dir: true
  backup_to_sqlite: true
log_level: info
`);
    const config = loadConfig(configPath);
    // Verify it has the fields needed to construct an InstanceConfig
    expect(config.working_directory).toBe("/tmp/legacy");
    expect(config.restart_policy.max_retries).toBe(5);
    expect(config.channel_plugin).toBe("telegram@claude-plugins-official");
  });

  it("reads full config from YAML file", () => {
    const configPath = join(tmpDir, "config.yaml");
    writeFileSync(
      configPath,
      `channel_plugin: telegram@claude-plugins-official
working_directory: /tmp/test
restart_policy:
  max_retries: 5
  backoff: linear
  reset_after: 120
context_guardian:
  threshold_percentage: 70
  max_idle_wait_ms: 300000
  completion_timeout_ms: 60000
  grace_period_ms: 600000
  max_age_hours: 2
memory:
  auto_summarize: false
  watch_memory_dir: false
  backup_to_sqlite: false
log_level: debug
`
    );
    const config = loadConfig(configPath);
    expect(config.channel_plugin).toBe("telegram@claude-plugins-official");
    expect(config.restart_policy.backoff).toBe("linear");
    expect(config.context_guardian.threshold_percentage).toBe(70);
    expect(config.memory.auto_summarize).toBe(false);
  });

  it("has correct default context_guardian values", () => {
    const config = loadConfig("/nonexistent/path.yaml");
    expect(config.context_guardian).toEqual({
      threshold_percentage: 60,
      max_idle_wait_ms: 300_000,
      completion_timeout_ms: 60_000,
      grace_period_ms: 600_000,
      max_age_hours: 8,
    });
  });
});

describe("loadFleetConfig", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `ccd-fleet-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("loads fleet.yaml with defaults merged into instances", () => {
    const fleetPath = join(tmpDir, "fleet.yaml");
    writeFileSync(
      fleetPath,
      `channel:
  type: telegram
  mode: dm
  bot_token_env: BOT_TOKEN
  access:
    mode: pairing
    allowed_users: []
    max_pending_codes: 5
    code_expiry_minutes: 10
defaults:
  restart_policy:
    max_retries: 3
    backoff: linear
    reset_after: 60
  log_level: debug
instances:
  mybot:
    working_directory: /home/user/mybot
    topic_id: 42
    context_guardian:
      threshold_percentage: 90
      max_idle_wait_ms: 300000
      completion_timeout_ms: 60000
      grace_period_ms: 600000
      max_age_hours: 2
`
    );
    const fleet = loadFleetConfig(fleetPath);

    // restart_policy from defaults should be merged in
    expect(fleet.instances.mybot.restart_policy.max_retries).toBe(3);
    expect(fleet.instances.mybot.restart_policy.backoff).toBe("linear");

    // context_guardian from instance overrides defaults
    expect(fleet.instances.mybot.context_guardian.threshold_percentage).toBe(90);
    expect(fleet.instances.mybot.context_guardian.max_idle_wait_ms).toBe(300000);

    // topic_id preserved
    expect(fleet.instances.mybot.topic_id).toBe(42);

    // top-level channel present
    expect(fleet.channel).toBeDefined();
    expect(fleet.channel!.type).toBe("telegram");
    expect(fleet.channel!.mode).toBe("dm");
  });

  it("validates required fields", () => {
    const fleetPath = join(tmpDir, "fleet.yaml");
    writeFileSync(
      fleetPath,
      `defaults: {}
instances:
  badbot:
    log_level: info
`
    );
    expect(() => loadFleetConfig(fleetPath)).toThrow(/working_directory/);
  });

  it("returns empty instances when no fleet.yaml exists", () => {
    const fleet = loadFleetConfig(join(tmpDir, "nonexistent-fleet.yaml"));
    expect(fleet.instances).toEqual({});
    expect(fleet.defaults).toEqual({});
  });
});
