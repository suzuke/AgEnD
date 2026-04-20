import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { processAttachments } from "../src/channel/attachment-handler.js";
import type { ChannelAdapter, InboundMessage } from "../src/channel/types.js";

vi.mock("../src/stt.js", () => ({
  transcribe: vi.fn(),
}));
vi.mock("node:fs", async (importOriginal) => {
  const actual = await importOriginal<typeof import("node:fs")>();
  return { ...actual, unlinkSync: vi.fn() };
});

import { transcribe } from "../src/stt.js";

const silentLogger = { info() {}, warn() {} };

function makeMsg(over: Partial<InboundMessage> = {}): InboundMessage {
  return {
    chatId: "c1", messageId: "m1", userId: "u1", username: "alice",
    text: "", timestamp: new Date(), source: "telegram",
    ...over,
  } as InboundMessage;
}

const mockAdapter = {
  downloadAttachment: vi.fn().mockResolvedValue("/tmp/voice.ogg"),
} as unknown as ChannelAdapter;

describe("processAttachments — STT opt-in (P3.4)", () => {
  beforeEach(() => {
    vi.mocked(transcribe).mockReset();
    vi.mocked(mockAdapter.downloadAttachment).mockClear();
    delete process.env.GROQ_API_KEY;
    delete process.env.CUSTOM_KEY;
  });
  afterEach(() => {
    delete process.env.GROQ_API_KEY;
    delete process.env.CUSTOM_KEY;
  });

  const voiceMsg = makeMsg({
    attachments: [{ kind: "voice", fileId: "f1" } as InboundMessage["attachments"][number]],
  });

  it("does NOT transcribe when stt config is absent (privacy default)", async () => {
    process.env.GROQ_API_KEY = "gsk_should_not_matter";
    const r = await processAttachments(voiceMsg, mockAdapter, silentLogger);
    expect(transcribe).not.toHaveBeenCalled();
    expect(mockAdapter.downloadAttachment).not.toHaveBeenCalled();
    expect(r.text).toContain("STT disabled");
  });

  it("does NOT transcribe when stt.enabled is false even if env key set", async () => {
    process.env.GROQ_API_KEY = "gsk_x";
    const r = await processAttachments(voiceMsg, mockAdapter, silentLogger, undefined, { enabled: false });
    expect(transcribe).not.toHaveBeenCalled();
    expect(r.text).toContain("STT disabled");
  });

  it("transcribes when stt.enabled === true and env key present", async () => {
    process.env.GROQ_API_KEY = "gsk_real";
    vi.mocked(transcribe).mockResolvedValue({ text: "hello world" });
    const r = await processAttachments(voiceMsg, mockAdapter, silentLogger, undefined, { enabled: true });
    expect(transcribe).toHaveBeenCalledWith("/tmp/voice.ogg", "gsk_real");
    expect(r.text).toContain("hello world");
  });

  it("warns when enabled but env var missing — still does NOT call transcribe", async () => {
    const warn = vi.fn();
    const logger = { info() {}, warn };
    const r = await processAttachments(voiceMsg, mockAdapter, logger, undefined, { enabled: true });
    expect(transcribe).not.toHaveBeenCalled();
    expect(warn).toHaveBeenCalled();
    expect(r.text).toContain("GROQ_API_KEY not set");
  });

  it("respects custom api_key_env name", async () => {
    process.env.CUSTOM_KEY = "gsk_custom";
    vi.mocked(transcribe).mockResolvedValue({ text: "ok" });
    await processAttachments(voiceMsg, mockAdapter, silentLogger, undefined, {
      enabled: true, api_key_env: "CUSTOM_KEY",
    });
    expect(transcribe).toHaveBeenCalledWith("/tmp/voice.ogg", "gsk_custom");
  });

  it("falls through to attachment_file_id even when STT disabled", async () => {
    const r = await processAttachments(voiceMsg, mockAdapter, silentLogger);
    expect(r.extraMeta.attachment_file_id).toBe("f1");
  });
});
