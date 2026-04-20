import { unlinkSync } from "node:fs";
import { transcribe } from "../stt.js";
import type { InboundMessage, ChannelAdapter } from "./types.js";
import type { STTConfig } from "../types.js";

export interface AttachmentResult {
  text: string;
  extraMeta: Record<string, string>;
}

/**
 * Process attachments on an inbound message:
 * - Auto-download photos → extraMeta.image_path
 * - Transcribe voice/audio via Groq Whisper → prepend to text (only if
 *   `stt.enabled === true` is explicitly set in fleet.yaml — privacy default
 *   is off so audio never silently leaves the system based on env var alone)
 * - Pass other attachment types as file_id for manual download
 */
export async function processAttachments(
  msg: InboundMessage,
  adapter: ChannelAdapter,
  logger: { info(obj: unknown, msg?: string): void; warn(obj: unknown, msg?: string): void },
  logPrefix?: string,
  stt?: STTConfig,
): Promise<AttachmentResult> {
  let text = msg.text;
  const extraMeta: Record<string, string> = {};

  // Auto-download photos so Claude can Read them directly
  const photoAttachment = msg.attachments?.find(a => a.kind === "photo");
  if (photoAttachment) {
    try {
      const localPath = await adapter.downloadAttachment(photoAttachment.fileId);
      extraMeta.image_path = localPath;
      text = `[📷 Image: ${localPath}]\n${text}`;
    } catch (err) {
      logger.warn({ err: (err as Error).message }, "Photo download failed");
    }
  }

  // Transcribe voice/audio — opt-in only.
  const voiceAttachment = msg.attachments?.find(a => a.kind === "voice" || a.kind === "audio");
  if (voiceAttachment) {
    const sttEnabled = stt?.enabled === true;
    const apiKeyEnv = stt?.api_key_env ?? "GROQ_API_KEY";
    const groqKey = sttEnabled ? process.env[apiKeyEnv] : undefined;

    if (!sttEnabled) {
      // Privacy default. Audio never uploaded.
      text = text || "[Voice message — STT disabled in fleet.yaml]";
    } else if (!groqKey) {
      logger.warn({ apiKeyEnv }, "STT enabled but API key env var is not set");
      text = text || `[Voice message — STT enabled but ${apiKeyEnv} not set]`;
    } else {
      try {
        const localPath = await adapter.downloadAttachment(voiceAttachment.fileId);
        const result = await transcribe(localPath, groqKey);
        try { unlinkSync(localPath); } catch { /* ignore */ }
        text = text ? `${text}\n\n[Voice message] ${result.text}` : `[Voice message] ${result.text}`;
        logger.info({ ...(logPrefix ? { context: logPrefix } : {}), transcription: result.text.slice(0, 80) }, "Voice transcribed");
      } catch (err) {
        logger.warn({ err: (err as Error).message }, "Voice transcription failed");
        text = text || "[Voice message — transcription failed]";
      }
    }
    extraMeta.attachment_file_id = voiceAttachment.fileId;
  }

  // Pass other attachment types as file_id for manual download
  const otherAttachment = msg.attachments?.find(a =>
    a.kind !== "photo" && a.kind !== "voice" && a.kind !== "audio",
  );
  if (otherAttachment) {
    extraMeta.attachment_file_id = otherAttachment.fileId;
  }

  return { text, extraMeta };
}
