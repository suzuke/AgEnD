import { spawn, execFile, type ChildProcess } from "node:child_process";
import { EventEmitter } from "node:events";
import { createInterface, type Interface } from "node:readline";
import type { Logger } from "./logger.js";
import { getTmuxSocketName } from "./paths.js";

function tmuxArgs(args: string[]): string[] {
  const socket = getTmuxSocketName();
  return socket ? ["-L", socket, ...args] : args;
}

function execTmux(args: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile("tmux", tmuxArgs(args), (err, stdout) => {
      if (err) reject(err);
      else resolve(stdout.trim());
    });
  });
}

/**
 * Persistent tmux control mode client that monitors %output events
 * to detect per-pane idle state. One instance per tmux session.
 *
 * Usage:
 *   const ctrl = new TmuxControlClient("agend", 2000, logger);
 *   ctrl.start();
 *   await ctrl.waitForIdle("@5");  // wait until window @5 is idle
 *   tmux.pasteText(msg);
 */
export class TmuxControlClient extends EventEmitter {
  private proc: ChildProcess | null = null;
  private rl: Interface | null = null;
  private lastOutputAt = new Map<string, number>(); // paneId → timestamp
  private paneToWindow = new Map<string, string>();  // paneId → windowId
  private registeredWindows = new Set<string>();    // windowIds we should re-resolve on reconnect
  private stopped = false;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private sessionName: string,
    private silenceMs: number = 2000,
    private logger?: Logger,
  ) {
    super();
  }

  start(): void {
    this.stopped = false;
    this.connect();
  }

  // PLACEHOLDER_REST

  stop(): void {
    this.stopped = true;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.cleanup();
  }

  /**
   * Register a window so we can track its pane's output.
   * Call this after createWindow().
   */
  async registerWindow(windowId: string): Promise<void> {
    this.registeredWindows.add(windowId);
    await this.resolvePane(windowId);
  }

  /** Unregister a window (call on killWindow) */
  unregisterWindow(windowId: string): void {
    this.registeredWindows.delete(windowId);
    for (const [pane, win] of this.paneToWindow) {
      if (win === windowId) {
        this.paneToWindow.delete(pane);
        this.lastOutputAt.delete(pane);
        break;
      }
    }
  }

  /** Resolve a window's current pane id and cache the mapping. */
  private async resolvePane(windowId: string): Promise<void> {
    try {
      const paneId = await execTmux([
        "list-panes", "-t", `${this.sessionName}:${windowId}`,
        "-F", "#{pane_id}",
      ]);
      if (paneId) {
        this.paneToWindow.set(paneId, windowId);
        this.logger?.debug({ windowId, paneId }, "Registered window→pane mapping");
      }
    } catch {
      this.logger?.debug({ windowId }, "Failed to resolve pane ID for window");
    }
  }

  /** Check if a window's pane has been silent for at least silenceMs */
  isIdle(windowId: string): boolean {
    const paneId = this.windowToPaneId(windowId);
    if (!paneId) return true; // unknown window = assume idle
    const last = this.lastOutputAt.get(paneId);
    if (last == null) return true;
    return Date.now() - last >= this.silenceMs;
  }

  // PLACEHOLDER_WAIT

  /**
   * Wait until a window's pane is idle (no output for silenceMs).
   * Returns true if idle detected, false if timeout reached.
   */
  waitForIdle(windowId: string, timeoutMs = 30_000): Promise<boolean> {
    if (this.isIdle(windowId)) return Promise.resolve(true);

    return new Promise((resolve) => {
      const check = setInterval(() => {
        if (this.isIdle(windowId)) {
          clearInterval(check);
          clearTimeout(timer);
          resolve(true);
        }
      }, 200);

      const timer = setTimeout(() => {
        clearInterval(check);
        this.logger?.warn({ windowId, timeoutMs }, "waitForIdle timed out — forcing delivery");
        resolve(false);
      }, timeoutMs);
    });
  }

  /**
   * Wait until a window's pane produces any output.
   * Used to verify CLI startup — if no output within timeout, CLI likely failed.
   */
  waitForOutput(windowId: string, timeoutMs = 15_000): Promise<boolean> {
    const paneId = this.windowToPaneId(windowId);
    // If already has output recorded, it's alive
    if (paneId && this.lastOutputAt.has(paneId)) return Promise.resolve(true);

    return new Promise((resolve) => {
      const check = setInterval(() => {
        const pid = this.windowToPaneId(windowId);
        if (pid && this.lastOutputAt.has(pid)) {
          clearInterval(check);
          clearTimeout(timer);
          resolve(true);
        }
      }, 300);

      const timer = setTimeout(() => {
        clearInterval(check);
        resolve(false);
      }, timeoutMs);
    });
  }

  private windowToPaneId(windowId: string): string | undefined {
    for (const [pane, win] of this.paneToWindow) {
      if (win === windowId) return pane;
    }
    return undefined;
  }

  private connect(): void {
    if (this.stopped) return;

    // Pane IDs are tmux-server-scoped: a server restart (or a long-enough
    // disconnect that windows churned) can leave our cached paneId →
    // windowId mapping pointing at a stale or recycled pane. Drop the
    // cache and re-resolve every registered window from the new server.
    this.paneToWindow.clear();
    this.lastOutputAt.clear();

    this.proc = spawn("tmux", tmuxArgs(["-C", "attach", "-t", this.sessionName, "-r"]), {
      stdio: ["pipe", "pipe", "pipe"],
    });

    this.rl = createInterface({ input: this.proc.stdout! });
    this.rl.on("line", (line) => this.parseLine(line));

    this.proc.on("close", () => {
      this.cleanup();
      if (!this.stopped) {
        this.logger?.debug("Control mode disconnected — reconnecting in 2s");
        this.reconnectTimer = setTimeout(() => this.connect(), 2000);
      }
    });

    this.proc.on("error", (err) => {
      this.logger?.warn({ err: (err as Error).message }, "Control mode spawn error");
    });

    // Re-resolve panes for any windows that were registered before this
    // (re)connect. Safe even on first connect: registeredWindows is empty.
    for (const windowId of this.registeredWindows) {
      void this.resolvePane(windowId);
    }

    this.logger?.debug("tmux control mode connected");
  }

  private parseLine(line: string): void {
    if (line.startsWith("%output ")) {
      const match = line.match(/^%output (%\d+) /);
      if (match) {
        this.lastOutputAt.set(match[1], Date.now());
      }
    }
  }

  private cleanup(): void {
    this.rl?.close();
    this.rl = null;
    if (this.proc && !this.proc.killed) {
      this.proc.stdin?.write("detach\n");
      this.proc.kill();
    }
    this.proc = null;
  }
}
