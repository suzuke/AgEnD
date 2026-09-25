# codex 0.156.1

> **TL;DR**
> - `codex app-server`：JSON-RPC over WebSocket over unix socket；三種忙碌等級都有原生方法。
> - 記住：**連線前先 `realpath` socket 路徑**；重連後要 `thread/resume` 才拿得到完整事件。
> - 下一步：driver 在 `crates/agend-daemon/src/driver/codex.rs`（第 7 施工關）。

來源：spike-codex.md（2026-09-24，`codex-cli 0.156.1`）。協定清單由 `codex app-server generate-json-schema --experimental` 產生：164 個 client→server 方法、11 個 server→client 請求、82 種通知。

## 結論表

| 項目 | 結果 | 做法 |
|---|---|---|
| S1 重連 | 可行 | 無 client 時 turn 照樣完成並持久化；新連線 `thread/resume {threadId, excludeTurns:true}`，再 `thread/turns/list` 補回 |
| S2 TUI 手動發起的 turn | 可行，但要先 resume | 只連線不 resume 只收到粗粒度 `thread/status/changed`；resume 後收到完整 `turn/*`、`item/*` |
| S3 插入 | 可行 | `turn/steer {threadId, expectedTurnId, input}`；同一 turn 的最終回答會採用 |
| S3 排隊 | 可行 | `thread/queue/add {threadId, clientUserMessageId, input}`；turn 結束自動開新 turn |
| S3 中斷 | 可行 | `turn/interrupt {threadId, turnId}` → `turn/completed` status `interrupted`；之後可立即 `turn/start` |
| S4 以 id resume | 可行 | 砍掉 TUI 與 app-server、同 `CODEX_HOME` 重起，`codex resume <id>` 保有完整上下文 |
| S5 sandbox + unix socket | 預設被擋 | `workspace-write` sandbox 內 connect 到 workspace 外 socket 得 `Operation not permitted`；經 approval 在 sandbox 外重跑成功 |
| S6 啟動提示 | 可避免 | 只有「Trust this folder?」；在 `CODEX_HOME/config.toml` 預寫 `[projects."<abs-path>"] trust_level = "trusted"` 就不出現 |
| S7 結構化授權 | 可行 | server→client 請求 `item/commandExecution/requestApproval`，回 `{"decision": ...}` |

授權回覆的 6 種 decision：`accept`、`acceptForSession`、`acceptWithExecpolicyAmendment`、`applyNetworkPolicyAmendment`、`decline`、`cancel`。

## 陷阱

- [ ] `--listen unix://<path>` 路徑過長時，真正 socket 在 `/private/tmp/codex-daemon-<uid>/<sha256>`，`<path>` 只是 symlink。任何 client（含 codex 自己的 `--remote`）都要連解析後的路徑。
- [ ] 不要在 `thread/queue/add` 之後呼叫 `thread/queue/start`：會和自動出列競爭，回 `-32600 thread already has an active or pending turn`。
- [ ] 忙碌時再送一個普通 `turn/start` 不會報錯，而是併進進行中的 turn（等同 steer）。
- [ ] 每個不同的 `--listen` 路徑各有一個 app-server 程序。
- [ ] `model_reasoning_effort = "minimal"` 會被模型拒絕（HTTP 400）；spike 改用 `low`。這是設定問題，不是協定問題。
- [ ] 2026-09-25 錄製（[RECORDER.md](../../crates/agend-testkit/RECORDER.md)）：`--listen` 路徑再短也綁在 `/private/tmp/codex-daemon-<uid>/<sha256>`，要求的路徑是 symlink。
- [ ] `-c mcp_servers={}` 關不掉使用者 `config.toml` 與 plugin 的 MCP server（`-c` 合併進設定表，不是取代；錄製時仍啟動）；它們的 `mcpServer/startupStatus/updated` 通知依機器而定。關掉的方法：`--disable plugins` 加上每個 server 一個 `-c mcp_servers.<名稱>.enabled=false`（`codex mcp list --json` 驗證，不花 token）；只對 config.toml 裡的 server 有效，對 plugin 提供的會報 `invalid transport`。
- [ ] `turn/steer` 在同一輪裡另成一則 user message 與一則回覆；`thread/resume` 沒帶 `excludeTurns: true` 時先送 `deprecationNotice`。

## 對 v2 的含意

1. socket 路徑長度是硬限制：路徑全程保持在 AF_UNIX 上限內，或連線前一律解析。
2. 忙碌狀態追蹤不精確也不會損壞狀態（最壞是變成 steer）；真的要「下一個 turn」就用 `thread/queue/add`。
3. 重連的 driver 一定要明確 `thread/resume`。
4. 斷線期間的 turn 結束不會遺失，可用 `thread/turns/list`／`thread/items/list` 補回。
5. 授權需要真的 handler：除非 `danger-full-access`，連 daemon 自己的 socket 都可能觸發 approval。v1 的做法是 managed server 用 `approval_policy="never"` + `sandbox_mode="danger-full-access"`。

## 下一步

```bash
cat docs/backends/claude-code.md
```
