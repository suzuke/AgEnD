import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { ContextGuardian } from "../src/context-guardian.js";
import { createLogger } from "../src/logger.js";
import { mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const makeConfig = (overrides = {}) => ({
  restart_threshold_pct: 80,
  grace_period_ms: 600_000,
  max_age_hours: 8,
  ...overrides,
});

describe("ContextGuardian v3", () => {
  const logger = createLogger("silent");
  let guardian: ContextGuardian;
  let tmpDir: string;
  let statusFile: string;

  beforeEach(() => {
    vi.useFakeTimers();
    tmpDir = join(tmpdir(), `ccd-guardian-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
    statusFile = join(tmpDir, "statusline.json");
    guardian = new ContextGuardian(makeConfig(), logger, statusFile);
  });

  afterEach(() => {
    guardian.stop();
    vi.useRealTimers();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("starts in NORMAL state", () => {
    expect(guardian.state).toBe("NORMAL");
  });

  it("transitions to RESTARTING when threshold exceeded", () => {
    const restartSpy = vi.fn();
    guardian.on("restart_requested", restartSpy);
    guardian.updateContextStatus({
      used_percentage: 85,
      remaining_percentage: 15,
      context_window_size: 1_000_000,
    });
    expect(guardian.state).toBe("RESTARTING");
    expect(restartSpy).toHaveBeenCalledTimes(1);
    expect(restartSpy).toHaveBeenCalledWith("context_full");
  });

  it("stays NORMAL below threshold", () => {
    guardian.updateContextStatus({
      used_percentage: 75,
      remaining_percentage: 25,
      context_window_size: 1_000_000,
    });
    expect(guardian.state).toBe("NORMAL");
  });

  it("enters GRACE after markRestartComplete", () => {
    guardian.updateContextStatus({
      used_percentage: 85,
      remaining_percentage: 15,
      context_window_size: 1_000_000,
    });
    expect(guardian.state).toBe("RESTARTING");
    guardian.markRestartComplete();
    expect(guardian.state).toBe("GRACE");
  });

  it("emits restart_complete on markRestartComplete", () => {
    const completeSpy = vi.fn();
    guardian.on("restart_complete", completeSpy);
    guardian.updateContextStatus({
      used_percentage: 85,
      remaining_percentage: 15,
      context_window_size: 1_000_000,
    });
    guardian.markRestartComplete();
    expect(completeSpy).toHaveBeenCalledTimes(1);
  });

  it("ignores threshold during GRACE period", () => {
    guardian.updateContextStatus({
      used_percentage: 85,
      remaining_percentage: 15,
      context_window_size: 1_000_000,
    });
    guardian.markRestartComplete();
    expect(guardian.state).toBe("GRACE");
    guardian.updateContextStatus({
      used_percentage: 90,
      remaining_percentage: 10,
      context_window_size: 1_000_000,
    });
    expect(guardian.state).toBe("GRACE");
  });

  it("returns to NORMAL after grace period expires", () => {
    guardian.updateContextStatus({
      used_percentage: 85,
      remaining_percentage: 15,
      context_window_size: 1_000_000,
    });
    guardian.markRestartComplete();
    vi.advanceTimersByTime(600_001);
    expect(guardian.state).toBe("NORMAL");
  });

  it("triggers restart on max_age_hours timer", () => {
    const restartSpy = vi.fn();
    guardian.on("restart_requested", restartSpy);
    guardian.startTimer();
    vi.advanceTimersByTime(8 * 60 * 60 * 1000);
    expect(guardian.state).toBe("RESTARTING");
    expect(restartSpy).toHaveBeenCalledTimes(1);
    expect(restartSpy).toHaveBeenCalledWith("max_age");
  });

  it("ignores requestRestart when not NORMAL", () => {
    guardian.requestRestart("context_full");
    expect(guardian.state).toBe("RESTARTING");
    // Second request should be ignored
    const restartSpy = vi.fn();
    guardian.on("restart_requested", restartSpy);
    guardian.requestRestart("max_age");
    expect(restartSpy).not.toHaveBeenCalled();
  });

  it("supports legacy threshold_percentage fallback", () => {
    const legacyGuardian = new ContextGuardian(
      makeConfig({ restart_threshold_pct: undefined, threshold_percentage: 60 }),
      logger,
      statusFile,
    );
    const restartSpy = vi.fn();
    legacyGuardian.on("restart_requested", restartSpy);
    legacyGuardian.updateContextStatus({
      used_percentage: 65,
      remaining_percentage: 35,
      context_window_size: 1_000_000,
    });
    expect(legacyGuardian.state).toBe("RESTARTING");
    legacyGuardian.stop();
  });

  it("resets age timer after grace period", () => {
    const restartSpy = vi.fn();
    guardian.on("restart_requested", restartSpy);
    guardian.startTimer();

    // Trigger restart via context
    guardian.updateContextStatus({
      used_percentage: 85,
      remaining_percentage: 15,
      context_window_size: 1_000_000,
    });
    guardian.markRestartComplete();

    // Grace expires, returns to NORMAL
    vi.advanceTimersByTime(600_001);
    expect(guardian.state).toBe("NORMAL");

    // Age timer should have been reset — wait full max_age
    vi.advanceTimersByTime(8 * 60 * 60 * 1000);
    expect(guardian.state).toBe("RESTARTING");
    expect(restartSpy).toHaveBeenCalledTimes(2);
  });
});
