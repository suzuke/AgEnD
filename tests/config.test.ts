import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { loadConfig, DEFAULT_CONFIG } from "../src/config.js";
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
  max_age_hours: 2
  strategy: timer
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
    expect(config.context_guardian.strategy).toBe("timer");
    expect(config.memory.auto_summarize).toBe(false);
  });
});
