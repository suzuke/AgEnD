import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { EventEmitter } from "node:events";
import { GrammyError } from "grammy";
import { runBotWithConflictRetry } from "../src/channel/adapters/telegram.js";

function conflictError(): GrammyError {
  // GrammyError requires (message, payload, method). We only care about
  // error_code in the retry loop, so shape just enough of the payload.
  return new GrammyError("Conflict", { ok: false, error_code: 409, description: "Conflict" }, "getUpdates", {});
}

describe("runBotWithConflictRetry (P3.2)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("retries on 409 and emits polling_conflict with backoff metadata", async () => {
    const ee = new EventEmitter();
    const conflicts: Array<{ attempt: number; delay: number }> = [];
    ee.on("polling_conflict", (p) => conflicts.push(p));

    let calls = 0;
    const startFn = vi.fn(async () => {
      calls++;
      if (calls < 3) throw conflictError();
      // On the third attempt, succeed (bot.stop() equivalent: return).
    });

    const done = runBotWithConflictRetry(startFn, ee, {
      maxAttempts: 30,
      backoffMs: () => 1, // tiny backoff so the test doesn't have to advance much
    });

    // Drain the pending backoff timers. Each failed attempt awaits setTimeout(1).
    await vi.advanceTimersByTimeAsync(10);
    await done;

    expect(startFn).toHaveBeenCalledTimes(3);
    expect(conflicts).toHaveLength(2);
    expect(conflicts[0]).toMatchObject({ attempt: 1 });
    expect(conflicts[1]).toMatchObject({ attempt: 2 });
  });

  it("escalates to polling_conflict_fatal after maxAttempts", async () => {
    const ee = new EventEmitter();
    const fatal = vi.fn();
    const errorEv = vi.fn();
    ee.on("polling_conflict_fatal", fatal);
    ee.on("error", errorEv);

    const startFn = vi.fn(async () => { throw conflictError(); });

    const done = runBotWithConflictRetry(startFn, ee, {
      maxAttempts: 3,
      backoffMs: () => 1,
    });

    await vi.advanceTimersByTimeAsync(20);
    await done;

    expect(startFn).toHaveBeenCalledTimes(3);
    expect(fatal).toHaveBeenCalledWith({ attempts: 3 });
    // error must also fire so operator-facing logs see the final GrammyError.
    expect(errorEv).toHaveBeenCalledTimes(1);
  });

  it("exits cleanly on the grammy 'Aborted delay' shutdown error", async () => {
    const ee = new EventEmitter();
    const errorEv = vi.fn();
    ee.on("error", errorEv);

    const startFn = vi.fn(async () => { throw new Error("Aborted delay"); });

    await runBotWithConflictRetry(startFn, ee);

    expect(startFn).toHaveBeenCalledTimes(1);
    expect(errorEv).not.toHaveBeenCalled();
  });

  it("emits 'error' and stops on non-409 GrammyError (no infinite retry)", async () => {
    const ee = new EventEmitter();
    const errorEv = vi.fn();
    ee.on("error", errorEv);

    const startFn = vi.fn(async () => {
      throw new GrammyError("Unauthorized", { ok: false, error_code: 401, description: "Unauthorized" }, "getMe", {});
    });

    await runBotWithConflictRetry(startFn, ee);

    expect(startFn).toHaveBeenCalledTimes(1);
    expect(errorEv).toHaveBeenCalledTimes(1);
  });

  it("first attempt drops pending updates; retries do not", async () => {
    const ee = new EventEmitter();
    const flags: boolean[] = [];
    let calls = 0;
    const startFn = vi.fn(async (dropPending: boolean) => {
      flags.push(dropPending);
      calls++;
      if (calls < 2) throw conflictError();
    });

    const done = runBotWithConflictRetry(startFn, ee, {
      maxAttempts: 5,
      backoffMs: () => 1,
    });
    await vi.advanceTimersByTimeAsync(10);
    await done;

    expect(flags).toEqual([true, false]);
  });
});
