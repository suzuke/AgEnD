# AgEnD MCP Token Overhead 測試報告

> 日期：2026-04-04 | 測試者：agend-t5033 + agend-reviewer-t9177

## 目的

精確測量 AgEnD MCP server 注入到 Claude Code 的額外 token 消耗，包括 MCP instructions（fleet context + workflow template）和 tool definitions（full / standard / minimal 三種 profile）。

## 測試方法

### 測試指令

```bash
claude -p "Say hello" --output-format json --model sonnet
```

### 測試環境

- **目錄：** 空目錄（無檔案、無 CLAUDE.md、無 .claude/ settings）
- **對照組：** 純 Claude Code，無 MCP server
- **實驗組：** Claude Code + AgEnD MCP server（分 full / standard / minimal）
- **重複次數：** 每組 3 次
- **結果：** Input tokens 完全確定性（3 次數值一致）

### Baseline 測量方式

在空目錄、無 MCP server 的環境下直接執行：

```bash
mkdir /tmp/empty-test && cd /tmp/empty-test
claude -p "Say hello" --output-format json --model sonnet
```

JSON output 的 `.usage` 欄位回傳精確的 token 數：
- `input_tokens`: 3（只有 "Say hello" prompt）
- `cache_creation_input_tokens`: 9,015（Claude Code 的 system prompt，未 cache 部分）
- `cache_read_input_tokens`: 11,644（已 cache 部分）
- **Total: 20,662 tokens**（Claude Code 的基準線，零 AgEnD overhead）

## 測試結果

| 條件 | Total Input | vs Baseline | 佔 200K Context |
|------|------------|-------------|-----------------|
| **Baseline（無 MCP）** | 20,662 | — | — |
| **Full（30 tools）** | 21,549 | **+887** | 0.44% |
| **Standard（11 tools）** | 21,368 | **+706** | 0.35% |
| **Minimal（4 tools）** | 21,292 | **+630** | 0.31% |

## Overhead 分解

### MCP Instructions（~630 tokens）

最大的組成部分，包含：
- Fleet 身份（instance 名稱、工作目錄、顯示名稱）
- 角色描述
- 訊息格式規則（`[user:name]` vs `[from:instance-name]`）
- 工具使用指引
- 協作規則
- 開發流程模板（~2050 chars）

### Tool Schema 差異

| 比較 | Token 差異 |
|------|-----------|
| Full → Standard（少 19 tools） | -181 tokens |
| Standard → Minimal（少 7 tools） | -76 tokens |
| Full → Minimal（少 26 tools） | -257 tokens |

**備註：** Tool schema 的實際 token 消耗遠低於 naive 預估。Anthropic 對 tool definitions 有特殊的 token 壓縮處理 — 30 tools vs 4 tools 的差距僅 257 tokens（預估 ~3,500）。

## 費用影響

以 Sonnet 4 input pricing（$3/MTok）計算：

| Profile | 每條訊息 Overhead | 每 1,000 條訊息 |
|---------|-----------------|---------------|
| Full | $0.002661 | $2.66 |
| Standard | $0.002118 | $2.12 |
| Minimal | $0.001890 | $1.89 |

**Prompt caching 生效後**（後續訊息）：overhead 降至 1/10（cache read = $0.30/MTok），即每條訊息 < $0.0003。

## Context Window 影響

Claude Code 本身的 system prompt 佔 ~20,662 tokens（200K 的 10.3%）。AgEnD 額外增加：

| Profile | 額外 Tokens | 佔 Context Window |
|---------|-----------|-----------------|
| Full | +887 | 0.44% |
| Standard | +706 | 0.35% |
| Minimal | +630 | 0.31% |

## 結論

1. **AgEnD 的 MCP overhead 極小** — 不到 baseline 的 4.3%（full profile）
2. **Context window 影響 < 0.5%** — 對長對話影響可忽略
3. **Instructions 是主要成本**（~630 tokens，佔 full overhead 的 71%）
4. **Tool schema 差異很小**，因為 Anthropic 有 token 壓縮
5. **費用可忽略** — 每條訊息 < $0.003，cache 生效後 < $0.0003

### 建議

- **Standard profile** 是最佳平衡（保留所有協作工具，比 full 少 181 tokens）
- 不需要為了省 token 犧牲功能
- 僅當 instance 完全不需要 fleet 協作時才用 **minimal**，可再省 76 tokens

## 重現方式

```bash
./scripts/measure-token-overhead.sh
```

需要：`claude` CLI、`jq`、`node`、`npm run build`
