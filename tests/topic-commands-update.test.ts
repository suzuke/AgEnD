import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { TopicCommands } from "../src/topic-commands.js";
import type { FleetContext } from "../src/fleet-context.js";
import type { InboundMessage } from "../src/channel/types.js";

interface SentText { chatId: string; text: string; threadId?: string }

class TestableTopicCommands extends TopicCommands {
  public execLog: Array<{ cmd: string; opts: { timeout: number } }> = [];
  public execResponses: Array<"ok" | "fail"> = [];
  public mockedVersion = "1.20.0";

  constructor(ctx: FleetContext) {
    super(ctx);
    this.exec = (cmd: string, opts: { timeout: number }) => {
      this.execLog.push({ cmd, opts });
      const r = this.execResponses.shift() ?? "ok";
      if (r === "fail") return Promise.reject(new Error("exec failed"));
      return Promise.resolve(undefined);
    };
  }

  protected override readCurrentVersion(): string { return this.mockedVersion; }

  /** Expose for tests. */
  public callHandle(msg: InboundMessage): Promise<boolean> {
    return this.handleGeneralCommand(msg);
  }
}

function makeCtx(allowedUsers: (number | string)[]): { ctx: FleetContext; sent: SentText[] } {
  const sent: SentText[] = [];
  const ctx = {
    adapter: {
      sendText: async (chatId: string, text: string, opts?: { threadId?: string }) => {
        sent.push({ chatId, text, threadId: opts?.threadId });
      },
    },
    fleetConfig: {
      channel: { access: { allowed_users: allowedUsers } },
    },
    dataDir: "/tmp/test-update",
  } as unknown as FleetContext;
  return { ctx, sent };
}

function msg(text: string, userId = "u1"): InboundMessage {
  return {
    chatId: "c1", messageId: "m1", userId, username: "alice",
    text, timestamp: new Date(), source: "telegram", threadId: undefined,
  } as InboundMessage;
}

describe("/update safety (P3.6)", () => {
  beforeEach(() => { vi.useFakeTimers({ shouldAdvanceTime: true }); });
  afterEach(() => { vi.useRealTimers(); });

  it("refuses when allowed_users is empty (no privilege escalation default)", async () => {
    const { ctx, sent } = makeCtx([]);
    const tc = new TestableTopicCommands(ctx);
    await tc.callHandle(msg("/update"));
    expect(tc.execLog).toHaveLength(0);
    expect(sent[0].text).toContain("disabled");
  });

  it("refuses non-allow-listed user", async () => {
    const { ctx, sent } = makeCtx(["alice-id"]);
    const tc = new TestableTopicCommands(ctx);
    await tc.callHandle(msg("/update", "intruder"));
    expect(tc.execLog).toHaveLength(0);
    expect(sent[0].text).toContain("Not authorized");
  });

  it("first /update only registers a pending request (no install yet)", async () => {
    const { ctx, sent } = makeCtx(["u1"]);
    const tc = new TestableTopicCommands(ctx);
    await tc.callHandle(msg("/update"));
    expect(tc.execLog).toHaveLength(0);
    expect(sent[0].text).toContain("Pending update");
    expect(sent[0].text).toMatch(/Current: 1\.20\.0/);
    expect(sent[0].text).toMatch(/Target: {2}@latest/);
    expect(sent[0].text).toMatch(/\/update confirm [0-9a-f]{8}/);
  });

  it("rejects /update confirm with no pending request", async () => {
    const { ctx, sent } = makeCtx(["u1"]);
    const tc = new TestableTopicCommands(ctx);
    await tc.callHandle(msg("/update confirm abcdef"));
    expect(tc.execLog).toHaveLength(0);
    expect(sent[0].text).toContain("No pending");
  });

  it("rejects wrong confirmation token", async () => {
    const { ctx, sent } = makeCtx(["u1"]);
    const tc = new TestableTopicCommands(ctx);
    await tc.callHandle(msg("/update"));
    await tc.callHandle(msg("/update confirm wrongtok"));
    expect(tc.execLog).toHaveLength(0);
    expect(sent[1].text).toContain("Wrong confirmation token");
  });

  it("rejects expired confirmation token", async () => {
    const { ctx, sent } = makeCtx(["u1"]);
    const tc = new TestableTopicCommands(ctx);
    await tc.callHandle(msg("/update"));
    const tokenLine = sent[0].text.match(/\/update confirm ([0-9a-f]{8})/);
    expect(tokenLine).not.toBeNull();
    vi.advanceTimersByTime(61_000);
    await tc.callHandle(msg(`/update confirm ${tokenLine![1]}`));
    expect(tc.execLog).toHaveLength(0);
    expect(sent.at(-1)!.text).toContain("expired");
  });

  it("only the originator can confirm", async () => {
    const { ctx, sent } = makeCtx(["u1", "u2"]);
    const tc = new TestableTopicCommands(ctx);
    await tc.callHandle(msg("/update", "u1"));
    const token = sent[0].text.match(/\/update confirm ([0-9a-f]{8})/)![1];
    await tc.callHandle(msg(`/update confirm ${token}`, "u2"));
    expect(tc.execLog).toHaveLength(0);
    expect(sent.at(-1)!.text).toContain("Only the user");
  });

  it("/update <bad-version> is rejected without registering pending", async () => {
    const { ctx, sent } = makeCtx(["u1"]);
    const tc = new TestableTopicCommands(ctx);
    await tc.callHandle(msg("/update notasemver"));
    expect(sent[0].text).toContain("Invalid version");
    // No pending → confirm with any token should say "no pending"
    await tc.callHandle(msg("/update confirm abc123"));
    expect(sent[1].text).toContain("No pending");
  });

  it("/update <semver> + valid confirm runs npm install with pinned version", async () => {
    const { ctx, sent } = makeCtx(["u1"]);
    const tc = new TestableTopicCommands(ctx);
    tc.execResponses = ["ok", "ok"]; // npm install + agend --version probe
    await tc.callHandle(msg("/update 1.22.5"));
    const token = sent[0].text.match(/\/update confirm ([0-9a-f]{8})/)![1];
    await tc.callHandle(msg(`/update confirm ${token}`));
    // First exec is npm install with pinned version
    expect(tc.execLog[0].cmd).toBe("npm install -g @suzuke/agend@1.22.5");
    // Second is the health probe
    expect(tc.execLog[1].cmd).toBe("agend --version");
  });

  it("rolls back when post-install probe fails", async () => {
    const { ctx, sent } = makeCtx(["u1"]);
    const tc = new TestableTopicCommands(ctx);
    tc.mockedVersion = "1.20.0";
    // npm install OK, probe FAIL, rollback OK
    tc.execResponses = ["ok", "fail", "ok"];
    await tc.callHandle(msg("/update 1.99.0"));
    const token = sent[0].text.match(/\/update confirm ([0-9a-f]{8})/)![1];
    await tc.callHandle(msg(`/update confirm ${token}`));
    expect(tc.execLog[0].cmd).toBe("npm install -g @suzuke/agend@1.99.0");
    expect(tc.execLog[1].cmd).toBe("agend --version");
    expect(tc.execLog[2].cmd).toBe("npm install -g @suzuke/agend@1.20.0");
    expect(sent.some(s => s.text.includes("Rolling back"))).toBe(true);
    expect(sent.some(s => s.text.includes("Rolled back to 1.20.0"))).toBe(true);
  });

  it("/update cancel clears pending", async () => {
    const { ctx, sent } = makeCtx(["u1"]);
    const tc = new TestableTopicCommands(ctx);
    await tc.callHandle(msg("/update"));
    const token = sent[0].text.match(/\/update confirm ([0-9a-f]{8})/)![1];
    await tc.callHandle(msg("/update cancel"));
    expect(sent.at(-1)!.text).toContain("cancelled");
    await tc.callHandle(msg(`/update confirm ${token}`));
    expect(tc.execLog).toHaveLength(0);
    expect(sent.at(-1)!.text).toContain("No pending");
  });

  it("notifies when a second user's /update supersedes a pending request", async () => {
    const { ctx, sent } = makeCtx(["u1", "u2"]);
    const tc = new TestableTopicCommands(ctx);
    await tc.callHandle(msg("/update", "u1"));
    const oldToken = sent[0].text.match(/\/update confirm ([0-9a-f]{8})/)![1];
    await tc.callHandle(msg("/update", "u2"));
    // u2's interaction should mention that u1's pending was superseded.
    const supersedeNotice = sent.find(s => s.text.includes("superseded"));
    expect(supersedeNotice).toBeDefined();
    expect(supersedeNotice!.text).toContain("u1");
    // u1's old token must no longer work.
    await tc.callHandle(msg(`/update confirm ${oldToken}`, "u1"));
    expect(tc.execLog).toHaveLength(0);
    expect(sent.at(-1)!.text).toMatch(/Wrong confirmation token|Only the user/);
  });

  it("token is single-use", async () => {
    const { ctx, sent } = makeCtx(["u1"]);
    const tc = new TestableTopicCommands(ctx);
    tc.execResponses = ["ok", "ok", "ok"]; // install + probe + (optional) launchctl/systemctl
    await tc.callHandle(msg("/update"));
    const token = sent[0].text.match(/\/update confirm ([0-9a-f]{8})/)![1];
    await tc.callHandle(msg(`/update confirm ${token}`));
    // First two execs are deterministic; a third (restart) may or may not run depending on host.
    expect(tc.execLog[0].cmd).toBe("npm install -g @suzuke/agend@latest");
    expect(tc.execLog[1].cmd).toBe("agend --version");
    const execsAfterFirstUse = tc.execLog.length;
    // Replaying same token should fail because pending was cleared on use.
    await tc.callHandle(msg(`/update confirm ${token}`));
    expect(tc.execLog).toHaveLength(execsAfterFirstUse); // no new exec on replay
    expect(sent.at(-1)!.text).toContain("No pending");
  });
});
