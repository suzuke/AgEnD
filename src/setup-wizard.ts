import { createInterface } from "node:readline/promises";
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";
import { stdin, stdout } from "node:process";

const DATA_DIR = join(homedir(), ".claude-channel-daemon");

export function validateBotToken(token: string): boolean {
  return /^\d+:[A-Za-z0-9_-]{30,}$/.test(token);
}

export async function verifyBotToken(
  token: string,
): Promise<{ valid: boolean; username: string | null }> {
  try {
    const res = await fetch(`https://api.telegram.org/bot${token}/getMe`);
    const data = await res.json();
    if (data.ok && data.result?.username) {
      return { valid: true, username: data.result.username };
    }
    return { valid: false, username: null };
  } catch {
    return { valid: false, username: null };
  }
}

export async function runSetupWizard(): Promise<void> {
  const rl = createInterface({ input: stdin, output: stdout });

  console.log("\nclaude-channel-daemon setup\n");

  // Step 1: Check prerequisites
  console.log("Checking prerequisites...");
  const { execSync } = await import("node:child_process");
  try {
    execSync("claude --version", { stdio: "pipe" });
    console.log("  Claude Code installed");
  } catch {
    console.error("  Claude Code not found. Install: https://docs.anthropic.com/en/docs/claude-code");
    rl.close();
    process.exit(1);
  }

  // Step 2: Bot token with API validation
  const token = await rl.question("Telegram Bot Token (from @BotFather): ");
  if (!validateBotToken(token.trim())) {
    console.error("Invalid token format. Expected: 123456789:ABC...");
    rl.close();
    process.exit(1);
  }

  console.log("Verifying token with Telegram API...");
  const verification = await verifyBotToken(token.trim());
  if (!verification.valid) {
    console.error("Token rejected by Telegram API. Check your token and try again.");
    rl.close();
    process.exit(1);
  }
  console.log(`  Token valid — bot username: @${verification.username}`);

  mkdirSync(DATA_DIR, { recursive: true });
  writeFileSync(join(DATA_DIR, ".env"), `TELEGRAM_BOT_TOKEN=${token.trim()}\n`);
  console.log("  Token saved");

  // Step 3: Working directory
  const workDir = await rl.question(`Working directory [${homedir()}]: `);
  const resolvedWorkDir = workDir.trim() || homedir();

  // Step 4: Write config
  const configContent = `channel_plugin: telegram@claude-plugins-official
working_directory: ${resolvedWorkDir}
restart_policy:
  max_retries: 10
  backoff: exponential
  reset_after: 300
context_guardian:
  threshold_percentage: 80
  max_age_hours: 4
  strategy: hybrid
memory:
  auto_summarize: true
  watch_memory_dir: true
  backup_to_sqlite: true
log_level: info
`;
  writeFileSync(join(DATA_DIR, "config.yaml"), configContent);
  console.log("  Config saved");

  // Step 5: System service
  const svcAnswer = await rl.question("Install as system service? (Y/n): ");
  if (svcAnswer.trim().toLowerCase() !== "n") {
    const { installService } = await import("./service-installer.js");
    const path = installService({
      label: "com.claude-channel-daemon",
      execPath: process.argv[1],
      workingDirectory: resolvedWorkDir,
      logPath: join(DATA_DIR, "daemon.log"),
    });
    console.log(`  Service installed at: ${path}`);
  }

  console.log(`\nSetup complete! Your bot @${verification.username} is ready.`);
  console.log("Run: claude-channel-daemon start\n");
  rl.close();
}
