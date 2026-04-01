export interface Schedule {
  id: string;
  cron: string;
  message: string;
  source: string;
  target: string;
  reply_chat_id: string;
  reply_thread_id: string | null;
  label: string | null;
  enabled: boolean;
  timezone: string;
  created_at: string;
  last_triggered_at: string | null;
  last_status: string | null;
}

export interface ScheduleRun {
  id: number;
  schedule_id: string;
  triggered_at: string;
  status: "delivered" | "delivered_fallback" | "retry" | "instance_offline" | "channel_dead";
  detail: string | null;
}

export interface CreateScheduleParams {
  cron: string;
  message: string;
  source: string;
  target: string;
  reply_chat_id: string;
  reply_thread_id: string | null;
  label?: string;
  timezone?: string;
}

export interface UpdateScheduleParams {
  cron?: string;
  message?: string;
  target?: string;
  label?: string;
  timezone?: string;
  enabled?: boolean;
}

export interface SchedulerConfig {
  max_schedules: number;
  default_timezone: string;
  retry_count: number;
  retry_interval_ms: number;
}

export const DEFAULT_SCHEDULER_CONFIG: SchedulerConfig = {
  max_schedules: 100,
  default_timezone: "Asia/Taipei",
  retry_count: 3,
  retry_interval_ms: 30_000,
};

// ── Shared Decisions ──────────────────────────────────────────

export type DecisionStatus = "active" | "superseded" | "archived";
export type DecisionScope = "project" | "fleet";

export interface Decision {
  id: string;
  project_root: string;
  scope: DecisionScope;
  title: string;
  content: string;
  tags: string[];
  status: DecisionStatus;
  superseded_by: string | null;
  created_by: string;
  created_at: string;
  expires_at: string | null;
  updated_at: string;
}

export interface CreateDecisionParams {
  project_root: string;
  scope?: DecisionScope; // "project" (default) or "fleet" (visible to all instances)
  title: string;
  content: string;
  tags?: string[];
  ttl_days?: number; // days until auto-archive. 0 or omitted = permanent
  created_by: string;
  supersedes?: string; // decision id to supersede
}

export interface UpdateDecisionParams {
  content?: string;
  tags?: string[];
  ttl_days?: number;
}

// ── Fleet Task Board ──────────────────────────────────────────

export type TaskStatus = "open" | "claimed" | "done" | "blocked" | "cancelled";
export type TaskPriority = "low" | "normal" | "high" | "urgent";

export interface Task {
  id: string;
  title: string;
  description: string | null;
  status: TaskStatus;
  priority: TaskPriority;
  assignee: string | null;
  created_by: string;
  depends_on: string[];
  result: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateTaskParams {
  title: string;
  description?: string;
  priority?: TaskPriority;
  assignee?: string;
  depends_on?: string[];
  created_by: string;
}

export interface UpdateTaskParams {
  status?: TaskStatus;
  assignee?: string;
  result?: string;
  priority?: TaskPriority;
}
