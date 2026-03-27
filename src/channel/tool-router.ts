import { homedir } from "node:os";
import { resolve, sep } from "node:path";
import { realpathSync, existsSync } from "node:fs";
import type { ChannelAdapter } from "./types.js";

const STATE_DIR = resolve(homedir(), ".claude-channel-daemon") + sep;
const INBOX_SEG = sep + "inbox" + sep;

/** Block files inside the state dir (except inbox/) from being sent out. */
function assertSendable(filePath: string): void {
  let resolved: string;
  try {
    resolved = realpathSync(filePath);
  } catch {
    if (!existsSync(filePath)) return; // truly missing — let adapter handle
    throw new Error(`Blocked: cannot resolve path ${filePath}`);
  }
  if (resolved.startsWith(STATE_DIR) && !resolved.includes(INBOX_SEG)) {
    throw new Error(`Blocked: refusing to send state file ${filePath}`);
  }
}

/**
 * Route a channel tool call (reply, react, edit_message, download_attachment)
 * to the adapter. Returns true if handled, false if unknown tool.
 */
export function routeToolCall(
  adapter: ChannelAdapter,
  tool: string,
  args: Record<string, unknown>,
  threadId: string | undefined,
  respond: (result: unknown, error?: string) => void,
): boolean {
  const chatId = args.chat_id as string ?? "";

  switch (tool) {
    case "reply": {
      const files = Array.isArray(args.files) ? args.files as string[] : [];
      try {
        for (const f of files) assertSendable(f);
      } catch (e: any) {
        respond(null, e.message);
        return true;
      }
      const replyThreadId = args.thread_id as string ?? threadId;
      adapter.sendText(chatId, args.text as string ?? "", {
        threadId: replyThreadId,
        replyTo: args.reply_to as string,
      }).then(async (sent) => {
        for (const filePath of files) {
          await adapter.sendFile(chatId, filePath, { threadId: replyThreadId });
        }
        respond(sent);
      }).catch(e => respond(null, e.message));
      return true;
    }
    case "react":
      adapter.react(chatId, args.message_id as string ?? "", args.emoji as string ?? "")
        .then(() => respond("ok"))
        .catch(e => respond(null, e.message));
      return true;
    case "edit_message":
      adapter.editMessage(chatId, args.message_id as string ?? "", args.text as string ?? "")
        .then(() => respond("ok"))
        .catch(e => respond(null, e.message));
      return true;
    case "download_attachment":
      adapter.downloadAttachment(args.file_id as string ?? "")
        .then(path => respond(path))
        .catch(e => respond(null, e.message));
      return true;
    default:
      return false;
  }
}
