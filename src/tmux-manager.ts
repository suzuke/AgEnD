import { execFile } from "node:child_process";
import { promisify } from "node:util";

const exec = promisify(execFile);

export class TmuxManager {
  private windowId: string;

  constructor(private sessionName: string, windowId: string) {
    this.windowId = windowId;
  }

  // === Static session-level methods ===

  static async ensureSession(name: string): Promise<void> {
    if (await TmuxManager.sessionExists(name)) return;
    await exec("tmux", ["new-session", "-d", "-s", name]);
  }

  static async sessionExists(name: string): Promise<boolean> {
    try {
      await exec("tmux", ["has-session", "-t", name]);
      return true;
    } catch { return false; }
  }

  static async killSession(name: string): Promise<void> {
    try { await exec("tmux", ["kill-session", "-t", name]); } catch {}
  }

  static async listWindows(sessionName: string): Promise<string[]> {
    try {
      const { stdout } = await exec("tmux", [
        "list-windows", "-t", sessionName, "-F", "#{window_id}"
      ]);
      return stdout.trim().split("\n").filter(Boolean);
    } catch { return []; }
  }

  // === Instance window methods ===

  async createWindow(command: string, cwd: string): Promise<string> {
    const { stdout } = await exec("tmux", [
      "new-window", "-t", this.sessionName, "-c", cwd,
      "-P", "-F", "#{window_id}", command,
    ]);
    this.windowId = stdout.trim();
    return this.windowId;
  }

  async killWindow(): Promise<void> {
    if (!this.windowId) return;
    try {
      await exec("tmux", ["kill-window", "-t", `${this.sessionName}:${this.windowId}`]);
    } catch {}
  }

  async isWindowAlive(): Promise<boolean> {
    if (!this.windowId) return false;
    try {
      const windows = await TmuxManager.listWindows(this.sessionName);
      return windows.includes(this.windowId);
    } catch { return false; }
  }

  async sendKeys(text: string): Promise<void> {
    await exec("tmux", ["send-keys", "-t", `${this.sessionName}:${this.windowId}`, text]);
  }

  async sendSpecialKey(key: "Enter" | "Escape" | "Up" | "Down"): Promise<void> {
    await exec("tmux", ["send-keys", "-t", `${this.sessionName}:${this.windowId}`, key]);
  }

  async pipeOutput(logPath: string): Promise<void> {
    await exec("tmux", [
      "pipe-pane", "-t", `${this.sessionName}:${this.windowId}`,
      `cat >> ${logPath}`,
    ]);
  }

  async capturePane(): Promise<string> {
    const { stdout } = await exec("tmux", [
      "capture-pane", "-t", `${this.sessionName}:${this.windowId}`, "-p",
    ]);
    return stdout;
  }

  getWindowId(): string { return this.windowId; }
}
