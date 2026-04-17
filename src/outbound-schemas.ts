/**
 * zod schemas for outbound tool-call args.
 *
 * Single source of truth for both runtime validation (in outbound-handlers.ts)
 * and JSON Schema generation for MCP tool listings (in channel/mcp-tools.ts).
 */
import { z } from "zod";

// ── Shared field schemas ────────────────────────────────────────────────
// Reused across multiple tools; declaring once avoids drift.

const NonEmptyString = z.string().min(1);

const MessageFormat = z.enum(["text", "markdown"]);

// ── Channel interaction (handled outside outbound-handlers.ts) ──────────

export const ReplyArgs = z.object({
  text: NonEmptyString,
  reply_to: z.string().optional()
    .describe("Message ID to thread under. Use message_id from the inbound block."),
  files: z.array(z.string()).optional()
    .describe("Absolute file paths to attach."),
  format: MessageFormat.optional().describe("Rendering mode. Default: 'text'."),
});

export const ReactArgs = z.object({
  message_id: NonEmptyString,
  emoji: NonEmptyString,
});

export const EditMessageArgs = z.object({
  message_id: NonEmptyString,
  text: NonEmptyString,
  format: MessageFormat.optional().describe("Rendering mode. Default: 'text'."),
});

export const DownloadAttachmentArgs = z.object({
  file_id: NonEmptyString.describe("The attachment_file_id from inbound meta"),
});

// ── Schedules ───────────────────────────────────────────────────────────

export const CreateScheduleArgs = z.object({
  cron: NonEmptyString.describe("Cron expression, e.g. '0 7 * * *' (every day at 7 AM)"),
  message: NonEmptyString.describe("Message to inject when triggered"),
  target: z.string().optional()
    .describe("Target instance name. Defaults to this instance if omitted."),
  label: z.string().optional().describe("Human-readable name for this schedule"),
  timezone: z.string().optional()
    .describe("IANA timezone, e.g. 'Asia/Taipei'. Defaults to Asia/Taipei."),
});

export const ListSchedulesArgs = z.object({
  target: z.string().optional().describe("Filter by target instance name"),
});

export const UpdateScheduleArgs = z.object({
  id: NonEmptyString.describe("Schedule ID"),
  cron: z.string().optional().describe("New cron expression"),
  message: z.string().optional().describe("New message"),
  target: z.string().optional().describe("New target instance"),
  label: z.string().optional().describe("New label"),
  timezone: z.string().optional().describe("New timezone"),
  enabled: z.boolean().optional().describe("Enable/disable the schedule"),
});

export const DeleteScheduleArgs = z.object({
  id: NonEmptyString.describe("Schedule ID to delete"),
});

// ── Fleet Task Board ────────────────────────────────────────────────────

export const TaskBoardArgs = z.object({
  action: z.enum(["create", "list", "claim", "done", "update"])
    .describe("Operation to perform"),
  title: z.string().optional().describe("Task title (create)"),
  description: z.string().optional().describe("Task details (create)"),
  priority: z.enum(["low", "normal", "high", "urgent"]).optional()
    .describe("Priority (create/update)"),
  assignee: z.string().optional().describe("Instance name to assign (create/update)"),
  depends_on: z.array(z.string()).optional()
    .describe("Task IDs this depends on (create)"),
  id: z.string().optional().describe("Task ID (claim/done/update)"),
  result: z.string().optional().describe("Completion summary (done)"),
  status: z.enum(["open", "claimed", "done", "blocked", "cancelled"]).optional()
    .describe("New status (update)"),
  filter_assignee: z.string().optional().describe("Filter by assignee (list)"),
  filter_status: z.string().optional().describe("Filter by status (list)"),
});

// ── Shared Decisions ────────────────────────────────────────────────────

export const PostDecisionArgs = z.object({
  title: NonEmptyString.describe("Short title for the decision"),
  content: NonEmptyString.describe("Full decision description"),
  scope: z.enum(["project", "fleet"]).optional()
    .describe("'project' (default) = same working directory. 'fleet' = all instances."),
  tags: z.array(z.string()).optional()
    .describe("Optional tags for categorization"),
  ttl_days: z.number().optional()
    .describe("Days until auto-archive. Default: permanent. Set e.g. 7 for temporary decisions."),
  supersedes: z.string().optional()
    .describe("Decision ID to supersede (marks old one as superseded)"),
});

export const ListDecisionsArgs = z.object({
  include_archived: z.boolean().optional()
    .describe("Include archived/superseded decisions. Default: false"),
  tags: z.array(z.string()).optional().describe("Filter by tags"),
});

export const UpdateDecisionArgs = z.object({
  id: NonEmptyString.describe("Decision ID"),
  content: z.string().optional().describe("Updated content"),
  tags: z.array(z.string()).optional().describe("Updated tags"),
  ttl_days: z.number().optional().describe("Updated TTL in days"),
  archive: z.boolean().optional().describe("Set to true to archive this decision"),
});

// ── Identity ────────────────────────────────────────────────────────────

export const SetDisplayNameArgs = z.object({
  name: NonEmptyString.describe("Your chosen display name"),
});

export const SetDescriptionArgs = z.object({
  description: NonEmptyString.describe(
    "Your role description, e.g. 'Code reviewer focused on security and error handling'",
  ),
});

// ── Repo checkout ───────────────────────────────────────────────────────

export const CheckoutRepoArgs = z.object({
  source: NonEmptyString.describe("Repo path (absolute or ~-prefixed) or instance name."),
  branch: z.string().optional().describe("Branch or commit to checkout. Default: HEAD."),
});

export const ReleaseRepoArgs = z.object({
  path: NonEmptyString.describe("Path returned by checkout_repo."),
});

// ── Instance management ─────────────────────────────────────────────────

export const ListInstancesArgs = z.object({
  tags: z.array(z.string()).optional().describe("Filter by tags"),
});

export const DescribeInstanceArgs = z.object({
  name: NonEmptyString.describe("Instance name to describe."),
});

export const StartInstanceArgs = z.object({
  name: NonEmptyString.describe("The instance name to start (from list_instances)"),
});

export const DeleteInstanceArgs = z.object({
  name: NonEmptyString.describe("The instance name to delete (from list_instances)"),
  delete_topic: z.boolean().optional()
    .describe("Whether to also delete the Telegram topic. Defaults to false."),
});

export const ReplaceInstanceArgs = z.object({
  name: NonEmptyString.describe("The instance name to replace"),
  reason: z.string().optional()
    .describe("Why the instance is being replaced (e.g. 'context polluted', 'stuck in loop')"),
});

// create_instance accepts the externally documented fields. passthrough() keeps
// any unknown keys so internal callers (deploy_template) can forward extras
// (profile-derived fields, start_point, …) without the schema stripping them.
export const CreateInstanceArgs = z.object({
  directory: z.string().optional().describe(
    "Absolute path or ~-prefixed path to the project directory. Optional — omit to auto-create a workspace at ~/.agend/workspaces/<name>.",
  ),
  topic_name: z.string().optional().describe(
    "Name for the Telegram topic. Defaults to directory basename. Required when directory is omitted.",
  ),
  description: z.string().optional().describe(
    "Human-readable description of what this instance does (e.g., 'Daily secretary for scheduling and reminders').",
  ),
  model: z.string().optional().describe(
    "Model to use. Claude: sonnet, opus, haiku, opusplan, best, sonnet[1m], opus[1m]. Codex: gpt-4o, o3. Gemini: gemini-2.5-pro. Omit for default.",
  ),
  backend: z.enum(["claude-code", "gemini-cli", "codex", "opencode", "kiro-cli"]).optional()
    .describe("CLI backend to use. Defaults to claude-code."),
  branch: z.string().optional().describe(
    "Git branch name. When specified, creates a git worktree from the directory's repo and uses it as the working directory. If the branch doesn't exist, it will be created.",
  ),
  detach: z.boolean().optional().describe(
    "Use detached HEAD (read-only). Useful for review instances that shouldn't commit to the branch.",
  ),
  worktree_path: z.string().optional().describe(
    "Custom path for the git worktree. Defaults to sibling directory of the repo.",
  ),
  systemPrompt: z.string().optional().describe(
    "Custom system prompt. Supports comma-separated file: paths for modularization (e.g. 'file:prompts/role.md, file:prompts/rules.md'). Injected after fleet context.",
  ),
  tags: z.array(z.string()).optional().describe("Tags for categorization and filtering."),
  workflow: z.string().optional().describe(
    "Workflow template. 'builtin' (default), 'false' to disable, or custom text.",
  ),
}).passthrough();

// ── Cross-instance communication ────────────────────────────────────────

export const SendToInstanceArgs = z.object({
  instance_name: NonEmptyString.describe(
    "Name of the target instance (e.g., 'ccplugin', 'blog-t1385'). Use list_instances to see available instances.",
  ),
  message: NonEmptyString.describe("The message to send to the target instance."),
  request_kind: z.enum(["query", "task", "report", "update"]).optional().describe(
    "Categorizes the message intent. 'query' = asking a question, 'task' = delegating work, 'report' = returning results, 'update' = status notification.",
  ),
  requires_reply: z.boolean().optional()
    .describe("Whether you expect the recipient to respond. Default: false."),
  correlation_id: z.string().optional()
    .describe("Echo this from a previous message to link request-response pairs."),
  task_summary: z.string().optional()
    .describe("Brief summary of the task or request (shown in logs and Telegram visibility posts)."),
  working_directory: z.string().optional()
    .describe("Working directory context to pass along (e.g. the repo path you are working in)."),
  branch: z.string().optional().describe("Git branch context to pass along."),
});


// broadcast excludes "report" (broadcasts don't reply to a specific correlation);
// send_to_instance accepts all four inline.
const BroadcastRequestKind = z.enum(["query", "task", "update"]);

export const BroadcastArgs = z.object({
  message: NonEmptyString.describe("Message to send"),
  targets: z.array(z.string()).optional().describe("Instance names. Omit for all running."),
  team: z.string().optional()
    .describe("Team name. Send to all running members of this team. Overrides targets."),
  tags: z.array(z.string()).optional()
    .describe("Filter targets by tags. Only instances with matching tags receive the message."),
  task_summary: z.string().optional().describe("Brief summary shown in logs"),
  request_kind: BroadcastRequestKind.optional().describe("Message intent"),
  requires_reply: z.boolean().optional().describe("Whether recipients should reply"),
});

export const RequestInformationArgs = z.object({
  target_instance: NonEmptyString.describe("Name of the instance to ask."),
  question: NonEmptyString.describe("The question to ask."),
  context: z.string().optional().describe("Optional context to help the recipient answer."),
});

export const DelegateTaskArgs = z.object({
  target_instance: NonEmptyString.describe("Name of the instance to delegate to."),
  task: NonEmptyString.describe("Description of the task to perform."),
  success_criteria: z.string().optional()
    .describe("How the recipient should judge if the task is complete."),
  context: z.string().optional().describe("Optional background context for the task."),
});

export const ReportResultArgs = z.object({
  target_instance: NonEmptyString.describe("Name of the instance to report to."),
  correlation_id: z.string().optional()
    .describe("The correlation_id from the original request."),
  summary: NonEmptyString.describe("Summary of the result."),
  artifacts: z.string().optional()
    .describe("Optional details: file paths, commit hashes, URLs, etc."),
});

// ── Teams ───────────────────────────────────────────────────────────────

export const CreateTeamArgs = z.object({
  name: NonEmptyString.describe("Team name (e.g. 'sprint-1', 'reviewers')"),
  members: z.array(z.string()).describe("Instance names to include"),
  description: z.string().optional()
    .describe("Optional description of the team's purpose"),
});

export const DeleteTeamArgs = z.object({
  name: NonEmptyString.describe("Team name to delete"),
});

export const UpdateTeamArgs = z.object({
  name: NonEmptyString.describe("Team name"),
  add: z.array(z.string()).optional()
    .describe("Instance names to add (duplicates ignored)"),
  remove: z.array(z.string()).optional()
    .describe("Instance names to remove"),
});

export const ListTeamsArgs = z.object({});

// ── Fleet Templates ─────────────────────────────────────────────────────

export const DeployTemplateArgs = z.object({
  template: NonEmptyString.describe("Template name from fleet.yaml templates section."),
  directory: NonEmptyString.describe(
    "Working directory (shared by all instances, or base repo for worktrees).",
  ),
  name: z.string().optional().describe(
    "Deployment name (used as team name and instance name prefix). Defaults to template name.",
  ),
  branch: z.string().optional().describe(
    "Git branch — each instance gets its own worktree branched from this.",
  ),
});

export const TeardownDeploymentArgs = z.object({
  name: NonEmptyString.describe("Deployment name (as used in deploy_template)."),
});

export const ListDeploymentsArgs = z.object({});

// ── Helpers ─────────────────────────────────────────────────────────────

/**
 * Validate raw args with a zod schema. Returns a tagged result so callers can
 * propagate a clean error message to the agent without leaking internals.
 */
export function validateArgs<T>(
  schema: z.ZodType<T>,
  args: unknown,
  toolName: string,
): { ok: true; data: T } | { ok: false; error: string } {
  const parsed = schema.safeParse(args);
  if (parsed.success) return { ok: true, data: parsed.data };
  const detail = parsed.error.issues
    .map((i) => `${i.path.join(".") || "(args)"}: ${i.message}`)
    .join("; ");
  return { ok: false, error: `Invalid args for ${toolName}: ${detail}` };
}
