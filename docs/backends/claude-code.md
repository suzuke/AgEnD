# claude 2.1.281

> **TL;DR**
> - 互動式 TUI + hooks（狀態）+ MCP channel（送訊息）；忙碌時用 Stop hook 排隊。
> - 記住：**`Esc` 中斷後的 channel 訊息，要有來源說明才會被處理**（無說明 0/3，有說明 3/3）。
> - 下一步：driver 在 `crates/agend-daemon/src/driver/claude.rs`（第 12 施工關）；規則見 D16。

來源：spike-claude.md（C1–C7）、spike-claude-f.md（F1–F5），2026-09-24，Claude Code 2.1.281。

## 結論表

| 項目 | 結果 | 說明 |
|---|---|---|
| C1 閒置時 channel | 可行 | 約 1–2 秒開新 turn；`UserPromptSubmit` 的 prompt 是 `<channel source=... delivery_id=... chat_id=... sender_id=...>` 包裝 |
| C1 忙碌時 channel | 部分 | 傳輸立即可靠，但訊息可能在下個 turn 被忽略（2+2 的問題一直沒被回答） |
| C2／F 中斷後送 | 有條件可行 | `Esc` 約 1 秒中斷；無來源說明 0/3、CLAUDE.md 說明 3/3、訊息標頭單獨 3/3、兩者並用 3/3 |
| F3 Esc 後等 Stop | 不可行 | `Esc` 後從不觸發 Stop；立即送與等 10 秒成功率相同（3/3） |
| F4 Stop hook 排隊 | 可行，3/3 | 先清空佇列再回 `{"decision":"block","reason":"<訊息>"}`；每次恰好 2 個 Stop（`stop_hook_active` false → true），無迴圈 |
| F5 生產模型 | 2/2 | `claude-opus-5` 用同樣做法成功 |
| C3 程式化 send-now | 僅 headless | `-p --input-format stream-json` 送 `control_request {subtype: "interrupt"}` 可中斷並自動開新 turn；互動模式未驗證 |
| C4 hooks 當狀態來源 | 可行 | 實際觸發：SessionStart、UserPromptSubmit、PreToolUse、PostToolUse、Stop |
| C5 以 id resume | 可行 | `--resume <session-id>` 與 cwd 無關（v1 用的 `--continue` 綁 cwd） |
| C6 權限 allowlist | 部分 | `permissions.allow: ["Bash(echo *)"]` 在 default 模式下免提示；反例未能確認 |
| C7 啟動提示 | 已列舉 | 見下表；隔離 `CLAUDE_CONFIG_DIR` 預寫設定 BLOCKED（要重新登入） |

## 啟動提示

| 提示 | 何時出現 | 預設游標 |
|---|---|---|
| workspace trust | 新目錄 | 「No, exit」（與 v1 註解相反） |
| trust 的變體：預先核准的權限警告 | 目錄有 `.claude/settings.json` 的 `permissions.allow` | 同上 |
| MCP server trust | `.mcp.json` 有 server，且沒帶 dev-channels 旗標 | 「Continue without using this MCP server」 |
| dev-channels 警告 | 帶 `--dangerously-load-development-channels` | 選項 1（繼續） |
| 首次使用精靈（主題、登入方式） | 全新 `CLAUDE_CONFIG_DIR` | — |

## 陷阱

- [ ] 「Stop hook error: …」只是 Claude Code 對 hook 觸發續行的通用標籤，不是錯誤。
- [ ] 每次 Stop 可能跑兩個 hook（本機還有外掛 hook）。
- [ ] 權限模式會依專案「黏著」：沒帶旗標重開會沿用上次的 `bypassPermissions`。
- [ ] 顯式 `--mcp-config <file>` 時 `server:<name>` 找不到；改用專案根目錄 `.mcp.json` 自動發現 + `--dangerously-load-development-channels server:<name>`。
- [ ] 在 claude 接管 tty 之前送進 PTY 的按鍵會出現在它的輸入框（typeahead）。
- [ ] `Notification`、`PermissionRequest`、`PreCompact`、`SessionEnd` hook 從未觀察到觸發。
- [ ] 2.1.282 錄製（[RECORDER.md](../../crates/agend-testkit/RECORDER.md)）：`PermissionRequest`（授權對話框時）與 `SessionEnd`（`/exit`）有觸發。
- [ ] 2.1.282 錄製：忙碌時送到的 channel 訊息會排隊，這輪（含 Stop hook 續行）結束後才觸發 `UserPromptSubmit` 並照做，和 C1 不同；D16 仍用 Stop hook 排隊（只有一次錄製）。
- [ ] 2.1.282：帶 `--dangerously-load-development-channels` 時仍會出現「New MCP server found in this project」對話框（預設游標「Continue without using this MCP server」）。
- [ ] `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` 會讓 channel 被忽略（「Channels are not currently available」）。
- [ ] `--setting-sources project,local`：不載入使用者自己的設定、hook 與 allow 規則。

## 下一步

```bash
cat docs/backends/opencode.md
```
