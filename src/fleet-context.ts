import type { FleetConfig, InstanceConfig } from "./types.js";
import type { ChannelAdapter, InboundMessage } from "./channel/types.js";
import type { RouteTarget } from "./meeting/types.js";
import type { IpcClient } from "./channel/ipc-bridge.js";
import type { Scheduler } from "./scheduler/index.js";
import type { Logger } from "./logger.js";
import type { CostGuard } from "./cost-guard.js";

/**
 * Shared context interface for fleet sub-modules (topic commands, meeting manager).
 * FleetManager implements this and passes `this` to extracted handlers.
 */
export interface FleetContext {
  readonly adapter: ChannelAdapter | null;
  readonly fleetConfig: FleetConfig | null;
  readonly routingTable: Map<number, RouteTarget>;
  readonly instanceIpcClients: Map<string, IpcClient>;
  readonly scheduler: Scheduler | null;
  readonly logger: Logger;
  readonly dataDir: string;
  readonly costGuard: CostGuard | null;

  getInstanceStatus(name: string): "running" | "stopped" | "crashed";
  startInstance(name: string, config: InstanceConfig, topicMode: boolean): Promise<void>;
  stopInstance(name: string): Promise<void>;
  connectIpcToInstance(name: string): Promise<void>;
  saveFleetConfig(): void;
  getInstanceDir(name: string): string;
  createForumTopic(topicName: string): Promise<number>;
}
