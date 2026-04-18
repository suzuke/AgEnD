import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { MessageQueue } from "../src/channel/message-queue.js";

// Build a sender that fails with a 429 a fixed number of times, then succeeds.
function makeSender(failCount: number) {
  const err: Error & { status?: number } = new Error("429 Too Many Requests");
  err.status = 429;
  let failures = 0;
  const sendSpy = vi.fn(async (_chatId: string, _threadId: string | undefined, _text: string) => {
    if (failures < failCount) {
      failures++;
      throw err;
    }
    return { messageId: "m1" };
  });
  return {
    send: sendSpy,
    edit: vi.fn(async () => {}),
    sendFile: vi.fn(async () => ({ messageId: "f1" })),
  };
}

describe("MessageQueue flood control reset (P3.8)", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("resets backoffMs after flood control drops status_update items", async () => {
    // fail 5 times — backoff goes 1s → 2s → 4s → 8s → 16s → 32s (capped 30s),
    // crossing the 10s flood-control threshold after attempt #4.
    const sender = makeSender(5);
    const q = new MessageQueue(sender, { warn: vi.fn() });
    q.start();

    // Enqueue several status_updates; they'll 429 repeatedly.
    for (let i = 0; i < 3; i++) {
      q.enqueue("chat1", undefined, { type: "status_update", text: `s${i}` } as never);
    }

    // Drive the worker through all 5 failures + backoff waits. Each backoff
    // is capped at 100ms of actual sleep per tick inside runWorker, so we
    // advance generously.
    await vi.advanceTimersByTimeAsync(60_000);

    // Peek at internal state — flood control should have reset backoff.
    const state = (q as unknown as { queues: Map<string, { backoffMs: number; items: unknown[] }> })
      .queues.get("chat1:");
    expect(state).toBeDefined();
    // INITIAL_BACKOFF_MS is 1_000 — after flood control runs, we expect that.
    expect(state!.backoffMs).toBe(1_000);

    q.stop();
  });

  it("does not reset backoff when there are no status_updates to drop", async () => {
    // Content items are not dropped by flood control; backoff should keep
    // growing (until success or stop).
    const sender = makeSender(100); // essentially always fails
    const q = new MessageQueue(sender, { warn: vi.fn() });
    q.start();

    q.enqueue("chat2", undefined, { type: "content", text: "keep me" } as never);

    await vi.advanceTimersByTimeAsync(60_000);

    const state = (q as unknown as { queues: Map<string, { backoffMs: number }> })
      .queues.get("chat2:");
    expect(state).toBeDefined();
    // Content items persist through flood control, so backoff is NOT reset;
    // it should have grown well past the initial value.
    expect(state!.backoffMs).toBeGreaterThan(1_000);

    q.stop();
  });
});
