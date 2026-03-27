import type { ChannelAdapter } from "./types.js";
import type { ChannelConfig } from "../types.js";
import type { AccessManager } from "./access-manager.js";

export interface AdapterOpts {
  id: string;
  botToken: string;
  accessManager: AccessManager;
  inboxDir: string;
}

export async function createAdapter(config: ChannelConfig, opts: AdapterOpts): Promise<ChannelAdapter> {
  switch (config.type) {
    case "telegram": {
      const { TelegramAdapter } = await import("./adapters/telegram.js");
      return new TelegramAdapter(opts);
    }
    default:
      throw new Error(`Unknown channel type: ${config.type}`);
  }
}
