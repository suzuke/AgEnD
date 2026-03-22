import { EventEmitter } from "node:events";
import { open, stat } from "node:fs/promises";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { Logger } from "./logger.js";

export class TranscriptMonitor extends EventEmitter {
  private fd: number | null = null;          // file handle for JSONL
  private byteOffset: number = 0;
  private transcriptPath: string | null = null;
  private pollTimer: ReturnType<typeof setInterval> | null = null;

  constructor(private instanceDir: string, private logger: Logger) { super(); }

  // Find transcript JSONL path from statusline.json or project dir scan
  async resolveTranscriptPath(): Promise<string | null> {
    // Try statusline.json first
    const statusFile = join(this.instanceDir, "statusline.json");
    if (existsSync(statusFile)) {
      try {
        const data = JSON.parse(readFileSync(statusFile, "utf-8"));
        if (data.transcript_path) return data.transcript_path;
      } catch {}
    }
    return null;
  }

  // Read new bytes from byteOffset, parse JSONL lines, emit events
  async pollIncrement(): Promise<void> {
    if (!this.transcriptPath) {
      this.transcriptPath = await this.resolveTranscriptPath();
      if (!this.transcriptPath) return;
      // Skip existing content on first discovery — only process new entries
      try {
        const initial = await stat(this.transcriptPath);
        this.byteOffset = initial.size;
        return;
      } catch { return; }
    }
    if (!existsSync(this.transcriptPath)) return;

    try {
      const stats = await stat(this.transcriptPath);
      if (stats.size <= this.byteOffset) return;

      const fh = await open(this.transcriptPath, "r");
      try {
        const length = stats.size - this.byteOffset;
        const buffer = Buffer.alloc(length);
        await fh.read(buffer, 0, length, this.byteOffset);
        this.byteOffset = stats.size;

        const text = buffer.toString("utf-8");
        for (const line of text.split("\n")) {
          if (!line.trim()) continue;
          try {
            const entry = JSON.parse(line);
            this.processEntry(entry);
          } catch {}
        }
      } finally {
        await fh.close();
      }
    } catch (err) {
      this.logger.debug({ err }, "TranscriptMonitor poll error");
    }
  }

  private processEntry(entry: any): void {
    const msg = entry.message;
    if (!msg?.role || !msg?.content) return;

    const contents = Array.isArray(msg.content) ? msg.content : [{ type: "text", text: msg.content }];

    for (const block of contents) {
      if (block.type === "tool_use") {
        this.emit("tool_use", block.name ?? "unknown", block.input ?? {});
      } else if (block.type === "tool_result") {
        this.emit("tool_result", block.tool_use_id ?? "unknown", block.content);
      } else if (block.type === "text" && msg.role === "assistant" && block.text?.trim()) {
        // Check if it's a channel message (user input via channel)
        const channelMatch = block.text.match(/<channel[^>]*user="([^"]*)"[^>]*>([\s\S]*?)<\/channel>/);
        if (channelMatch) {
          this.emit("channel_message", channelMatch[1], channelMatch[2]);
        } else {
          this.emit("assistant_text", block.text);
        }
      }
    }
  }

  startPolling(intervalMs = 2000): void {
    this.pollTimer = setInterval(() => this.pollIncrement(), intervalMs);
  }

  stop(): void {
    if (this.pollTimer) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
  }

  // For testing: set transcript path directly
  setTranscriptPath(path: string): void {
    this.transcriptPath = path;
  }

  // Reset offset (for session rotation)
  resetOffset(): void {
    this.byteOffset = 0;
    this.transcriptPath = null;
  }
}
