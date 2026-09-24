# Backend 實測行為（2026-09-24）

> **TL;DR**
> - codex、opencode 的排隊與中斷都是結構化 API；claude 的排隊走 Stop hook，中斷是 PTY 送 `Esc` + channel 訊息（專案 CLAUDE.md 須說明訊息來源）。只有 codex 能「插入不中斷」。
> - 記住：**這些結論綁定下表的版本**；backend 升版就要重測並更新本頁。
> - 下一步：寫 driver 前讀對應的分頁與「陷阱」。

實測日期：2026-09-24。來源：spike-codex.md、spike-claude.md、spike-claude-f.md、spike-opencode.md。

## 總表

| | codex 0.156.1 | opencode 1.18.31 | claude 2.1.281 |
|---|---|---|---|
| 協定 | app-server：JSON-RPC over WebSocket over unix socket | `opencode serve`：HTTP + SSE | 互動式 TUI + hooks + MCP channel |
| 重連 | `thread/resume` + `thread/turns/list` 補回 | SSE 無 replay，以 REST 補狀態 | hooks 重新觸發 |
| 狀態來源 | app-server 事件 | SSE + REST 核對 | hooks |
| 排隊 | `thread/queue/add`（自動出列） | `prompt_async`（server 原生 FIFO） | Stop hook `decision: block` |
| 插入 | `turn/steer` | 不支援 → 中斷 | 不支援 → 中斷 |
| 中斷 | `turn/interrupt` | `POST /session/:id/abort` | `Esc` 後立即經 channel 送（需 CLAUDE.md 來源說明） |
| resume | 明確 thread id | 明確 session id | `--resume <id>` |
| 授權 | `item/commandExecution/requestApproval`（6 種回覆） | 以 `GET /permission` 輪詢為準 | allowlist；拒絕情況未釐清 |
| 啟動提示 | trust（預寫 config 可跳過） | 無 | trust、MCP trust、dev-channels |

## 陷阱（跨 backend 必讀）

| # | 陷阱 | 對策 |
|---|---|---|
| 1 | codex `--listen unix://<長路徑>` 實際 socket 在短路徑，長路徑只是 symlink；直接連會失敗 | 連線前 `realpath`（`agend_daemon::driver::codex::socket_connect_path`） |
| 2 | codex 預設 sandbox 內連不到 workspace 外的 unix socket | daemon socket 放進 workspace 範圍，或由 holder 自動核准 |
| 3 | claude 忙碌時經 channel 送的訊息會被擱置不處理 | 忙碌時改走 Stop hook 排隊 |
| 4 | claude `Esc` 中斷後，沒有來源說明的 channel 訊息 0/3 被處理 | 專案 CLAUDE.md 說明來源 → 3/3 |
| 5 | claude `Esc` 不會觸發 Stop hook | 中斷後不要等 Stop，立即送 |
| 6 | claude 新目錄的 trust 提示預設游標在「No, exit」；盲按 Enter 會退出 | 不盲按；規則檔明確選項 |
| 7 | opencode `permission.asked` SSE 事件兩次試驗漏發一次 | 以 `GET /permission` 輪詢為準 |
| 8 | 在 PTY 用 tmux 一次送「文字 + Enter」會吞掉 Enter（codex、claude 都遇到） | v2 不在 PTY 打字；只送單一控制鍵 |
| 9 | AF_UNIX 路徑上限約 104 bytes（macOS） | 所有 socket 路徑都要算長度或解析 symlink |

## 分頁

| backend | 細節 |
|---|---|
| codex | [backends/codex.md](backends/codex.md) |
| claude | [backends/claude-code.md](backends/claude-code.md) |
| opencode | [backends/opencode.md](backends/opencode.md) |

## 第 0 階段的 8 個問題

| # | 問題 | 結論 |
|---|---|---|
| 1 | 附屬程序存活時 daemon 能否重連並補回事件 | codex、opencode 可以（以獨立程序測，非 holder 內） |
| 2 | codex 是否通知 TUI 手動發起的 turn | 可以，但要先 `thread/resume` |
| 3 | claude `Esc` 後 channel 訊息是否立即處理 | 有 CLAUDE.md 來源說明 3/3；程式化 send-now 只在 headless 驗證 |
| 4 | opencode 插入與中斷 | 無插入；abort 可用 |
| 5 | 以明確 id resume | 三個都可以 |
| 6 | codex sandbox 內 CLI 能否連 unix socket | 預設被擋；需 approval 或把 socket 放進 workspace |
| 7 | claude 以 allowlist 免除 `agend` 權限提示 | allow 規則有效；反例與 `agend` 本身未直接驗證 |
| 8 | 啟動提示能否全部避免；授權是否有結構化管道 | codex 可預寫 trust；opencode 無提示；claude 預寫設定 BLOCKED。授權：codex、opencode 可用，claude 未驗證 |

## 仍未驗證

- [ ] claude：在隔離的 `CLAUDE_CONFIG_DIR` 預寫設定以跳過 trust／dev-channels 提示（需要重新登入，BLOCKED）。
- [ ] claude：不在 allowlist 的指令會不會跳提示（本機全域設定干擾，未能確認）；`PermissionRequest` hook 能否程式化回答（未觸發過）；以 allowlist 免除 `agend` 本身的提示（未直接測）。
- [ ] claude：`--input-format stream-json` 的 `control_request interrupt` 只在 `-p` headless 模式驗證過。
- [ ] claude：規劃 §4.1 提到的 channel bridge「SSE 帶 last_event_id」重連，spike 未測。
- [ ] codex：`item/fileChange/requestApproval`、`item/permissions/requestApproval` 未觸發過。
- [ ] opencode：`permission.asked` 漏發的原因未查明；V2 permission API（`permission.v2.asked`）未測。
- [ ] 以上都不是在 holder 內跑的；holder 持有附屬程序時的重連要在第 6 關驗證。

## 下一步

```bash
cat docs/backends/codex.md
```
