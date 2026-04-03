#!/usr/bin/env node
/**
 * Mock CLI backend for E2E testing.
 *
 * Replaces the real claude/gemini/codex CLI. It:
 * 1. Spawns the real agend MCP server as a child (stdio inherited for debug)
 * 2. Writes periodic statusline.json updates
 * 3. Reads stdin for inbound messages from daemon
 * 4. Outputs "MOCK_READY" once MCP server is connected
 *
 * Environment variables:
 *   AGEND_SOCKET_PATH  — path to daemon's Unix socket (required)
 *   AGEND_INSTANCE_NAME — instance name (required)
 *   MOCK_INSTANCE_DIR  — instance directory for statusline.json (required)
 *   MOCK_RESPONSE      — default text response (optional)
 *   MOCK_DELAY         — response delay in ms (optional, default 100)
 *   MOCK_CONTEXT_PCT   — simulated context window usage % (optional, default 10)
 */

import { spawn } from "node:child_process";
import { writeFileSync, existsSync, readFileSync, watchFile, unwatchFile } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const SOCKET_PATH = process.env.AGEND_SOCKET_PATH;
const INSTANCE_NAME = process.env.AGEND_INSTANCE_NAME;
const INSTANCE_DIR = process.env.MOCK_INSTANCE_DIR;
const MOCK_RESPONSE = process.env.MOCK_RESPONSE ?? "Mock response from mock-claude";
const MOCK_DELAY = parseInt(process.env.MOCK_DELAY ?? "100", 10) || 100;
const MOCK_CONTEXT_PCT = parseInt(process.env.MOCK_CONTEXT_PCT ?? "10", 10) || 10;

if (!SOCKET_PATH || !INSTANCE_NAME || !INSTANCE_DIR) {
  process.stderr.write("mock-claude: missing required env vars (AGEND_SOCKET_PATH, AGEND_INSTANCE_NAME, MOCK_INSTANCE_DIR)\n");
  process.exit(1);
}

// ---------------------------------------------------------------------------
// 1. Find and spawn the real MCP server
// ---------------------------------------------------------------------------

const projectRoot = join(__dirname, "..", "..");
let mcpServerPath = join(projectRoot, "dist", "channel", "mcp-server.js");
if (!existsSync(mcpServerPath)) {
  mcpServerPath = join(projectRoot, "src", "channel", "mcp-server.ts");
}

// Read MCP config to get env vars for the server
let mcpEnv = {};
const mcpConfigPath = join(INSTANCE_DIR, "mcp-config.json");
if (existsSync(mcpConfigPath)) {
  try {
    const config = JSON.parse(readFileSync(mcpConfigPath, "utf-8"));
    const agendServer = config.mcpServers?.agend;
    if (agendServer?.env) {
      mcpEnv = agendServer.env;
    }
  } catch { /* ignore */ }
}

const runner = mcpServerPath.endsWith(".ts") ? "tsx" : "node";

// Fix #2: inherit stdout so MCP server's JSON-RPC output doesn't fill pipe buffer.
// In mock mode, no real AI reads the stdio transport — inheriting is safe and helps debug.
const mcpChild = spawn(runner, [mcpServerPath], {
  stdio: ["pipe", "inherit", "inherit"],
  env: {
    ...process.env,
    ...mcpEnv,
    AGEND_SOCKET_PATH: SOCKET_PATH,
    AGEND_INSTANCE_NAME: INSTANCE_NAME,
  },
});

mcpChild.on("error", (err) => {
  process.stderr.write(`mock-claude: MCP server spawn error: ${err.message}\n`);
});

mcpChild.on("exit", (code) => {
  process.stderr.write(`mock-claude: MCP server exited with code ${code}\n`);
});

// ---------------------------------------------------------------------------
// 2. Write periodic statusline.json (matching real Claude Code format)
// ---------------------------------------------------------------------------

let contextPct = MOCK_CONTEXT_PCT;
const sessionId = `mock-${INSTANCE_NAME}-${Date.now()}`;

function writeStatusline() {
  const statusline = {
    session_id: sessionId,
    model: "mock-model",
    cost_usd: 0,
    total_tokens: 1000,
    context_window: {
      used_percentage: contextPct,
      context_window_size: 200000,
    },
  };
  try {
    writeFileSync(join(INSTANCE_DIR, "statusline.json"), JSON.stringify(statusline));
  } catch { /* ignore write errors during shutdown */ }
}

writeFileSync(join(INSTANCE_DIR, "session-id"), sessionId);
writeStatusline();
const statusTimer = setInterval(writeStatusline, 5000);

// ---------------------------------------------------------------------------
// 3. Handle stdin (daemon sends messages to the CLI via tmux sendKeys)
// ---------------------------------------------------------------------------

const rl = createInterface({ input: process.stdin });

rl.on("line", (line) => {
  const trimmed = line.trim();
  if (!trimmed) return;

  process.stderr.write(`mock-claude: received input: ${trimmed.slice(0, 100)}\n`);

  setTimeout(() => {
    process.stdout.write(`${MOCK_RESPONSE}\n`);
  }, MOCK_DELAY);
});

// ---------------------------------------------------------------------------
// 4. Signal ready — wait for channel.sock to appear (MCP server connected)
// ---------------------------------------------------------------------------

function signalReady() {
  process.stdout.write("MOCK_READY\n");
  process.stderr.write("mock-claude: ready\n");
}

// Poll for channel.sock existence (MCP server creates IPC connection)
const READY_TIMEOUT = parseInt(process.env.MOCK_READY_TIMEOUT ?? "10000", 10) || 10_000;
let readySignaled = false;
const readyTimeout = setTimeout(() => {
  // Fallback: signal ready after timeout even if socket not found
  if (!readySignaled) {
    readySignaled = true;
    process.stderr.write("mock-claude: timeout waiting for channel.sock, signaling ready anyway\n");
    signalReady();
  }
}, READY_TIMEOUT);

const socketCheckInterval = setInterval(() => {
  if (readySignaled) {
    clearInterval(socketCheckInterval);
    return;
  }
  if (existsSync(SOCKET_PATH)) {
    readySignaled = true;
    clearInterval(socketCheckInterval);
    clearTimeout(readyTimeout);
    // Give MCP server a moment to complete IPC handshake
    setTimeout(signalReady, 200);
  }
}, 200);

// ---------------------------------------------------------------------------
// 5. Cleanup on exit — wait for child to exit
// ---------------------------------------------------------------------------

function cleanup() {
  clearInterval(statusTimer);
  clearInterval(socketCheckInterval);
  clearInterval(controlTimer);
  clearTimeout(readyTimeout);
  mcpChild.kill("SIGTERM");
}

function gracefulExit(code) {
  cleanup();
  mcpChild.on("exit", () => process.exit(code));
  // Force exit after 3s if child doesn't respond
  setTimeout(() => process.exit(code), 3000);
}

process.on("SIGTERM", () => gracefulExit(0));
process.on("SIGINT", () => gracefulExit(0));

// ---------------------------------------------------------------------------
// 6. Runtime control via file (tests can adjust behavior)
// ---------------------------------------------------------------------------

const controlFile = join(INSTANCE_DIR, "mock-control.json");
const controlTimer = setInterval(() => {
  try {
    if (existsSync(controlFile)) {
      const ctrl = JSON.parse(readFileSync(controlFile, "utf-8"));
      if (typeof ctrl.context_pct === "number") contextPct = ctrl.context_pct;
      if (ctrl.exit) gracefulExit(0);
    }
  } catch { /* ignore */ }
}, 1000);
