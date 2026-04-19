import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { EventEmitter } from "node:events";
import { Readable, Writable } from "node:stream";

// Stub node:child_process.spawn so we never launch a real tmux control
// process in tests. Each spawn() call returns a MockProc we can close at
// will to drive the real close handler in TmuxControlClient.connect().
class MockProc extends EventEmitter {
  stdout = new Readable({ read() {} });
  stderr = new Readable({ read() {} });
  stdin = new Writable({ write(_c, _e, cb) { cb(); } });
  killed = false;
  kill() { this.killed = true; }
}

const spawned: MockProc[] = [];
vi.mock("node:child_process", async () => {
  const actual = await vi.importActual<typeof import("node:child_process")>("node:child_process");
  return {
    ...actual,
    spawn: vi.fn(() => {
      const p = new MockProc();
      spawned.push(p);
      return p;
    }),
  };
});

// Imported after vi.mock so the mock is in place.
const { TmuxControlClient } = await import("../src/tmux-control.js");

/**
 * Regression test for P2.1: when the control-mode proc closes, the real
 * close handler in connect() must clear paneToWindow + lastOutputAt
 * (recycled pane IDs) and on successful reconnect the client must emit
 * "reconnected" so FleetManager can re-register live windows.
 */
describe("TmuxControlClient reconnect", () => {
  beforeEach(() => {
    spawned.length = 0;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "setImmediate", "clearImmediate"] });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("close handler wipes pane maps; reconnect emits 'reconnected'", async () => {
    const client = new TmuxControlClient("agend-test", 2000);
    const inner = client as unknown as {
      paneToWindow: Map<string, string>;
      lastOutputAt: Map<string, number>;
    };

    client.start();
    expect(spawned).toHaveLength(1);

    // Seed state as if a window were registered and producing output.
    inner.paneToWindow.set("%1", "@5");
    inner.lastOutputAt.set("%1", Date.now());

    // Listener must be attached BEFORE the reconnect fires.
    const reconnected = new Promise<void>((resolve) => {
      client.once("reconnected", () => resolve());
    });

    // Drop the first connection — triggers the real close handler, which
    // should clear maps AND schedule a reconnect via setTimeout(2000).
    spawned[0].emit("close");
    expect(inner.paneToWindow.size).toBe(0);
    expect(inner.lastOutputAt.size).toBe(0);

    // Fire the reconnect timer; connect() spawns a second mock and, because
    // hadPreviousConnection is true, schedules emit("reconnected") via
    // setImmediate.
    await vi.runOnlyPendingTimersAsync();
    expect(spawned).toHaveLength(2);
    await vi.runOnlyPendingTimersAsync();
    await reconnected;

    client.stop();
  });

  it("isIdle returns true for unknown window after reconnect wipe", () => {
    const client = new TmuxControlClient("agend-test", 2000);
    // Without any registered window, isIdle must not block the caller.
    expect(client.isIdle("@99")).toBe(true);
  });
});
