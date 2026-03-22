import { readFileSync } from "node:fs";
import { basename } from "node:path";

export interface SttResult {
  text: string;
  language?: string;
  duration?: number;
}

/**
 * Transcribe audio using Groq Whisper API.
 * Supports OGG, MP3, WAV, FLAC, M4A, WEBM.
 */
export async function transcribe(
  filePath: string,
  apiKey: string,
  model = "whisper-large-v3",
): Promise<SttResult> {
  const fileBuffer = readFileSync(filePath);
  const fileName = basename(filePath);

  const formData = new FormData();
  formData.append("file", new Blob([fileBuffer]), fileName);
  formData.append("model", model);

  const res = await fetch("https://api.groq.com/openai/v1/audio/transcriptions", {
    method: "POST",
    headers: { Authorization: `Bearer ${apiKey}` },
    body: formData,
  });

  if (!res.ok) {
    const errText = await res.text();
    throw new Error(`Groq STT failed (${res.status}): ${errText}`);
  }

  const data = (await res.json()) as { text: string; language?: string; duration?: number };
  return {
    text: data.text,
    language: data.language,
    duration: data.duration,
  };
}
