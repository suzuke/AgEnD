import pino from "pino";
import { join } from "node:path";
import { homedir } from "node:os";
import { mkdirSync, statSync, renameSync, unlinkSync } from "node:fs";

const DATA_DIR = join(homedir(), ".agend");
const LOG_FILE = join(DATA_DIR, "daemon.log");
const MAX_LOG_SIZE = 10 * 1024 * 1024; // 10 MB
const MAX_LOG_BACKUPS = 3;

/** Rotate log file if it exceeds MAX_LOG_SIZE */
function rotateLogIfNeeded(logPath: string): void {
  try {
    const stat = statSync(logPath);
    if (stat.size < MAX_LOG_SIZE) return;

    // Rotate: .log.3 → delete, .log.2 → .log.3, .log.1 → .log.2, .log → .log.1
    for (let i = MAX_LOG_BACKUPS; i >= 1; i--) {
      const from = i === 1 ? logPath : `${logPath}.${i - 1}`;
      const to = `${logPath}.${i}`;
      try {
        if (i === MAX_LOG_BACKUPS) unlinkSync(to);
      } catch { /* file may not exist */ }
      try { renameSync(from, to); } catch { /* file may not exist */ }
    }
  } catch { /* file may not exist yet */ }
}

export function createLogger(level: string = "info") {
  mkdirSync(DATA_DIR, { recursive: true });
  rotateLogIfNeeded(LOG_FILE);
  return pino({
    level,
    transport: {
      targets: [
        {
          target: "pino-pretty",
          options: {
            destination: 1,
            colorize: true,
            translateTime: "SYS:HH:MM:ss",
            ignore: "pid,hostname",
          },
          level,
        },
        {
          target: "pino-pretty",
          options: {
            destination: LOG_FILE,
            colorize: false,
            translateTime: "SYS:HH:MM:ss",
            ignore: "pid,hostname",
          },
          level,
        },
      ],
    },
  });
}

export type Logger = ReturnType<typeof createLogger>;
