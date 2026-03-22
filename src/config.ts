import { readFileSync, existsSync } from "node:fs";
import yaml from "js-yaml";
import type { DaemonConfig, FleetConfig, InstanceConfig } from "./types.js";

export const DEFAULT_CONFIG: DaemonConfig = {
  channel_plugin: "telegram@claude-plugins-official",
  working_directory: process.env.HOME || "~",
  restart_policy: {
    max_retries: 10,
    backoff: "exponential",
    reset_after: 300,
  },
  context_guardian: {
    threshold_percentage: 60,
    max_idle_wait_ms: 300_000,
    completion_timeout_ms: 60_000,
    grace_period_ms: 600_000,
    max_age_hours: 8,
  },
  memory: {
    auto_summarize: true,
    watch_memory_dir: true,
    backup_to_sqlite: true,
  },
  log_level: "info",
};

function deepMergeGeneric<T extends object>(target: T, source: Partial<T>): T {
  const result = { ...target } as Record<string, unknown>;
  const sourceRecord = source as Record<string, unknown>;

  for (const key of Object.keys(sourceRecord)) {
    const sourceVal = sourceRecord[key];
    const targetVal = result[key];
    if (
      sourceVal !== null &&
      typeof sourceVal === "object" &&
      !Array.isArray(sourceVal) &&
      typeof targetVal === "object" &&
      targetVal !== null &&
      !Array.isArray(targetVal)
    ) {
      result[key] = deepMergeGeneric(
        targetVal as object,
        sourceVal as Partial<object>,
      );
    } else if (sourceVal !== undefined) {
      result[key] = sourceVal;
    }
  }
  return result as unknown as T;
}

function deepMerge(target: DaemonConfig, source: Partial<DaemonConfig>): DaemonConfig {
  return deepMergeGeneric(target, source);
}

export const DEFAULT_INSTANCE_CONFIG: Omit<InstanceConfig, "working_directory"> = {
  restart_policy: DEFAULT_CONFIG.restart_policy,
  context_guardian: DEFAULT_CONFIG.context_guardian,
  memory: DEFAULT_CONFIG.memory,
  log_level: DEFAULT_CONFIG.log_level,
};

export function loadConfig(configPath: string): DaemonConfig {
  if (!existsSync(configPath)) {
    return { ...DEFAULT_CONFIG };
  }
  const raw = readFileSync(configPath, "utf-8");
  const parsed = yaml.load(raw) as Partial<DaemonConfig> | null;
  if (!parsed) {
    return { ...DEFAULT_CONFIG };
  }
  return deepMerge(DEFAULT_CONFIG, parsed);
}

export function loadFleetConfig(configPath: string): FleetConfig {
  if (!existsSync(configPath)) {
    return { defaults: {}, instances: {} };
  }

  const raw = readFileSync(configPath, "utf-8");
  const parsed = yaml.load(raw) as {
    channel?: FleetConfig["channel"];
    project_roots?: string[];
    defaults?: Partial<InstanceConfig>;
    instances?: Record<string, Partial<InstanceConfig>>;
  } | null;

  if (!parsed) {
    return { defaults: {}, instances: {} };
  }

  const fleetDefaults: Partial<InstanceConfig> = parsed.defaults ?? {};
  const rawInstances = parsed.instances ?? {};
  const instances: Record<string, InstanceConfig> = {};

  for (const [name, overrides] of Object.entries(rawInstances)) {
    const merged = deepMergeGeneric(
      deepMergeGeneric(DEFAULT_INSTANCE_CONFIG as Partial<InstanceConfig>, fleetDefaults),
      overrides,
    ) as Partial<InstanceConfig>;

    if (!merged.working_directory) {
      throw new Error(
        `Instance "${name}" is missing required field: working_directory`,
      );
    }

    instances[name] = merged as InstanceConfig;
  }

  return {
    channel: parsed.channel,
    project_roots: parsed.project_roots,
    defaults: fleetDefaults,
    instances,
  };
}
