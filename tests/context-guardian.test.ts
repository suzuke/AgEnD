import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { ContextGuardian } from "../src/context-guardian.js";
import { DEFAULT_CONFIG } from "../src/config.js";
import { createLogger } from "../src/logger.js";
import { writeFileSync, mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("ContextGuardian", () => {
  const logger = createLogger("silent");
  let guardian: ContextGuardian;
  let tmpDir: string;
  let statusFile: string;

  beforeEach(() => {
    vi.useFakeTimers();
    tmpDir = join(tmpdir(), `ccd-guardian-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
    statusFile = join(tmpDir, "statusline.json");
    guardian = new ContextGuardian(DEFAULT_CONFIG.context_guardian, logger, statusFile);
  });

  afterEach(() => {
    guardian.stop();
    vi.useRealTimers();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("triggers rotation when usage exceeds threshold", () => {
    const rotateSpy = vi.fn();
    guardian.on("rotate", rotateSpy);

    guardian.updateContextStatus({
      used_percentage: 85,
      remaining_percentage: 15,
      context_window_size: 200000,
    });

    expect(rotateSpy).toHaveBeenCalledTimes(1);
  });

  it("does not trigger rotation below threshold", () => {
    const rotateSpy = vi.fn();
    guardian.on("rotate", rotateSpy);

    guardian.updateContextStatus({
      used_percentage: 50,
      remaining_percentage: 50,
      context_window_size: 200000,
    });

    expect(rotateSpy).not.toHaveBeenCalled();
  });

  it("triggers timer-based rotation after max_age_hours", () => {
    const rotateSpy = vi.fn();
    guardian.on("rotate", rotateSpy);
    guardian.startTimer();

    vi.advanceTimersByTime(4 * 60 * 60 * 1000); // 4 hours

    expect(rotateSpy).toHaveBeenCalledTimes(1);
  });
});
