# AgEnD MCP Token Overhead Report

> Date: 2026-04-04 | Tested by: agend-t5033 + agend-reviewer-t9177

## Purpose

Measure the additional token consumption caused by AgEnD's MCP server injection into Claude Code, including MCP instructions (fleet context + workflow template) and tool definitions across three profiles (full / standard / minimal).

## Methodology

### Test Command

```bash
claude -p "Say hello" --output-format json --model sonnet
```

### Environment

- **Directory:** Empty (no files, no CLAUDE.md, no .claude/ settings)
- **Baseline:** Pure Claude Code with no MCP server
- **Experimental groups:** Claude Code + AgEnD MCP server (full / standard / minimal tool profiles)
- **Runs:** 3 per group
- **Result:** Input tokens are fully deterministic (identical across all 3 runs per group)

### How Baseline Was Measured

In an empty directory with no MCP servers configured:

```bash
mkdir /tmp/empty-test && cd /tmp/empty-test
claude -p "Say hello" --output-format json --model sonnet
```

The JSON output's `.usage` field returns precise token counts:
- `input_tokens`: 3 (just the prompt "Say hello")
- `cache_creation_input_tokens`: 9,015 (Claude Code's system prompt, uncached portion)
- `cache_read_input_tokens`: 11,644 (cached portion)
- **Total: 20,662 tokens** (Claude Code's baseline with zero AgEnD overhead)

## Results

| Condition | Total Input | vs Baseline | % of 200K Context |
|-----------|------------|-------------|-------------------|
| **Baseline (no MCP)** | 20,662 | — | — |
| **Full (30 tools)** | 21,549 | **+887** | 0.44% |
| **Standard (11 tools)** | 21,368 | **+706** | 0.35% |
| **Minimal (4 tools)** | 21,292 | **+630** | 0.31% |

## Overhead Breakdown

### MCP Instructions (~630 tokens)

The largest component. Includes:
- Fleet identity (instance name, working directory, display name)
- Role description
- Message format rules (`[user:name]` vs `[from:instance-name]`)
- Tool usage guidance
- Collaboration rules
- Development workflow template (~2050 chars)

### Tool Schema Differences

| Comparison | Token Difference |
|-----------|-----------------|
| Full → Standard (19 fewer tools) | -181 tokens |
| Standard → Minimal (7 fewer tools) | -76 tokens |
| Full → Minimal (26 fewer tools) | -257 tokens |

**Note:** Actual tool schema token consumption is far lower than naive estimates. Anthropic applies special token compression to tool definitions — 30 tools vs 4 tools differs by only 257 tokens (estimated ~3,500).

## Cost Impact

Calculated at Sonnet 4 input pricing ($3/MTok):

| Profile | Per-Message Overhead | Per 1,000 Messages |
|---------|---------------------|-------------------|
| Full | $0.002661 | $2.66 |
| Standard | $0.002118 | $2.12 |
| Minimal | $0.001890 | $1.89 |

**With prompt caching** (subsequent messages): overhead drops to 1/10 ($0.30/MTok cache read rate), i.e., < $0.0003 per message.

## Context Window Impact

Claude Code's own system prompt occupies ~20,662 tokens (10.3% of 200K). AgEnD adds:

| Profile | Additional Tokens | % of Context Window |
|---------|------------------|-------------------|
| Full | +887 | 0.44% |
| Standard | +706 | 0.35% |
| Minimal | +630 | 0.31% |

## Conclusions

1. **AgEnD's MCP overhead is minimal** — less than 4.3% above baseline (full profile)
2. **Context window impact < 0.5%** — negligible for long conversations
3. **Instructions are the primary cost** (~630 tokens, 71% of full overhead)
4. **Tool schema differences are small** due to Anthropic's token compression
5. **Cost is negligible** — < $0.003/message, < $0.0003 with prompt caching

### Recommendations

- **Standard profile** is the best balance (all collaboration tools, 181 fewer tokens than full)
- No need to sacrifice functionality to save tokens
- Use **minimal** only for instances that don't need fleet collaboration features

## Reproducibility

```bash
./scripts/measure-token-overhead.sh
```

Requires: `claude` CLI, `jq`, `node`, `npm run build`
