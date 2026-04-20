import { describe, it, expect, beforeEach, vi } from "vitest";
import { EventEmitter } from "node:events";

// ── Mock node:child_process & node:readline so we never spawn real tmux ──
const spawnMock = vi.fn();
const execFileMock = vi.fn();

vi.mock("node:child_process", () => ({
  spawn: (...args: unknown[]) => spawnMock(...args),
  execFile: (...args: unknown[]) => execFileMock(...args),
}));

vi.mock("node:readline", () => ({
  createInterface: vi.fn(() => ({ on: vi.fn(), close: vi.fn() })),
}));

import { TmuxControlClient } from "../src/tmux-control.js";

interface FakeProc extends EventEmitter {
  stdout: EventEmitter;
  stdin: { write: ReturnType<typeof vi.fn> };
  killed: boolean;
  kill: () => void;
}

function makeFakeProc(): FakeProc {
  const proc = new EventEmitter() as FakeProc;
  proc.stdout = new EventEmitter();
  proc.stdin = { write: vi.fn() };
  proc.killed = false;
  proc.kill = () => { proc.killed = true; };
  return proc;
}

interface ClientInternals {
  paneToWindow: Map<string, string>;
  lastOutputAt: Map<string, number>;
  registeredWindows: Set<string>;
  connect: () => void;
}

describe("TmuxControlClient reconnect", () => {
  beforeEach(() => {
    spawnMock.mockReset();
    execFileMock.mockReset();
    spawnMock.mockReturnValue(makeFakeProc());
    // Default: execFile returns empty stdout so resolvePane is a harmless no-op
    execFileMock.mockImplementation((_cmd, _args, cb: (e: Error | null, s: string) => void) => cb(null, ""));
  });

  it("registerWindow tracks the windowId for future reconnects", async () => {
    const client = new TmuxControlClient("s", 100);
    const internal = client as unknown as ClientInternals;

    execFileMock.mockImplementationOnce((_c, _a, cb) => cb(null, "%1"));
    await client.registerWindow("@10");

    expect(internal.registeredWindows.has("@10")).toBe(true);
    expect(internal.paneToWindow.get("%1")).toBe("@10");
  });

  it("unregisterWindow removes from registeredWindows AND pane caches", () => {
    const client = new TmuxControlClient("s", 100);
    const internal = client as unknown as ClientInternals;
    internal.registeredWindows.add("@5");
    internal.paneToWindow.set("%1", "@5");
    internal.lastOutputAt.set("%1", Date.now());

    client.unregisterWindow("@5");

    expect(internal.registeredWindows.has("@5")).toBe(false);
    expect(internal.paneToWindow.has("%1")).toBe(false);
    expect(internal.lastOutputAt.has("%1")).toBe(false);
  });

  it("clears stale paneToWindow / lastOutputAt on reconnect", () => {
    const client = new TmuxControlClient("s", 100);
    const internal = client as unknown as ClientInternals;
    // Simulate state from a prior connection
    internal.paneToWindow.set("%1", "@5");
    internal.lastOutputAt.set("%1", Date.now());
    internal.registeredWindows.add("@5");

    internal.connect();

    expect(internal.paneToWindow.size).toBe(0);
    expect(internal.lastOutputAt.size).toBe(0);
    // Registered windows are preserved so resolvePane can refresh them
    expect(internal.registeredWindows.has("@5")).toBe(true);
  });

  it("re-resolves all registered windows after reconnect", async () => {
    const client = new TmuxControlClient("s", 100);
    const internal = client as unknown as ClientInternals;
    internal.registeredWindows.add("@5");
    internal.registeredWindows.add("@7");

    const seen: string[] = [];
    execFileMock.mockImplementation((_cmd, args: string[], cb: (e: Error | null, s: string) => void) => {
      seen.push(args.join(" "));
      cb(null, "%99");
    });

    internal.connect();
    // Allow the fired-and-forget resolvePane promises to settle
    await new Promise(r => setImmediate(r));

    expect(seen.some(s => s.includes("@5"))).toBe(true);
    expect(seen.some(s => s.includes("@7"))).toBe(true);
  });
});
