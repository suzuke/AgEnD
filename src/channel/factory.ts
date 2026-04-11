import type { ChannelAdapter } from "./types.js";
import type { ChannelConfig } from "../types.js";
import type { AccessManager } from "./access-manager.js";
import { execSync } from "node:child_process";
import { join } from "node:path";
import { existsSync, readFileSync } from "node:fs";

export interface AdapterOpts {
  id: string;
  botToken: string;
  accessManager: AccessManager;
  inboxDir: string;
}

/** Factory function that external adapter packages must default-export. */
export type AdapterFactory = (config: ChannelConfig, opts: AdapterOpts) => ChannelAdapter;

/** Resolve the entry point for a global npm package. */
function resolveGlobalPackage(pkg: string): string | null {
  try {
    const globalRoot = execSync("npm root -g", { stdio: "pipe" }).toString().trim();
    const pkgDir = join(globalRoot, pkg);
    if (!existsSync(pkgDir)) return null;
    // Read main from package.json, fallback to dist/index.js
    try {
      const pkgJson = JSON.parse(readFileSync(join(pkgDir, "package.json"), "utf-8"));
      return join(pkgDir, pkgJson.main ?? "dist/index.js");
    } catch {
      return join(pkgDir, "dist/index.js");
    }
  } catch { return null; }
}

/** Try to import a plugin by name — local node_modules first, then global. */
async function tryImportPlugin(pkg: string): Promise<{ default: unknown } | null> {
  try { return await import(pkg); } catch { /* not in local node_modules */ }
  const globalPath = resolveGlobalPackage(pkg);
  if (globalPath) {
    try { return await import(globalPath); } catch { /* global import failed */ }
  }
  return null;
}

export async function createAdapter(config: ChannelConfig, opts: AdapterOpts): Promise<ChannelAdapter> {
  // Built-in adapters
  if (config.type === "telegram") {
    const { TelegramAdapter } = await import("./adapters/telegram.js");
    return new TelegramAdapter({ ...opts, apiRoot: config.telegram_api_root });
  }

  // Plugin adapters — try multiple package name conventions
  const candidates = [
    `@suzuke/agend-plugin-${config.type}`, // scoped official plugin
    `agend-plugin-${config.type}`,          // community plugin
    `agend-adapter-${config.type}`,         // legacy convention
    config.type,                             // bare name
  ];

  for (const pkg of candidates) {
    const mod = await tryImportPlugin(pkg);
    if (!mod) continue;
    const factory = mod.default;
    // Support both: factory function and object with createAdapter method
    if (typeof factory === "function") return factory(config, opts) as ChannelAdapter;
    if (factory && typeof (factory as Record<string, unknown>).createAdapter === "function") {
      return (factory as { createAdapter: AdapterFactory }).createAdapter(config, opts);
    }
  }

  throw new Error(
    `Channel adapter "${config.type}" not found. ` +
    `Install the plugin: npm install -g @suzuke/agend-plugin-${config.type}`
  );
}
