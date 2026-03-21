import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { ApprovalServer } from "../src/approval-server.js";

function makeLogger() {
  return {
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
    debug: vi.fn(),
  } as any;
}

describe("ApprovalServer", () => {
  let server: ApprovalServer;
  let logger: ReturnType<typeof makeLogger>;
  const PORT = 19321; // avoid conflict with real daemon

  beforeEach(() => {
    logger = makeLogger();
    server = new ApprovalServer(logger);
  });

  afterEach(async () => {
    await server.stop();
  });

  it("starts and listens on the specified port", async () => {
    await server.start(PORT);
    const res = await fetch(`http://127.0.0.1:${PORT}/approve`, { method: "GET" });
    expect(res.status).toBe(404); // GET not supported, but server is up
  });

  it("auto-allows when no Telegram config is set", async () => {
    await server.start(PORT);

    const hookInput = {
      tool_name: "Bash",
      tool_input: { command: "ls -la" },
      tool_use_id: "test_001",
    };

    const res = await fetch(`http://127.0.0.1:${PORT}/approve`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(hookInput),
    });

    const data = await res.json();
    expect(data.hookSpecificOutput.permissionDecision).toBe("allow");
  });

  it("auto-allows on invalid JSON input", async () => {
    await server.start(PORT);

    const res = await fetch(`http://127.0.0.1:${PORT}/approve`, {
      method: "POST",
      body: "not json",
    });

    const data = await res.json();
    expect(data.hookSpecificOutput.permissionDecision).toBe("allow");
    expect(data.hookSpecificOutput.permissionDecisionReason).toContain("fail-open");
  });

  it("resolveApproval resolves a pending request", async () => {
    // Mock fetch for Telegram API — must handle all Telegram URLs
    const originalFetch = globalThis.fetch;
    globalThis.fetch = vi.fn(async (url: string | URL | Request, init?: any) => {
      const urlStr = typeof url === "string" ? url : url instanceof URL ? url.toString() : url.url;
      if (urlStr.includes("api.telegram.org")) {
        return {
          ok: true,
          json: async () => ({ ok: true, result: urlStr.includes("getUpdates") ? [] : { message_id: 42 } }),
        } as any;
      }
      return originalFetch(url, init);
    }) as any;

    try {
      await server.start(PORT);
      server.setChatId("12345");
      // Force bot token for testing
      (server as any).botToken = "fake:token";

      const hookInput = {
        tool_name: "Bash",
        tool_input: { command: "rm test.txt" },
        tool_use_id: "test_resolve",
      };

      // Start the approval request in background (use originalFetch for localhost)
      const approvalPromise = originalFetch(`http://127.0.0.1:${PORT}/approve`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(hookInput),
      });

      // Wait for the pending approval to be registered
      await new Promise(r => setTimeout(r, 500));

      // Resolve it externally
      const resolved = server.resolveApproval("test_resolve", "allow");
      expect(resolved).toBe(true);

      const res = await approvalPromise;
      const data = await res.json() as any;
      expect(data.hookSpecificOutput.permissionDecision).toBe("allow");
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  it("returns 404 for non-/approve paths", async () => {
    await server.start(PORT);
    const res = await fetch(`http://127.0.0.1:${PORT}/other`);
    expect(res.status).toBe(404);
  });
});
