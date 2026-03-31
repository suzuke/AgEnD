import { join } from "node:path";
import { execSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { type CliBackend, type CliBackendConfig, resolveBinary } from "./types.js";

export class CodexBackend implements CliBackend {
  readonly binaryName = "codex";
  private binaryPath: string;

  constructor(private instanceDir: string) {
    this.binaryPath = resolveBinary("codex");
  }

  buildCommand(config: CliBackendConfig): string {
    let cmd = `${this.binaryPath} --full-auto`;

    if (config.model) {
      cmd += ` -c model="${config.model}"`;
    }

    // Codex reads AGENTS.md from the working directory for system instructions.
    // Write the prompt there so Codex auto-loads it on startup.
    if (config.systemPrompt) {
      const promptPath = join(this.instanceDir, "system-prompt.md");
      writeFileSync(promptPath, config.systemPrompt);
    }

    return cmd;
  }

  writeConfig(config: CliBackendConfig): void {
    // Codex uses codex mcp add (global config), write a setup script
    // that registers MCP servers on first launch
    const setupScript = Object.entries(config.mcpServers).map(([name, entry]) => {
      const args = entry.args.map(a => `"${a}"`).join(" ");
      const envFlags = Object.entries(entry.env || {}).map(([k, v]) => `--env ${k}="${v}"`).join(" ");
      return `codex mcp add ${name} ${envFlags} -- ${entry.command} ${args} 2>/dev/null || true`;
    }).join("\n");
    writeFileSync(join(this.instanceDir, "setup-mcp.sh"), setupScript, { mode: 0o755 });

    // Run setup immediately
    try {
      execSync(`bash ${join(this.instanceDir, "setup-mcp.sh")}`, { stdio: "ignore" });
    } catch { /* best effort */ }
  }

  getReadyPattern(): RegExp {
    return /% left|OpenAI Codex/m;
  }

  getContextUsage(): number | null {
    return null;
  }

  getSessionId(): string | null {
    try {
      const f = join(this.instanceDir, "session-id");
      return readFileSync(f, "utf-8").trim() || null;
    } catch { return null; }
  }

  cleanup(config: CliBackendConfig): void {
    for (const name of Object.keys(config.mcpServers)) {
      try { execSync(`codex mcp remove ${name}`, { stdio: "ignore" }); } catch { /* best effort */ }
    }
  }
}
