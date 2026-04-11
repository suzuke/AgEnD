import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { ContextGuardian } from "../src/context-guardian.js";
import { createLogger } from "../src/logger.js";
import { mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const makeConfig = (overrides = {}) => ({
  grace_period_ms: 600_000,
  max_age_hours: 0,
  ...overrides,
});

describe("ContextGuardian (pure monitoring)", () => {
  const logger = createLogger("silent");
  let guardian: ContextGuardian;
  let tmpDir: string;
  let statusFile: string;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `ccd-guardian-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
    statusFile = join(tmpDir, "statusline.json");
    guardian = new ContextGuardian(makeConfig(), logger, statusFile);
  });

  afterEach(() => {
    guardian.stop();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("emits status_update when statusline is written", async () => {
    const spy = vi.fn();
    guardian.on("status_update", spy);
    guardian.startWatching();

    const { writeFileSync } = await import("node:fs");
    writeFileSync(statusFile, JSON.stringify({
      session_id: "test",
      model: { id: "test", display_name: "test" },
      context_window: {
        total_input_tokens: 100,
        total_output_tokens: 50,
        context_window_size: 200000,
        current_usage: 50000,
        used_percentage: 25,
        remaining_percentage: 75,
      },
      cost: { total_cost_usd: 0.5, total_duration_ms: 1000 },
    }));

    // watchFile polls at 2s interval — wait for it
    await new Promise(r => setTimeout(r, 3000));
    expect(spy).toHaveBeenCalled();
    expect(spy.mock.calls[0][0].used_percentage).toBe(25);
  });

  it("does not have state, requestRestart, or startTimer methods", () => {
    // Verify the simplified API — no state machine
    expect((guardian as any).state).toBeUndefined();
    expect((guardian as any).requestRestart).toBeUndefined();
    expect((guardian as any).startTimer).toBeUndefined();
  });
});
