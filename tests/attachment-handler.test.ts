import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { processAttachments } from "../src/channel/attachment-handler.js";
import type { InboundMessage } from "../src/channel/types.js";

const noopLogger = { info: vi.fn(), warn: vi.fn() };

const makeAdapter = () => ({
  downloadAttachment: vi.fn(async () => "/tmp/unused"),
}) as never;

const voiceMsg = (): InboundMessage => ({
  topicId: "1",
  text: "",
  userId: "u1",
  messageId: "m1",
  timestamp: Date.now(),
  attachments: [{ kind: "voice", fileId: "f1" }],
} as never);

describe("processAttachments STT opt-in (P3.4)", () => {
  let originalEnabled: string | undefined;
  let originalKey: string | undefined;

  beforeEach(() => {
    originalEnabled = process.env.AGEND_STT_ENABLED;
    originalKey = process.env.GROQ_API_KEY;
  });
  afterEach(() => {
    if (originalEnabled === undefined) delete process.env.AGEND_STT_ENABLED;
    else process.env.AGEND_STT_ENABLED = originalEnabled;
    if (originalKey === undefined) delete process.env.GROQ_API_KEY;
    else process.env.GROQ_API_KEY = originalKey;
  });

  it("does NOT transcribe when AGEND_STT_ENABLED is unset, even with GROQ_API_KEY", async () => {
    delete process.env.AGEND_STT_ENABLED;
    process.env.GROQ_API_KEY = "fake-key";
    const adapter = makeAdapter();
    const { text } = await processAttachments(voiceMsg(), adapter, noopLogger);
    expect(adapter.downloadAttachment).not.toHaveBeenCalled();
    expect(text).toMatch(/STT disabled/);
  });

  it("does NOT transcribe when AGEND_STT_ENABLED=1 but no GROQ_API_KEY", async () => {
    process.env.AGEND_STT_ENABLED = "1";
    delete process.env.GROQ_API_KEY;
    const adapter = makeAdapter();
    const { text } = await processAttachments(voiceMsg(), adapter, noopLogger);
    expect(adapter.downloadAttachment).not.toHaveBeenCalled();
    expect(text).toMatch(/API key not set/);
  });

  it("attaches file_id on voice attachment regardless of opt-in state", async () => {
    delete process.env.AGEND_STT_ENABLED;
    const adapter = makeAdapter();
    const { extraMeta } = await processAttachments(voiceMsg(), adapter, noopLogger);
    expect(extraMeta.attachment_file_id).toBe("f1");
  });
});
