#!/bin/bash
# Token Overhead Measurement Script
# Uses `claude -p --output-format json` to measure AgEnD MCP overhead.
#
# Prerequisites: claude CLI, node, jq, npm run build
#
# Usage: ./scripts/measure-token-overhead.sh

set -euo pipefail

MODEL="sonnet"
PROMPT="Say hello"
RUNS=3

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
MCP_SERVER="$PROJECT_ROOT/dist/channel/mcp-server.js"

if [ ! -f "$MCP_SERVER" ]; then
  echo "Error: dist not found. Run 'npm run build' first."
  exit 1
fi

TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

# Create empty test directory (no CLAUDE.md, no .claude/)
TEST_DIR="$TMPDIR/test-project"
mkdir -p "$TEST_DIR"

# Generate MCP configs for each tool profile
for profile in full standard minimal; do
  cat > "$TMPDIR/mcp-$profile.json" <<MCPEOF
{
  "mcpServers": {
    "agend": {
      "command": "node",
      "args": ["$MCP_SERVER"],
      "env": {
        "AGEND_SOCKET_PATH": "/tmp/agend-token-test-dummy.sock",
        "AGEND_INSTANCE_NAME": "token-test",
        "AGEND_WORKING_DIR": "$TEST_DIR",
        "AGEND_TOOL_SET": "$profile"
      }
    }
  }
}
MCPEOF
done

echo "=========================================="
echo "AgEnD Token Overhead Measurement"
echo "Model: $MODEL"
echo "Prompt: \"$PROMPT\""
echo "Runs per test: $RUNS"
echo "=========================================="
echo ""

# Function: run claude -p and extract usage
run_test() {
  local label="$1"
  local mcp_config="$2"  # empty = no MCP

  local args=("-p" "$PROMPT" "--output-format" "json" "--model" "$MODEL")
  if [ -n "$mcp_config" ]; then
    args+=("--mcp-config" "$mcp_config")
  fi

  local output
  output=$(cd "$TEST_DIR" && claude "${args[@]}" 2>/dev/null)

  local input_tokens cache_creation cache_read output_tokens
  input_tokens=$(echo "$output" | jq '.usage.input_tokens // 0')
  cache_creation=$(echo "$output" | jq '.usage.cache_creation_input_tokens // 0')
  cache_read=$(echo "$output" | jq '.usage.cache_read_input_tokens // 0')
  output_tokens=$(echo "$output" | jq '.usage.output_tokens // 0')

  local total_input=$((input_tokens + cache_creation + cache_read))
  echo "$total_input $input_tokens $cache_creation $cache_read $output_tokens"
}

# Test matrix
declare -a GROUP_NAMES=("Baseline" "Full (30 tools)" "Standard (11 tools)" "Minimal (4 tools)")
declare -a GROUP_MCP=("" "$TMPDIR/mcp-full.json" "$TMPDIR/mcp-standard.json" "$TMPDIR/mcp-minimal.json")
declare -a GROUP_AVG_TOTAL=()

echo "Running tests..."
echo ""

for i in "${!GROUP_NAMES[@]}"; do
  label="${GROUP_NAMES[$i]}"
  mcp="${GROUP_MCP[$i]}"

  echo "--- $label ---"

  sum_total=0
  sum_output=0

  for run in $(seq 1 $RUNS); do
    result=$(run_test "$label" "$mcp")
    total=$(echo "$result" | awk '{print $1}')
    input=$(echo "$result" | awk '{print $2}')
    cache_create=$(echo "$result" | awk '{print $3}')
    cache_read_val=$(echo "$result" | awk '{print $4}')
    output=$(echo "$result" | awk '{print $5}')

    sum_total=$((sum_total + total))
    sum_output=$((sum_output + output))

    echo "  Run $run: total_input=$total (uncached=$input cache_create=$cache_create cache_read=$cache_read_val) output=$output"
  done

  avg_total=$((sum_total / RUNS))
  avg_output=$((sum_output / RUNS))
  GROUP_AVG_TOTAL+=("$avg_total")

  echo "  Average: total_input=$avg_total output=$avg_output"
  echo ""
done

# Results summary
baseline=${GROUP_AVG_TOTAL[0]}

echo "=========================================="
echo "RESULTS SUMMARY"
echo "=========================================="
echo ""
printf "%-22s %12s %12s\n" "Condition" "Avg Input" "vs Baseline"
printf "%-22s %12s %12s\n" "----------------------" "------------" "------------"

for i in "${!GROUP_NAMES[@]}"; do
  label="${GROUP_NAMES[$i]}"
  total=${GROUP_AVG_TOTAL[$i]}
  overhead=$((total - baseline))

  if [ "$i" -eq 0 ]; then
    printf "%-22s %12d %12s\n" "$label" "$total" "—"
  else
    printf "%-22s %12d %+12d\n" "$label" "$total" "$overhead"
  fi
done

echo ""
echo "--- AgEnD Overhead Breakdown ---"
full_overhead=$((${GROUP_AVG_TOTAL[1]} - baseline))
standard_overhead=$((${GROUP_AVG_TOTAL[2]} - baseline))
minimal_overhead=$((${GROUP_AVG_TOTAL[3]} - baseline))

echo "Full profile (30 tools):     +$full_overhead tokens"
echo "Standard profile (11 tools): +$standard_overhead tokens"
echo "Minimal profile (4 tools):   +$minimal_overhead tokens"

echo ""
echo "--- Cost per First Message (Sonnet 4 input: \$3/MTok) ---"
PRICE=3.00
echo "Full:     \$$(echo "scale=6; $full_overhead * $PRICE / 1000000" | bc) overhead"
echo "Standard: \$$(echo "scale=6; $standard_overhead * $PRICE / 1000000" | bc) overhead"
echo "Minimal:  \$$(echo "scale=6; $minimal_overhead * $PRICE / 1000000" | bc) overhead"

echo ""
echo "--- Context Window Impact (200K window) ---"
echo "Full:     $(echo "scale=2; $full_overhead * 100 / 200000" | bc)% of context"
echo "Standard: $(echo "scale=2; $standard_overhead * 100 / 200000" | bc)% of context"
echo "Minimal:  $(echo "scale=2; $minimal_overhead * 100 / 200000" | bc)% of context"

echo ""
echo "Note: Baseline includes Claude Code's own system prompt (~20K tokens)."
echo "AgEnD overhead is the delta above baseline."
echo ""
echo "Done."
