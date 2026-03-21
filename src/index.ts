// TODO(Task 15): ProcessManager removed — will be replaced by TmuxManager-based orchestrator
// export { ProcessManager, STATUSLINE_FILE } from "./process-manager.js";
export { TmuxManager } from "./tmux-manager.js";
export { ContextGuardian } from "./context-guardian.js";
export { MemoryLayer } from "./memory-layer.js";
export { MemoryDb } from "./db.js";
export { loadConfig, DEFAULT_CONFIG } from "./config.js";
export { createLogger } from "./logger.js";
export { installService, detectPlatform } from "./service-installer.js";
export type { DaemonConfig, ContextStatus, StatusLineData, MemoryBackupRow } from "./types.js";
