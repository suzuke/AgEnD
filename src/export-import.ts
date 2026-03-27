import { execFileSync } from "node:child_process";
import {
  existsSync,
  statSync,
  mkdirSync,
  copyFileSync,
  readdirSync,
  readFileSync,
} from "node:fs";
import { join, basename, resolve } from "node:path";
import { homedir } from "node:os";

const MINIMAL_FILES = ["fleet.yaml", ".env", "scheduler.db"];
const RUNTIME_EXCLUDES = [
  "*.sock",
  "*.pid",
  "*.log",
  "output.log",
  "fleet.log",
  "node_modules",
];

export async function exportConfig(
  dataDir: string,
  outputPath: string | undefined,
  full: boolean
): Promise<void> {
  if (!existsSync(dataDir)) {
    console.error(`Data directory not found: ${dataDir}`);
    process.exit(1);
  }

  const date = new Date().toISOString().slice(0, 10);
  const outFile = resolve(outputPath ?? `ccd-export-${date}.tar.gz`);

  if (full) {
    const excludeArgs = RUNTIME_EXCLUDES.flatMap((p) => ["--exclude", p]);
    execFileSync("tar", [
      "czf", outFile, ...excludeArgs,
      "-C", join(dataDir, ".."), basename(dataDir),
    ], { stdio: "pipe" });
  } else {
    // Minimal: only config files that exist
    const existing = MINIMAL_FILES.filter((f) => existsSync(join(dataDir, f)));
    if (existing.length === 0) {
      console.error("No config files found to export.");
      process.exit(1);
    }
    const fileArgs = existing.map((f) => `${basename(dataDir)}/${f}`);
    execFileSync("tar", [
      "czf", outFile, "-C", join(dataDir, ".."), ...fileArgs,
    ], { stdio: "pipe" });
  }

  const size = statSync(outFile).size;
  const sizeStr =
    size < 1024
      ? `${size} B`
      : size < 1024 * 1024
        ? `${(size / 1024).toFixed(1)} KB`
        : `${(size / (1024 * 1024)).toFixed(1)} MB`;

  console.log(`Exported to: ${outFile} (${sizeStr})`);
  console.warn(
    "\n⚠️  This file contains secrets (bot token, API keys). Transfer securely."
  );
}

export async function importConfig(
  dataDir: string,
  filePath: string
): Promise<void> {
  const absFile = resolve(filePath);
  if (!existsSync(absFile)) {
    console.error(`File not found: ${absFile}`);
    process.exit(1);
  }

  mkdirSync(dataDir, { recursive: true });

  // Backup existing config files
  const timestamp = Date.now();
  for (const name of ["fleet.yaml", ".env"]) {
    const target = join(dataDir, name);
    if (existsSync(target)) {
      const bakPath = `${target}.bak.${timestamp}`;
      copyFileSync(target, bakPath);
      console.log(`Backed up: ${name} → ${basename(bakPath)}`);
    }
  }

  // Extract — strip the top-level directory name
  execFileSync("tar", ["xzf", absFile, "-C", join(dataDir, "..")], { stdio: "pipe" });
  console.log(`Imported to: ${dataDir}`);

  // Validate paths in fleet.yaml
  const fleetPath = join(dataDir, "fleet.yaml");
  if (existsSync(fleetPath)) {
    const yaml = await import("js-yaml");
    const config = yaml.load(readFileSync(fleetPath, "utf-8")) as any;
    const missing: string[] = [];

    // Check project_roots
    if (Array.isArray(config?.project_roots)) {
      for (const root of config.project_roots) {
        const expanded = expandHome(root);
        if (!existsSync(expanded)) missing.push(expanded);
      }
    }

    // Check instance working directories
    if (config?.instances) {
      for (const [name, inst] of Object.entries<any>(config.instances)) {
        if (inst?.working_directory) {
          const expanded = expandHome(inst.working_directory);
          if (!existsSync(expanded)) missing.push(expanded);
        }
      }
    }

    if (missing.length > 0) {
      console.warn(`\n⚠️  The following paths in fleet.yaml do not exist on this device:`);
      for (const p of missing) {
        console.warn(`   • ${p}`);
      }
      console.warn(`\nEdit ${fleetPath} to fix these before running 'ccd fleet start'.`);
    } else {
      console.log("\nAll paths in fleet.yaml verified.");
    }
  }
}

function expandHome(p: string): string {
  if (p.startsWith("~/")) return join(homedir(), p.slice(2));
  return p;
}
