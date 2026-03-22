import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { ContextGuardian } from "../src/context-guardian.js";
import { createLogger } from "../src/logger.js";
import { mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const makeConfig = (overrides = {}) => ({
  threshold_percentage: 60,
  max_idle_wait_ms: 300_000,
  completion_timeout_ms: 60_000,
  grace_period_ms: 600_000,
  max_age_hours: 8,
  ...overrides,
});

describe("ContextGuardian v2", () => {
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

  it("transitions to PENDING when threshold exceeded", () => {
    const pendingSpy = vi.fn();
    guardian.on("pending", pendingSpy);
    guardian.updateContextStatus({
      used_percentage: 65,
      remaining_percentage: 35,
      context_window_size: 1_000_000,
    });
    expect(guardian.state).toBe("PENDING");
    expect(pendingSpy).toHaveBeenCalledTimes(1);
  });

  it("stays NORMAL below threshold", () => {
    guardian.updateContextStatus({
      used_percentage: 55,
      remaining_percentage: 45,
      context_window_size: 1_000_000,
    });
    expect(guardian.state).toBe("NORMAL");
  });

  it("transitions PENDING → HANDING_OVER on idle signal", () => {
    const handoverSpy = vi.fn();
    guardian.on("request_handover", handoverSpy);
    guardian.updateContextStatus({
      used_percentage: 65,
      remaining_percentage: 35,
      context_window_size: 1_000_000,
    });
    guardian.signalIdle();
    expect(guardian.state).toBe("HANDING_OVER");
    expect(handoverSpy).toHaveBeenCalledTimes(1);
  });

  it("ignores idle signal when not PENDING", () => {
    guardian.signalIdle();
    expect(guardian.state).toBe("NORMAL");
  });

  it("transitions HANDING_OVER → ROTATING on handover complete", () => {
    const rotateSpy = vi.fn();
    guardian.on("rotate", rotateSpy);
    guardian.updateContextStatus({
      used_percentage: 65,
      remaining_percentage: 35,
      context_window_size: 1_000_000,
    });
    guardian.signalIdle();
    guardian.signalHandoverComplete();
    expect(guardian.state).toBe("ROTATING");
    expect(rotateSpy).toHaveBeenCalledTimes(1);
  });

  it("transitions HANDING_OVER → ROTATING on completion timeout", () => {
    const rotateSpy = vi.fn();
    guardian.on("rotate", rotateSpy);
    guardian.updateContextStatus({
      used_percentage: 65,
      remaining_percentage: 35,
      context_window_size: 1_000_000,
    });
    guardian.signalIdle();
    expect(guardian.state).toBe("HANDING_OVER");
    vi.advanceTimersByTime(60_001);
    expect(guardian.state).toBe("ROTATING");
    expect(rotateSpy).toHaveBeenCalledTimes(1);
  });

  it("enters GRACE after markRotationComplete", () => {
    guardian.updateContextStatus({
      used_percentage: 65,
      remaining_percentage: 35,
      context_window_size: 1_000_000,
    });
    guardian.signalIdle();
    guardian.signalHandoverComplete();
    guardian.markRotationComplete();
    expect(guardian.state).toBe("GRACE");
  });

  it("ignores threshold during GRACE period", () => {
    guardian.updateContextStatus({
      used_percentage: 65,
      remaining_percentage: 35,
      context_window_size: 1_000_000,
    });
    guardian.signalIdle();
    guardian.signalHandoverComplete();
    guardian.markRotationComplete();
    expect(guardian.state).toBe("GRACE");
    guardian.updateContextStatus({
      used_percentage: 70,
      remaining_percentage: 30,
      context_window_size: 1_000_000,
    });
    expect(guardian.state).toBe("GRACE");
  });

  it("returns to NORMAL after grace period expires", () => {
    guardian.updateContextStatus({
      used_percentage: 65,
      remaining_percentage: 35,
      context_window_size: 1_000_000,
    });
    guardian.signalIdle();
    guardian.signalHandoverComplete();
    guardian.markRotationComplete();
    vi.advanceTimersByTime(600_001);
    expect(guardian.state).toBe("NORMAL");
  });

  it("falls back to NORMAL if idle not detected within max_idle_wait", () => {
    guardian.updateContextStatus({
      used_percentage: 65,
      remaining_percentage: 35,
      context_window_size: 1_000_000,
    });
    expect(guardian.state).toBe("PENDING");
    vi.advanceTimersByTime(300_001);
    expect(guardian.state).toBe("NORMAL");
  });

  it("triggers rotation on max_age_hours timer", () => {
    const pendingSpy = vi.fn();
    guardian.on("pending", pendingSpy);
    guardian.startTimer();
    vi.advanceTimersByTime(8 * 60 * 60 * 1000);
    expect(guardian.state).toBe("PENDING");
    expect(pendingSpy).toHaveBeenCalledTimes(1);
  });
});
