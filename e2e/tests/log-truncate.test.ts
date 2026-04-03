/**
 * E2E Test: Log Truncation — daemon.log size management
 *
 * T9: When daemon.log exceeds 10MB, truncateLogIfNeeded() keeps the last 5MB
 *     (tail-preserving), produces no backup/rotation files.
 */
import { describe, it, expect, afterAll } from "vitest";
import { join } from "node:path";
import {
  mkdirSync,
  writeFileSync,
  statSync,
  readFileSync,
  rmSync,
  readdirSync,
} from "node:fs";
import { truncateLogIfNeeded } from "../../src/logger.js";

const testDir = `/tmp/ae2e-logtrunc-${Date.now().toString(36)}`;

describe("Log Truncation E2E", () => {
  afterAll(() => {
    rmSync(testDir, { recursive: true, force: true });
  });

  it("T9: file under 10MB is not truncated", () => {
    mkdirSync(testDir, { recursive: true });
    const logPath = join(testDir, "small.log");

    // Create a 1MB file
    const oneMB = Buffer.alloc(1024 * 1024, "A");
    writeFileSync(logPath, oneMB);

    truncateLogIfNeeded(logPath);

    const stat = statSync(logPath);
    expect(stat.size).toBe(1024 * 1024); // unchanged
  });

  it("T9: file over 10MB is truncated to ~5MB tail", () => {
    const logPath = join(testDir, "large.log");

    // Create a 12MB file with identifiable content
    // Fill with numbered lines so we can verify tail is preserved
    const lines: string[] = [];
    let totalSize = 0;
    let lineNum = 0;
    while (totalSize < 12 * 1024 * 1024) {
      const line = `[${String(lineNum).padStart(8, "0")}] Log entry for testing truncation behavior\n`;
      lines.push(line);
      totalSize += line.length;
      lineNum++;
    }
    writeFileSync(logPath, lines.join(""));

    const sizeBefore = statSync(logPath).size;
    expect(sizeBefore).toBeGreaterThan(10 * 1024 * 1024);

    truncateLogIfNeeded(logPath);

    const sizeAfter = statSync(logPath).size;

    // Should be approximately 5MB (may be slightly less due to newline alignment)
    expect(sizeAfter).toBeLessThanOrEqual(5 * 1024 * 1024);
    expect(sizeAfter).toBeGreaterThan(4 * 1024 * 1024);

    // Verify content is from the TAIL (last lines should have high line numbers)
    const content = readFileSync(logPath, "utf-8");
    const resultLines = content.trim().split("\n");
    const lastLine = resultLines[resultLines.length - 1];
    const lastLineNum = parseInt(lastLine.match(/\[(\d+)\]/)?.[1] ?? "0", 10);
    expect(lastLineNum).toBe(lineNum - 1); // last line of original file

    // First line of truncated file should be from well into the original
    const firstLine = resultLines[0];
    const firstLineNum = parseInt(firstLine.match(/\[(\d+)\]/)?.[1] ?? "0", 10);
    expect(firstLineNum).toBeGreaterThan(lineNum / 2); // from the latter half
  });

  it("T9: no backup or rotation files created", () => {
    const files = readdirSync(testDir);
    const backupFiles = files.filter(
      (f) =>
        f.includes(".bak") ||
        f.includes(".old") ||
        f.includes(".1") ||
        f.includes(".gz") ||
        f.match(/\.log\.\d+$/),
    );
    expect(backupFiles).toHaveLength(0);
  });

  it("T9: truncated file starts at a complete log line", () => {
    const logPath = join(testDir, "large.log");
    const content = readFileSync(logPath, "utf-8");

    // First character should be '[' (start of a log line), not a partial line
    expect(content[0]).toBe("[");
  });
});
