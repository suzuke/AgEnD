import type { ChannelAdapter } from "./types.js";

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
