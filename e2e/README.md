# agend E2E Testing Environment

## Architecture

```
Host (macOS, Apple Silicon)
├── Mock Telegram Bot API    (localhost:8443)
├── Mock Anthropic API       (localhost:8444)
├── Tart VM (macOS, headless, SSH)
│   ├── agend (npm linked from shared dir)
│   ├── fleet.yaml → mock servers via host IP
│   ├── .env (mock bot token, mock API key)
│   └── tmux session running fleet
└── Test Runner (Vitest)
    ├── Controls mock servers (inject messages, set responses)
    ├── SSH into VM to trigger/verify agend behavior
    └── Asserts on mock server call logs
```

## How It Works

1. **Golden Image**: A Tart macOS VM with Node.js, tmux, and agend dependencies pre-installed.
2. **Mock Servers**: Express servers on the host that mimic Telegram Bot API and Anthropic API.
   - Telegram mock: accepts `sendMessage`, `getUpdates`, webhook calls, etc.
   - AI mock: returns canned Claude API responses.
3. **Test Flow**: `tart clone → tart run --no-graphics → SSH provision → run tests → tart delete`
4. **No real APIs**: Everything runs locally with fake tokens and mock endpoints.

## Key Design Decisions

- **Mock servers on host, not in VM** — simpler, easier to debug, VM just runs agend.
- **Shell script for VM setup** (not Packer) — KISS, Packer is overkill for local dev.
- **Vitest as test runner** — consistent with existing test infrastructure.
- **grammy apiRoot override** — grammy `Bot` supports `client.apiRoot` option; we add
  `telegram_api_root` field in `fleet.yaml`'s channel config to redirect API calls to our mock.
- **ANTHROPIC_BASE_URL** — already supported by agend, points to mock AI server.

## Directory Structure

```
e2e/
├── README.md                 # This file
├── mock-servers/
│   ├── telegram-mock.ts      # Mock Telegram Bot API
│   ├── ai-mock.ts            # Mock Anthropic/Claude API
│   └── shared.ts             # Shared utilities (logging, state)
├── vm-setup/
│   ├── setup-vm.sh           # Golden image provisioning script
│   └── fleet-test.yaml       # Test fleet configuration template
├── scripts/
│   ├── run-e2e.sh            # Full E2E test lifecycle
│   └── ssh-cmd.sh            # Helper: run command in VM via SSH
├── tests/
│   ├── mock-infrastructure.test.ts # Mock server + backend verification
│   ├── instance-crud.test.ts       # Instance create/delete (T3/T4)
│   ├── fleet-lifecycle.test.ts     # Fleet start/stop (T1/T2) [TODO]
│   ├── cross-instance.test.ts      # send_to_instance (T7) [TODO]
│   ├── context-rotation.test.ts    # Context rotation (T10) [TODO]
│   ├── mcp-instructions.test.ts    # MCP injection (T16) [TODO]
│   └── team-broadcast.test.ts      # Team broadcast (T8) [TODO]
└── vitest.config.e2e.ts      # E2E-specific Vitest config
```

## Prerequisites

```bash
brew install cirruslabs/cli/tart
brew install cirruslabs/cli/sshpass
```

## Quick Start

```bash
# 1. Build golden image (first time only, ~10 min)
./e2e/vm-setup/setup-vm.sh

# 2. Run E2E tests
./e2e/scripts/run-e2e.sh
```
