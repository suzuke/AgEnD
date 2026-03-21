import { describe, it, expect, afterAll } from "vitest";
import { TmuxManager } from "../src/tmux-manager.js";

describe("TmuxManager", () => {
  const session = `ccd-test-${Date.now()}`;

  afterAll(async () => {
    await TmuxManager.killSession(session);
  });

  it("creates and detects session", async () => {
    await TmuxManager.ensureSession(session);
    expect(await TmuxManager.sessionExists(session)).toBe(true);
  });

  it("creates window and checks alive", async () => {
    const tm = new TmuxManager(session, "");
    const windowId = await tm.createWindow("sleep 30", "/tmp");
    expect(windowId).toMatch(/@\d+/);
    expect(await tm.isWindowAlive()).toBe(true);
  });

  it("sends keys and captures pane", async () => {
    const tm = new TmuxManager(session, "");
    await tm.createWindow("cat", "/tmp");
    await tm.sendKeys("hello world");
    await tm.sendSpecialKey("Enter");
    await new Promise(r => setTimeout(r, 500));
    const output = await tm.capturePane();
    expect(output).toContain("hello world");
  });

  it("kills window", async () => {
    const tm = new TmuxManager(session, "");
    const wid = await tm.createWindow("sleep 30", "/tmp");
    await tm.killWindow();
    await new Promise(r => setTimeout(r, 200));
    expect(await tm.isWindowAlive()).toBe(false);
  });

  it("lists windows", async () => {
    const tm = new TmuxManager(session, "");
    await tm.createWindow("sleep 30", "/tmp");
    const windows = await TmuxManager.listWindows(session);
    expect(windows.length).toBeGreaterThan(0);
  });
});
