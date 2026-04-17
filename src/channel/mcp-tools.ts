/**
 * MCP tool definitions. Schemas come from src/outbound-schemas.ts so runtime
 * validation and the JSON Schema surfaced to agents share one source.
 *
 * Kept free of runtime side effects — safe to import from tests and tools
 * without pulling in channel infrastructure.
 */
import { z, type ZodType } from "zod";
import * as schemas from "../outbound-schemas.js";

/** Narrow JSON-schema fragment shape that MCP tool listings need. */
type JsonSchemaObject = {
  type: "object";
  properties?: Record<string, unknown>;
  required?: string[];
  additionalProperties?: unknown;
  [k: string]: unknown;
};

/** Strip `$schema`/`$defs` that some MCP clients reject. */
function toToolInputSchema(schema: ZodType): JsonSchemaObject {
  const raw = z.toJSONSchema(schema) as Record<string, unknown>;
  const { $schema: _s, $defs: _d, ...rest } = raw;
  if (rest.type !== "object") {
    throw new Error("Expected object-typed zod schema for MCP tool input");
  }
  return rest as JsonSchemaObject;
}

type ToolDef = {
  name: string;
  description: string;
  inputSchema: JsonSchemaObject;
};

/**
 * Build the tool list. The `[schema, description]` pairs keep the declaration
 * terse and guarantee every tool has both a description and a schema.
 */
const DEFS: Array<[string, ZodType, string]> = [
  ["reply", schemas.ReplyArgs,
    "Reply on the channel. Routing is handled automatically — do not pass chat_id or thread_id."],
  ["react", schemas.ReactArgs,
    "Add an emoji reaction to a channel message."],
  ["edit_message", schemas.EditMessageArgs,
    "Edit a message the bot previously sent. Useful for interim progress updates."],
  ["download_attachment", schemas.DownloadAttachmentArgs,
    "Download a file attachment from a channel message. Returns the local file path ready to Read."],
  ["create_schedule", schemas.CreateScheduleArgs,
    "Create a cron-based schedule. When triggered, sends a message to the target instance."],
  ["list_schedules", schemas.ListSchedulesArgs,
    "List all schedules. Optionally filter by target instance."],
  ["update_schedule", schemas.UpdateScheduleArgs,
    "Update an existing schedule. Only include fields you want to change."],
  ["delete_schedule", schemas.DeleteScheduleArgs,
    "Delete a schedule by ID."],
  ["task", schemas.TaskBoardArgs,
    "Manage fleet task board. Actions: create (new task), list (show tasks), claim (assign to self), done (mark complete), update (change status/priority/assignee)."],
  ["post_decision", schemas.PostDecisionArgs,
    "Record a decision. scope='project' (default) is visible to instances sharing this working directory. scope='fleet' is visible to ALL instances regardless of directory — use for workflow rules, review policies, and team conventions."],
  ["list_decisions", schemas.ListDecisionsArgs,
    "List active decisions for this project. Returns decisions that were recorded by any instance sharing this working directory."],
  ["update_decision", schemas.UpdateDecisionArgs,
    "Update or archive an existing decision."],
  ["broadcast", schemas.BroadcastArgs,
    "Send a message to multiple instances at once. Priority: team > targets > tags > all running."],
  ["send_to_instance", schemas.SendToInstanceArgs,
    "Send a message to another instance. Use for cross-instance communication."],
  ["request_information", schemas.RequestInformationArgs,
    "Ask another instance a question and expect a reply. Wrapper around send_to_instance with request_kind=query and requires_reply=true."],
  ["delegate_task", schemas.DelegateTaskArgs,
    "Delegate a task to another instance and expect a result report back. Wrapper around send_to_instance with request_kind=task and requires_reply=true."],
  ["report_result", schemas.ReportResultArgs,
    "Report results back to an instance that delegated a task or asked a question. Wrapper around send_to_instance with request_kind=report."],
  ["create_team", schemas.CreateTeamArgs,
    "Create a named group of instances for targeted broadcasting. Teams persist across restarts."],
  ["delete_team", schemas.DeleteTeamArgs,
    "Dissolve a team. Does not affect the member instances."],
  ["list_teams", schemas.ListTeamsArgs,
    "List all teams with their members and running status."],
  ["update_team", schemas.UpdateTeamArgs,
    "Add or remove members from an existing team. Duplicates are ignored."],
  ["describe_instance", schemas.DescribeInstanceArgs,
    "Get detailed information about a specific instance: description, working directory, status, tags, and recent activity."],
  ["list_instances", schemas.ListInstancesArgs,
    "List all currently running instances that you can send messages to."],
  ["start_instance", schemas.StartInstanceArgs,
    "Start a stopped instance by name."],
  ["create_instance", schemas.CreateInstanceArgs,
    "Create a new instance bound to a project directory with a channel topic. If directory is omitted, a workspace is auto-created at ~/.agend/workspaces/<instance-name>."],
  ["delete_instance", schemas.DeleteInstanceArgs,
    "Delete an instance: stop daemon, remove config, optionally delete topic."],
  ["replace_instance", schemas.ReplaceInstanceArgs,
    "Replace an instance with a fresh one. Collects handover context from the old instance (or falls back to daemon ring buffer), deletes it, creates a new instance with the same config, and sends the handover context to the new instance."],
  ["set_display_name", schemas.SetDisplayNameArgs,
    "Set your display name. This name will be shown in Telegram messages, activity logs, and when other agents refer to you."],
  ["set_description", schemas.SetDescriptionArgs,
    "Set your role description. This is injected into your system prompt as your role definition. Takes effect on next context rotation."],
  ["checkout_repo", schemas.CheckoutRepoArgs,
    "Mount another repo as a read-only worktree. Returns a local path you can Read files from. Use instance name or absolute path as source."],
  ["release_repo", schemas.ReleaseRepoArgs,
    "Remove a previously checked-out repo worktree."],
  ["deploy_template", schemas.DeployTemplateArgs,
    "Deploy a fleet template — creates instances and optionally a team in one operation."],
  ["teardown_deployment", schemas.TeardownDeploymentArgs,
    "Tear down a template deployment — stops and deletes all instances and team."],
  ["list_deployments", schemas.ListDeploymentsArgs,
    "List active template deployments with their instances and status."],
];

export const TOOLS: ToolDef[] = DEFS.map(([name, schema, description]) => ({
  name,
  description,
  inputSchema: toToolInputSchema(schema),
}));

/** Predefined tool profiles to reduce token overhead per instance. */
export const TOOL_SETS: Record<string, string[]> = {
  full: TOOLS.map(t => t.name),
  standard: [
    "reply", "react", "edit_message",
    "send_to_instance", "broadcast", "list_instances", "describe_instance",
    "list_decisions", "post_decision", "task", "set_display_name", "set_description",
  ],
  minimal: ["reply", "send_to_instance", "list_decisions", "download_attachment"],
};
