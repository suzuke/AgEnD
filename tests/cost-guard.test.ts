import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdirSync, rmSync } from "node:fs";
import { CostGuard } from "../src/cost-guard.js";
import { EventLog } from "../src/event-log.js";
import type { CostGuardConfig } from "../src/types.js";

const makeConfig = (overrides: Partial<CostGuardConfig> = {}): CostGuardConfig => ({
  daily_limit_usd: 10,
  warn_at_percentage: 80,
  timezone: "UTC",
  ...overrides,
});

describe("CostGuard", () => {
  let tmpDir: string;
  let eventLog: EventLog;
  let guard: CostGuard;

  beforeEach(() => {
    vi.useFakeTimers();
    tmpDir = join(tmpdir(), `ccd-cost-guard-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
    eventLog = new EventLog(join(tmpDir, "events.db"));
    guard = new CostGuard(makeConfig(), eventLog);
  });

  afterEach(() => {
    guard.stop();
    eventLog.close();
    vi.useRealTimers();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("tracks cost in cents", () => {
    guard.updateCost("agent1", 3.50);
    expect(guard.getDailyCostCents("agent1")).toBe(350);
  });

  it("accumulates across sessions (rotation)", () => {
    guard.updateCost("agent1", 3.50);
    guard.snapshotAndReset("agent1");
    guard.updateCost("agent1", 1.20);
    // 350 + 120 = 470
    expect(guard.getDailyCostCents("agent1")).toBe(470);
  });

  it("emits warn when threshold exceeded (85% of $10 = $8.50)", () => {
    const warnSpy = vi.fn();
    guard.on("warn", warnSpy);
    // 80% of $10 = $8.00 threshold; $8.50 > $8.00
    guard.updateCost("agent1", 8.50);
    expect(warnSpy).toHaveBeenCalledTimes(1);
    expect(warnSpy).toHaveBeenCalledWith("agent1", 850, 1000);
  });

  it("emits limit when daily limit exceeded ($10.50 > $10)", () => {
    const limitSpy = vi.fn();
    guard.on("limit", limitSpy);
    guard.updateCost("agent1", 10.50);
    expect(limitSpy).toHaveBeenCalledTimes(1);
    expect(limitSpy).toHaveBeenCalledWith("agent1", 1050, 1000);
  });

  it("does not emit warn or limit when limit is 0 (disabled)", () => {
    const guardDisabled = new CostGuard(makeConfig({ daily_limit_usd: 0 }), eventLog);
    const warnSpy = vi.fn();
    const limitSpy = vi.fn();
    guardDisabled.on("warn", warnSpy);
    guardDisabled.on("limit", limitSpy);
    guardDisabled.updateCost("agent1", 999.99);
    expect(warnSpy).not.toHaveBeenCalled();
    expect(limitSpy).not.toHaveBeenCalled();
    guardDisabled.stop();
  });

  it("resets at midnight (resetDaily clears everything)", () => {
    guard.updateCost("agent1", 5.00);
    guard.updateCost("agent2", 3.00);
    expect(guard.getDailyCostCents("agent1")).toBe(500);
    expect(guard.getDailyCostCents("agent2")).toBe(300);
    guard.resetDaily();
    expect(guard.getDailyCostCents("agent1")).toBe(0);
    expect(guard.getDailyCostCents("agent2")).toBe(0);
  });

  it("emits daily_reset event on resetDaily", () => {
    const resetSpy = vi.fn();
    guard.on("daily_reset", resetSpy);
    guard.resetDaily();
    expect(resetSpy).toHaveBeenCalledTimes(1);
  });

  it("logs cost_snapshot event on snapshot", () => {
    guard.updateCost("agent1", 4.25);
    guard.snapshotAndReset("agent1");
    const events = eventLog.query({ instance: "agent1", type: "cost_snapshot" });
    expect(events).toHaveLength(1);
    expect(events[0].payload).toMatchObject({
      session_cost_usd: 4.25,
      accumulated_cents: 425,
    });
  });

  it("returns fleet total across instances", () => {
    guard.updateCost("agent1", 3.00);
    guard.updateCost("agent2", 2.50);
    guard.updateCost("agent3", 1.00);
    // 300 + 250 + 100 = 650
    expect(guard.getFleetTotalCents()).toBe(650);
  });

  it("does not emit warn/limit twice for the same day", () => {
    const warnSpy = vi.fn();
    const limitSpy = vi.fn();
    guard.on("warn", warnSpy);
    guard.on("limit", limitSpy);
    guard.updateCost("agent1", 8.50);
    guard.updateCost("agent1", 9.00);
    expect(warnSpy).toHaveBeenCalledTimes(1);
    guard.updateCost("agent1", 10.50);
    guard.updateCost("agent1", 11.00);
    expect(limitSpy).toHaveBeenCalledTimes(1);
  });

  it("isLimited returns true when limit exceeded", () => {
    guard.updateCost("agent1", 10.01);
    expect(guard.isLimited("agent1")).toBe(true);
  });

  it("isLimited returns false when limit is 0 (disabled)", () => {
    const guardDisabled = new CostGuard(makeConfig({ daily_limit_usd: 0 }), eventLog);
    guardDisabled.updateCost("agent1", 999.99);
    expect(guardDisabled.isLimited("agent1")).toBe(false);
    guardDisabled.stop();
  });

  it("re-emits limit after rotation if accumulated still exceeds (P2.2)", () => {
    // Reproduces the bug where a user manually restarts a paused instance
    // and the new session burns through the daily budget unannounced.
    const limitSpy = vi.fn();
    guard.on("limit", limitSpy);

    // Session 1: blow past $10 limit → emit + (in real fleet) instance paused
    guard.updateCost("agent1", 10.50);
    expect(limitSpy).toHaveBeenCalledTimes(1);

    // User manually restarts → new session reports cost=0 → rotation detected
    guard.updateCost("agent1", 0);
    // Then ramps the new session up past the (already-exceeded) accumulated cap
    guard.updateCost("agent1", 0.50);

    // Should re-emit so the fleet can re-pause the instance
    expect(limitSpy).toHaveBeenCalledTimes(2);
  });

  it("re-emits warn after rotation if accumulated still above warn threshold (P2.2)", () => {
    const warnSpy = vi.fn();
    guard.on("warn", warnSpy);

    guard.updateCost("agent1", 8.50); // warn at 80% of $10
    expect(warnSpy).toHaveBeenCalledTimes(1);

    // Rotation
    guard.updateCost("agent1", 0);
    guard.updateCost("agent1", 0.10);

    // accumulated $8.50 + new $0.10 = $8.60, still > warn threshold $8
    expect(warnSpy).toHaveBeenCalledTimes(2);
  });

  it("does not re-emit on rotation if accumulated drops below threshold (sanity)", () => {
    // If the next session doesn't push us back over the threshold, no re-emit.
    const warnSpy = vi.fn();
    guard.on("warn", warnSpy);

    // Session 1: under warn threshold
    guard.updateCost("agent1", 5.00);
    expect(warnSpy).not.toHaveBeenCalled();

    // Rotation, low new session
    guard.updateCost("agent1", 0);
    guard.updateCost("agent1", 1.00);

    // accumulated $5 + $1 = $6, below $8 warn → still no emit
    expect(warnSpy).not.toHaveBeenCalled();
  });

  it("schedules midnight reset via startMidnightReset", () => {
    const resetSpy = vi.fn();
    guard.on("daily_reset", resetSpy);
    guard.startMidnightReset();
    // Advance past 24 hours to ensure midnight fires
    vi.advanceTimersByTime(25 * 60 * 60 * 1000);
    expect(resetSpy).toHaveBeenCalled();
    guard.stop();
  });
});
