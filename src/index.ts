export { TmuxManager } from "./tmux-manager.js";
export { ContextGuardian } from "./context-guardian.js";
export { loadFleetConfig, DEFAULT_INSTANCE_CONFIG } from "./config.js";
export { createLogger } from "./logger.js";
export { installService, detectPlatform } from "./service-installer.js";
export type { ContextStatus, StatusLineData, InstanceConfig, ChannelConfig, AccessConfig } from "./types.js";

// Channel adapter types — for external adapter authors (agend-adapter-*)
export type {
  ChannelAdapter,
  SendOpts,
  SentMessage,
  InboundMessage,
  Attachment,
  PermissionPrompt,
  ApprovalHandle,
  ApprovalResponse,
  Choice,
  AlertData,
  InstanceStatusData,
  QueuedMessage,
} from "./channel/types.js";
export type { AdapterOpts, AdapterFactory } from "./channel/factory.js";
