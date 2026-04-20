/**
 * RPC / CRUD handlers for fleet-side actions invoked by instances over IPC
 * or by the agent CLI over HTTP.
 *
 * Pattern follows outbound-handlers.ts: a `RpcHandlersContext` interface +
 * a `rpcHandlers` dispatch dictionary. FleetManager implements the context
 * and forwards each call so external surfaces (AgentEndpointContext, IPC
 * dispatch) keep their existing shape.
 *
 * Extracted from fleet-manager.ts (P4.1 step 3 of 4).
 */
import type { Logger } from "./logger.js";
import type { Scheduler } from "./scheduler/index.js";
import type { IpcClient } from "./channel/ipc-bridge.js";
import type { EventLog } from "./event-log.js";
import type { FleetConfig } from "./types.js";

export interface RpcHandlersContext {
  readonly scheduler: Scheduler | null;
  readonly fleetConfig: FleetConfig | null;
  readonly logger: Logger;
  readonly eventLog: EventLog | null;
  readonly instanceIpcClients: Map<string, IpcClient>;
  saveFleetConfig(): void;
}

/** Resolve display name for an instance, fallback to instance name. */
export function resolveDisplayName(
  ctx: { fleetConfig: FleetConfig | null },
  instanceName: string,
): string {
  return ctx.fleetConfig?.instances[instanceName]?.display_name ?? instanceName;
}

/** One-line description of a fleet tool call for activity logs. */
export function summarizeToolCall(tool: string, args: Record<string, unknown>): string {
  switch (tool) {
    case "send_to_instance": return `send_to_instance(${args.instance_name})`;
    case "broadcast": return `broadcast(${(args.targets as string[])?.join(", ") ?? "all"})`;

    case "request_information": return `request_information(${args.target_instance}, "${(args.question as string ?? "").slice(0, 60)}")`;
    case "delegate_task": return `delegate_task(${args.target_instance}, "${(args.task as string ?? "").slice(0, 60)}")`;
    case "report_result": return `report_result(${args.target_instance})`;
    case "task": return `task(${args.action}${args.title ? `, "${(args.title as string).slice(0, 40)}"` : args.id ? `, ${(args.id as string).slice(0, 8)}` : ""})`;
    case "post_decision": return `post_decision("${(args.title as string ?? "").slice(0, 40)}")`;
    case "list_decisions": return "list_decisions()";
    case "list_instances": return "list_instances()";
    case "describe_instance": return `describe_instance(${args.name})`;
    case "start_instance": return `start_instance(${args.name})`;
    case "create_instance": return `create_instance(${args.directory})`;
    case "delete_instance": return `delete_instance(${args.name})`;
    case "replace_instance": return `replace_instance(${args.name})`;
    default: return `${tool}()`;
  }
}

export const rpcHandlers = {
  handleScheduleCrud(ctx: RpcHandlersContext, instanceName: string, msg: Record<string, unknown>): void {
    const fleetRequestId = msg.fleetRequestId as string;
    const payload = (msg.payload ?? {}) as Record<string, unknown>;
    const meta = (msg.meta ?? {}) as Record<string, string>;
    const ipc = ctx.instanceIpcClients.get(instanceName);
    if (!ipc) return;

    try {
      let result: unknown;

      switch (msg.type) {
        case "fleet_schedule_create": {
          const params = {
            cron: payload.cron as string,
            message: payload.message as string,
            source: instanceName,
            target: (payload.target as string) || instanceName,
            reply_chat_id: meta.chat_id,
            reply_thread_id: meta.thread_id || null,
            label: payload.label as string | undefined,
            timezone: payload.timezone as string | undefined,
          };
          result = ctx.scheduler!.create(params);
          break;
        }
        case "fleet_schedule_list":
          result = ctx.scheduler!.list(payload.target as string | undefined);
          break;
        case "fleet_schedule_update":
          result = ctx.scheduler!.update(payload.id as string, payload as Record<string, unknown>);
          break;
        case "fleet_schedule_delete":
          ctx.scheduler!.delete(payload.id as string);
          result = "ok";
          break;
      }

      ipc.send({ type: "fleet_schedule_response", fleetRequestId, result });
    } catch (err) {
      ipc.send({ type: "fleet_schedule_response", fleetRequestId, error: (err as Error).message });
    }
  },

  handleDecisionCrud(ctx: RpcHandlersContext, instanceName: string, msg: Record<string, unknown>): void {
    const fleetRequestId = msg.fleetRequestId as string;
    const payload = (msg.payload ?? {}) as Record<string, unknown>;
    const meta = (msg.meta ?? {}) as Record<string, string>;
    const ipc = ctx.instanceIpcClients.get(instanceName);
    if (!ipc || !ctx.scheduler) return;

    const db = ctx.scheduler.db;
    const projectRoot = meta.working_directory || ctx.fleetConfig?.instances[instanceName]?.working_directory || "";

    try {
      let result: unknown;

      switch (msg.type) {
        case "fleet_decision_create": {
          // Prune expired decisions on create
          db.pruneExpiredDecisions();
          result = db.createDecision({
            project_root: projectRoot,
            scope: (payload.scope as "project" | "fleet" | undefined),
            title: payload.title as string,
            content: payload.content as string,
            tags: payload.tags as string[] | undefined,
            ttl_days: payload.ttl_days as number | undefined,
            created_by: instanceName,
            supersedes: payload.supersedes as string | undefined,
          });
          break;
        }
        case "fleet_decision_list":
          db.pruneExpiredDecisions();
          result = db.listDecisions(projectRoot, {
            includeArchived: payload.include_archived as boolean | undefined,
            tags: payload.tags as string[] | undefined,
          });
          break;
        case "fleet_decision_update": {
          const id = payload.id as string;
          if (payload.archive) {
            db.archiveDecision(id);
            result = { archived: true, id };
          } else {
            result = db.updateDecision(id, {
              content: payload.content as string | undefined,
              tags: payload.tags as string[] | undefined,
              ttl_days: payload.ttl_days as number | undefined,
            });
          }
          break;
        }
      }

      ipc.send({ type: "fleet_decision_response", fleetRequestId, result });
    } catch (err) {
      ipc.send({ type: "fleet_decision_response", fleetRequestId, error: (err as Error).message });
    }
  },

  handleSetDisplayName(ctx: RpcHandlersContext, instanceName: string, msg: Record<string, unknown>): void {
    const fleetRequestId = msg.fleetRequestId as string;
    const payload = (msg.payload ?? {}) as Record<string, unknown>;
    const ipc = ctx.instanceIpcClients.get(instanceName);
    if (!ipc || !ctx.fleetConfig) return;

    const displayName = payload.name as string;
    if (!displayName || displayName.length > 30) {
      ipc.send({ type: "fleet_display_name_response", fleetRequestId, error: "Name must be 1-30 characters" });
      return;
    }

    ctx.fleetConfig.instances[instanceName].display_name = displayName;
    ctx.saveFleetConfig();
    ctx.logger.info({ instanceName, displayName }, "Display name set");
    ipc.send({ type: "fleet_display_name_response", fleetRequestId, result: { display_name: displayName } });
  },

  handleSetDescription(ctx: RpcHandlersContext, instanceName: string, msg: Record<string, unknown>): void {
    const fleetRequestId = msg.fleetRequestId as string;
    const payload = (msg.payload ?? {}) as Record<string, unknown>;
    const ipc = ctx.instanceIpcClients.get(instanceName);
    if (!ipc || !ctx.fleetConfig) return;

    const description = payload.description as string;
    if (!description) {
      ipc.send({ type: "fleet_description_response", fleetRequestId, error: "Description cannot be empty" });
      return;
    }

    ctx.fleetConfig.instances[instanceName].description = description;
    ctx.saveFleetConfig();
    ctx.logger.info({ instanceName, description: description.slice(0, 80) }, "Description set");
    ipc.send({ type: "fleet_description_response", fleetRequestId, result: { description } });
  },

  // ── Agent CLI HTTP handlers ─────────────────────────────────────────

  async handleScheduleCrudHttp(ctx: RpcHandlersContext, instance: string, op: string, args: Record<string, unknown>): Promise<unknown> {
    if (!ctx.scheduler) return { error: "Scheduler not available" };
    switch (op) {
      case "create":
        return ctx.scheduler.create({
          cron: args.cron as string, message: args.message as string,
          source: instance, target: (args.target as string) || instance,
          reply_chat_id: "", reply_thread_id: null,
          label: args.label as string | undefined,
          timezone: args.timezone as string | undefined,
        });
      case "list": return ctx.scheduler.list(args.target as string | undefined);
      case "update": return ctx.scheduler.update(args.id as string, args);
      case "delete": ctx.scheduler.delete(args.id as string); return "ok";
      default: return { error: `Unknown schedule op: ${op}` };
    }
  },

  async handleDecisionCrudHttp(ctx: RpcHandlersContext, instance: string, op: string, args: Record<string, unknown>): Promise<unknown> {
    if (!ctx.scheduler) return { error: "Scheduler not available" };
    const db = ctx.scheduler.db;
    const projectRoot = ctx.fleetConfig?.instances[instance]?.working_directory ?? "";
    const asStr = (v: unknown): string | undefined => typeof v === "string" ? v : undefined;
    const asNum = (v: unknown): number | undefined => typeof v === "number" ? v : undefined;
    const asStrArr = (v: unknown): string[] | undefined =>
      Array.isArray(v) && v.every(x => typeof x === "string") ? v as string[] : undefined;
    switch (op) {
      case "post": {
        const title = asStr(args.title);
        const content = asStr(args.content);
        if (!title || !content) return { error: "title and content are required" };
        const scope = args.scope === "fleet" ? "fleet" : "project";
        return db.createDecision({
          project_root: projectRoot,
          scope,
          title,
          content,
          tags: asStrArr(args.tags),
          ttl_days: asNum(args.ttl_days),
          supersedes: asStr(args.supersedes),
          created_by: instance,
        });
      }
      case "list": return db.listDecisions(projectRoot, {
        includeArchived: args.includeArchived === true,
        tags: asStrArr(args.tags),
      });
      case "update": {
        const id = asStr(args.id);
        if (!id) return { error: "id is required" };
        return db.updateDecision(id, {
          content: asStr(args.content),
          tags: asStrArr(args.tags),
          ttl_days: asNum(args.ttl_days),
        });
      }
      default: return { error: `Unknown decision op: ${op}` };
    }
  },

  async handleTaskCrudHttp(ctx: RpcHandlersContext, instance: string, args: Record<string, unknown>): Promise<unknown> {
    if (!ctx.scheduler) return { error: "Scheduler not available" };
    const db = ctx.scheduler.db;
    const action = args.action as string;
    const asStr = (v: unknown): string | undefined => typeof v === "string" ? v : undefined;
    const asStrArr = (v: unknown): string[] | undefined =>
      Array.isArray(v) && v.every(x => typeof x === "string") ? v as string[] : undefined;
    const asPriority = (v: unknown): "low" | "normal" | "high" | "urgent" | undefined => {
      return (v === "low" || v === "normal" || v === "high" || v === "urgent") ? v : undefined;
    };
    const asStatus = (v: unknown): "open" | "claimed" | "done" | "blocked" | "cancelled" | undefined => {
      return (v === "open" || v === "claimed" || v === "done" || v === "blocked" || v === "cancelled") ? v : undefined;
    };
    switch (action) {
      case "create": {
        const title = asStr(args.title);
        if (!title) return { error: "title is required" };
        return db.createTask({
          title,
          description: asStr(args.description),
          priority: asPriority(args.priority),
          assignee: asStr(args.assignee),
          depends_on: asStrArr(args.depends_on),
          created_by: instance,
        });
      }
      case "list": return db.listTasks({ assignee: asStr(args.filter_assignee), status: asStr(args.filter_status) });
      case "claim": {
        const id = asStr(args.id);
        if (!id) return { error: "id is required" };
        return db.claimTask(id, instance);
      }
      case "done": {
        const id = asStr(args.id);
        if (!id) return { error: "id is required" };
        return db.completeTask(id, asStr(args.result));
      }
      case "update": {
        const id = asStr(args.id);
        if (!id) return { error: "id is required" };
        return db.updateTask(id, {
          status: asStatus(args.status),
          assignee: asStr(args.assignee),
          result: asStr(args.result),
          priority: asPriority(args.priority),
        });
      }
      default: return { error: `Unknown task action: ${action}` };
    }
  },

  async handleSetDisplayNameHttp(ctx: RpcHandlersContext, instance: string, name: string): Promise<unknown> {
    if (!ctx.fleetConfig) return { error: "Fleet config not available" };
    if (!name || name.length > 30) return { error: "Name must be 1-30 characters" };
    ctx.fleetConfig.instances[instance].display_name = name;
    ctx.saveFleetConfig();
    return { display_name: name };
  },

  async handleSetDescriptionHttp(ctx: RpcHandlersContext, instance: string, description: string): Promise<unknown> {
    if (!ctx.fleetConfig) return { error: "Fleet config not available" };
    if (!description) return { error: "Description cannot be empty" };
    ctx.fleetConfig.instances[instance].description = description;
    ctx.saveFleetConfig();
    return { description };
  },

  handleTaskCrud(ctx: RpcHandlersContext, instanceName: string, msg: Record<string, unknown>): void {
    const fleetRequestId = msg.fleetRequestId as string;
    const payload = (msg.payload ?? {}) as Record<string, unknown>;
    const meta = (msg.meta ?? {}) as Record<string, string>;
    const ipc = ctx.instanceIpcClients.get(instanceName);
    if (!ipc || !ctx.scheduler) return;

    const db = ctx.scheduler.db;
    const action = payload.action as string;

    try {
      let result: unknown;
      switch (action) {
        case "create":
          result = db.createTask({
            title: payload.title as string,
            description: payload.description as string | undefined,
            priority: payload.priority as "low" | "normal" | "high" | "urgent" | undefined,
            assignee: payload.assignee as string | undefined,
            depends_on: payload.depends_on as string[] | undefined,
            created_by: meta.instance_name || instanceName,
          });
          break;
        case "list":
          result = db.listTasks({
            assignee: payload.filter_assignee as string | undefined,
            status: payload.filter_status as string | undefined,
          });
          break;
        case "claim":
          result = db.claimTask(payload.id as string, meta.instance_name || instanceName);
          break;
        case "done":
          result = db.completeTask(payload.id as string, payload.result as string | undefined);
          break;
        case "update":
          result = db.updateTask(payload.id as string, {
            status: payload.status as string | undefined,
            assignee: payload.assignee as string | undefined,
            result: payload.result as string | undefined,
            priority: payload.priority as string | undefined,
          } as Record<string, unknown>);
          break;
        default:
          throw new Error(`Unknown task action: ${action}`);
      }
      ipc.send({ type: "fleet_task_response", fleetRequestId, result });

      // Activity log for task lifecycle events
      if (action === "create") {
        const t = result as { title: string; assignee?: string };
        ctx.eventLog?.logActivity("task_update", instanceName, `created task: ${t.title}`, t.assignee ?? undefined);
      } else if (action === "claim") {
        const t = result as { title: string };
        ctx.eventLog?.logActivity("task_update", instanceName, `claimed: ${t.title}`);
      } else if (action === "done") {
        const t = result as { title: string; result?: string };
        ctx.eventLog?.logActivity("task_update", instanceName, `completed: ${t.title}`, undefined, t.result ?? undefined);
      }
    } catch (err) {
      ipc.send({ type: "fleet_task_response", fleetRequestId, error: (err as Error).message });
    }
  },
};
