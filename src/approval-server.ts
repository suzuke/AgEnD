/**
 * Approval server for Claude Code PreToolUse hooks.
 *
 * Listens on 127.0.0.1:18321 for tool-approval requests from Claude Code's
 * PreToolUse hook.  Sends an approval prompt to Telegram with inline buttons
 * and waits for the user to press Approve / Deny.
 *
 * Callback-query handling: The Telegram channel plugin (grammy) already holds
 * the long-poll slot on getUpdates for *messages*.  To avoid a 409 conflict we
 * do NOT long-poll ourselves.  Instead we short-poll getUpdates with timeout=0
 * and allowed_updates=["callback_query"] every 1 second.  This is safe because
 * Telegram only rejects *concurrent* long-polls — non-overlapping short-polls
 * interleave fine with grammy's long-poll cycle.
 *
 * NOTE: If this still causes 409s in practice, an alternative is to set a
 *       Telegram webhook for callback_query only, or to use a second bot token.
 */

import { createServer, type Server, type IncomingMessage, type ServerResponse } from "node:http";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";
import type { Logger } from "./logger.js";

const TELEGRAM_ENV = join(homedir(), ".claude", "channels", "telegram", ".env");
const DEFAULT_PORT = 18321;
const APPROVAL_TIMEOUT_MS = 120_000; // 2 minutes

interface HookInput {
  session_id?: string;
  tool_name: string;
  tool_input: Record<string, unknown>;
  tool_use_id?: string;
  hook_event_name?: string;
}

interface PendingApproval {
  hookInput: HookInput;
  resolve: (decision: "allow" | "deny") => void;
  timer: ReturnType<typeof setTimeout>;
  telegramMessageId: number;
}

function loadBotToken(): string | null {
  try {
    for (const line of readFileSync(TELEGRAM_ENV, "utf8").split("\n")) {
      const m = line.match(/^TELEGRAM_BOT_TOKEN=(.+)$/);
      if (m) return m[1];
    }
  } catch {}
  return null;
}

function summarizeTool(input: HookInput): string {
  const { tool_name, tool_input } = input;
  switch (tool_name) {
    case "Bash":
      return `Bash: ${String(tool_input.command ?? "").slice(0, 300)}`;
    case "Write":
      return `Write: ${tool_input.file_path}`;
    case "Edit":
      return `Edit: ${tool_input.file_path}`;
    case "Read":
      return `Read: ${tool_input.file_path}`;
    default:
      return `${tool_name}: ${JSON.stringify(tool_input).slice(0, 200)}`;
  }
}

export class ApprovalServer {
  private server: Server | null = null;
  private pending = new Map<string, PendingApproval>();
  private botToken: string | null = null;
  private chatId: string | null = null;
  private pollTimer: ReturnType<typeof setTimeout> | null = null;
  private callbackOffset = 0;

  constructor(private logger: Logger) {}

  /** Set the Telegram chat ID for sending approval requests. */
  setChatId(chatId: string): void {
    this.chatId = chatId;
  }

  async start(port = DEFAULT_PORT): Promise<void> {
    this.botToken = loadBotToken();
    if (!this.botToken) {
      this.logger.warn("No TELEGRAM_BOT_TOKEN — approval server will auto-allow all requests");
    }

    this.server = createServer((req, res) => this.handleRequest(req, res));

    return new Promise((resolve, reject) => {
      this.server!.listen(port, "127.0.0.1", () => {
        this.logger.info({ port }, "Approval server listening");
        if (this.botToken && this.chatId) {
          this.startCallbackPolling();
        }
        resolve();
      });
      this.server!.on("error", reject);
    });
  }

  async stop(): Promise<void> {
    if (this.pollTimer) {
      clearTimeout(this.pollTimer);
      this.pollTimer = null;
    }
    for (const [id, pending] of this.pending) {
      clearTimeout(pending.timer);
      pending.resolve("deny");
      this.pending.delete(id);
    }
    return new Promise((resolve) => {
      if (!this.server) return resolve();
      this.server.close(() => resolve());
    });
  }

  /** Resolve a pending approval externally (e.g. from PTY output). */
  resolveApproval(requestId: string, decision: "allow" | "deny"): boolean {
    const pending = this.pending.get(requestId);
    if (!pending) return false;
    clearTimeout(pending.timer);
    this.pending.delete(requestId);
    this.updateMessage(pending.telegramMessageId, summarizeTool(pending.hookInput),
      decision === "allow" ? "✅ Approved" : "❌ Denied");
    pending.resolve(decision);
    return true;
  }

  private handleRequest(req: IncomingMessage, res: ServerResponse): void {
    if (req.method !== "POST" || req.url !== "/approve") {
      res.writeHead(404);
      res.end();
      return;
    }

    let body = "";
    req.on("data", (chunk: Buffer) => { body += chunk.toString(); });
    req.on("end", () => {
      this.processApproval(body, res).catch((err) => {
        this.logger.error({ err }, "Approval processing error");
        this.respond(res, "allow", "Approval server error — fail-open");
      });
    });
  }

  private respond(res: ServerResponse, decision: "allow" | "deny", reason: string): void {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        permissionDecision: decision,
        permissionDecisionReason: reason,
      },
    }));
  }

  private async processApproval(body: string, res: ServerResponse): Promise<void> {
    let input: HookInput;
    try {
      input = JSON.parse(body);
    } catch {
      return this.respond(res, "allow", "Invalid hook input — fail-open");
    }

    this.logger.info({ tool: input.tool_name }, "Approval request");

    // No Telegram config → auto-allow
    if (!this.botToken || !this.chatId) {
      return this.respond(res, "allow", "No Telegram config — auto-allow");
    }

    const requestId = input.tool_use_id ?? `req_${Date.now()}`;
    const summary = summarizeTool(input);

    // Send inline keyboard to Telegram
    const msgId = await this.sendApprovalMessage(summary, requestId);

    // Start callback polling if not already running
    if (!this.pollTimer) {
      this.startCallbackPolling();
    }

    // Wait for user decision or timeout
    const decision = await new Promise<"allow" | "deny">((resolve) => {
      const timer = setTimeout(() => {
        this.pending.delete(requestId);
        this.updateMessage(msgId, summary, "⏰ Timeout — auto-denied");
        resolve("deny");
      }, APPROVAL_TIMEOUT_MS);

      this.pending.set(requestId, {
        hookInput: input,
        resolve,
        timer,
        telegramMessageId: msgId,
      });
    });

    this.respond(res, decision, decision === "allow" ? "Approved via Telegram" : "Denied via Telegram");
  }

  private async sendApprovalMessage(summary: string, requestId: string): Promise<number> {
    const text = `🔐 Permission Request\n\n${summary}`;
    const keyboard = {
      inline_keyboard: [[
        { text: "✅ Approve", callback_data: `a:${requestId}` },
        { text: "❌ Deny", callback_data: `d:${requestId}` },
      ]],
    };

    try {
      const res = await fetch(`https://api.telegram.org/bot${this.botToken}/sendMessage`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ chat_id: this.chatId, text, reply_markup: keyboard }),
      });
      const data = await res.json() as { ok: boolean; result?: { message_id: number } };
      return data.result?.message_id ?? 0;
    } catch (err) {
      this.logger.error({ err }, "Failed to send approval message");
      return 0;
    }
  }

  private async updateMessage(messageId: number, summary: string, status: string): Promise<void> {
    if (!messageId) return;
    await fetch(`https://api.telegram.org/bot${this.botToken}/editMessageText`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        chat_id: this.chatId,
        message_id: messageId,
        text: `🔐 Permission Request\n\n${summary}\n\n${status}`,
      }),
    }).catch(() => {});
  }

  /**
   * Short-poll Telegram for callback_query updates (button presses).
   * Uses timeout=0 (non-blocking) to avoid fighting grammy's long-poll.
   */
  private startCallbackPolling(): void {
    const poll = async () => {
      // Stop if server is down or no pending approvals exist (save API calls)
      if (!this.server?.listening) return;
      if (this.pending.size === 0) {
        this.pollTimer = setTimeout(poll, 2000);
        return;
      }

      try {
        const url = `https://api.telegram.org/bot${this.botToken}/getUpdates`
          + `?offset=${this.callbackOffset}&timeout=0&allowed_updates=${encodeURIComponent('["callback_query"]')}`;
        const res = await fetch(url);
        const data = await res.json() as {
          ok: boolean;
          result?: Array<{
            update_id: number;
            callback_query?: {
              id: string;
              data?: string;
              from: { id: number; username?: string };
            };
          }>;
        };

        if (data.ok && data.result) {
          for (const update of data.result) {
            this.callbackOffset = update.update_id + 1;
            const cb = update.callback_query;
            if (!cb?.data) continue;

            // Acknowledge button press
            fetch(`https://api.telegram.org/bot${this.botToken}/answerCallbackQuery`, {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({ callback_query_id: cb.id }),
            }).catch(() => {});

            // Parse: "a:requestId" or "d:requestId"
            const colonIdx = cb.data.indexOf(":");
            if (colonIdx < 0) continue;
            const action = cb.data.slice(0, colonIdx);
            const requestId = cb.data.slice(colonIdx + 1);

            const pending = this.pending.get(requestId);
            if (!pending) continue;

            clearTimeout(pending.timer);
            this.pending.delete(requestId);

            const decision = action === "a" ? "allow" as const : "deny" as const;
            const user = cb.from.username ?? String(cb.from.id);
            this.updateMessage(
              pending.telegramMessageId,
              summarizeTool(pending.hookInput),
              `${decision === "allow" ? "✅ Approved" : "❌ Denied"} by @${user}`,
            );

            pending.resolve(decision);
          }
        }
      } catch (err) {
        this.logger.debug({ err }, "Callback poll error");
      }

      this.pollTimer = setTimeout(poll, 1000);
    };

    poll();
  }
}
