import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { createHmac } from "node:crypto";
import { WebhookEmitter, parseRetryAfter } from "../src/webhook-emitter.js";
import type { WebhookConfig } from "../src/types.js";

const makeLogger = () => ({
  info: vi.fn(),
  debug: vi.fn(),
  warn: vi.fn(),
  error: vi.fn(),
}) as never;

describe("WebhookEmitter (P3.1)", () => {
  let fetchMock: ReturnType<typeof vi.fn>;
  const originalFetch = globalThis.fetch;

  beforeEach(() => {
    fetchMock = vi.fn();
    // @ts-expect-error override
    globalThis.fetch = fetchMock;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.useRealTimers();
  });

  it("signs body with HMAC-SHA256 when secret is set", async () => {
    fetchMock.mockResolvedValue(new Response(null, { status: 200 }));
    const cfg: WebhookConfig = { url: "http://x/y", events: ["*"], secret: "s3cr3t" };
    const e = new WebhookEmitter([cfg], makeLogger());
    e.emit("test", "agent1", { k: "v" });
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));

    const [, init] = fetchMock.mock.calls[0];
    const sent = init.body as string;
    const expected = createHmac("sha256", "s3cr3t").update(sent).digest("hex");
    expect(init.headers["X-Agend-Signature"]).toBe(`sha256=${expected}`);
  });

  it("omits signature header when no secret is configured", async () => {
    fetchMock.mockResolvedValue(new Response(null, { status: 200 }));
    const cfg: WebhookConfig = { url: "http://x/y", events: ["*"] };
    const e = new WebhookEmitter([cfg], makeLogger());
    e.emit("test", "agent1");
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const [, init] = fetchMock.mock.calls[0];
    expect(init.headers["X-Agend-Signature"]).toBeUndefined();
  });

  it("retries on 5xx up to max_attempts and stops after success", async () => {
    vi.useFakeTimers();
    fetchMock
      .mockResolvedValueOnce(new Response(null, { status: 503 }))
      .mockResolvedValueOnce(new Response(null, { status: 200 }));
    const cfg: WebhookConfig = { url: "http://x/y", events: ["*"], max_attempts: 3 };
    const e = new WebhookEmitter([cfg], makeLogger());
    e.emit("test", "agent1");
    // First attempt fires synchronously within the microtask queue
    await vi.advanceTimersByTimeAsync(0);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("does not retry 4xx responses", async () => {
    vi.useFakeTimers();
    fetchMock.mockResolvedValue(new Response(null, { status: 400 }));
    const cfg: WebhookConfig = { url: "http://x/y", events: ["*"], max_attempts: 3 };
    const e = new WebhookEmitter([cfg], makeLogger());
    e.emit("test", "agent1");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("retries on 429 and honours Retry-After delta-seconds", async () => {
    vi.useFakeTimers();
    const now = Date.now();
    vi.setSystemTime(now);
    fetchMock
      .mockResolvedValueOnce(new Response(null, { status: 429, headers: { "Retry-After": "2" } }))
      .mockResolvedValueOnce(new Response(null, { status: 200 }));
    const cfg: WebhookConfig = { url: "http://x/y", events: ["*"], max_attempts: 3 };
    const e = new WebhookEmitter([cfg], makeLogger());
    e.emit("test", "agent1");
    await vi.advanceTimersByTimeAsync(0);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    // Retry-After: 2 means wait 2000ms — 1000ms is not enough.
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("falls back to exponential backoff when 429 has no Retry-After", async () => {
    vi.useFakeTimers();
    fetchMock
      .mockResolvedValueOnce(new Response(null, { status: 429 }))
      .mockResolvedValueOnce(new Response(null, { status: 200 }));
    const cfg: WebhookConfig = { url: "http://x/y", events: ["*"], max_attempts: 3 };
    const e = new WebhookEmitter([cfg], makeLogger());
    e.emit("test", "agent1");
    await vi.advanceTimersByTimeAsync(0);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    // Default backoff on attempt 1 = 1000ms (2^0 * 1000).
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("caps Retry-After at 60s to prevent denial-of-wallet", async () => {
    vi.useFakeTimers();
    fetchMock
      .mockResolvedValueOnce(new Response(null, { status: 429, headers: { "Retry-After": "3600" } }))
      .mockResolvedValueOnce(new Response(null, { status: 200 }));
    const cfg: WebhookConfig = { url: "http://x/y", events: ["*"], max_attempts: 3 };
    const e = new WebhookEmitter([cfg], makeLogger());
    e.emit("test", "agent1");
    await vi.advanceTimersByTimeAsync(0);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    // Server asked for 1h but we cap at 60s.
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("parseRetryAfter accepts delta-seconds and HTTP-date", () => {
    expect(parseRetryAfter("120")).toBe(120_000);
    expect(parseRetryAfter(null)).toBeNull();
    expect(parseRetryAfter("garbage")).toBeNull();
    // HTTP-date: exercise the branch by passing a date ~5s in the future.
    const future = new Date(Date.now() + 5000).toUTCString();
    const ms = parseRetryAfter(future);
    expect(ms).not.toBeNull();
    expect(ms!).toBeGreaterThanOrEqual(0);
    expect(ms!).toBeLessThanOrEqual(5000);
  });

  it("caps retries at max_attempts", async () => {
    vi.useFakeTimers();
    fetchMock.mockRejectedValue(new Error("boom"));
    const cfg: WebhookConfig = { url: "http://x/y", events: ["*"], max_attempts: 2 };
    const e = new WebhookEmitter([cfg], makeLogger());
    e.emit("test", "agent1");
    await vi.advanceTimersByTimeAsync(0);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    // No third attempt.
    await vi.advanceTimersByTimeAsync(10_000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });
});
