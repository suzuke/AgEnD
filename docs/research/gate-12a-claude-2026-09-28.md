# 第 12A Claude 歷史實測：2026-09-28

> **TL;DR**
> - 原提案 F1–F10：Claude Code 2.1.283，5 個短回合、8 次不送 prompt 的啟動。
> - 這是當時的實測紀錄，不是 2026-10-03 新實測或新版認證。
> - 下一步：由[第 12A 提案](../gates/gate-12-adapters.md)逐項討論；新真 CLI 測試另需授權。

來源：[原 #138 固定 head](https://github.com/suzuke/AgEnD/blob/05b0621e6dbe5ca1e997fc9f6171dd937ac39f2c/docs/gates/gate-12-adapters.md)。下段保留原實測描述與結果，文中的「這台」「你的」均指 2026-09-28 的環境。

在 `record-sandbox.sh` 裡用私有 tmux 跑真的 claude `2.1.283`（錄製檔是 `2.1.282`），模型 haiku（P3 的 auto 另用 sonnet 起一次），工作目錄 `/private/tmp/agend-rec-g12-*`，環境用 `env -i` 只給 `HOME`、`USER`、`TERM`、`LANG`、`TMPDIR`、`PATH`（假 shim 目錄在最前面）。hooks 用 `--settings <dir>/settings.json` 註冊 `SessionStart`、`UserPromptSubmit`、`PreToolUse`、`PostToolUse`、`PermissionRequest`、`Stop`、`StopFailure`、`SessionEnd`、`Notification`，每個都把 payload 附加到一個檔。共 5 個短回合、8 次不送 prompt 的啟動；跑完沒有殘留的 tmux、claude 或假 MCP server 行程。

| # | 查什麼 | 做法 | 結果 |
|---|---|---|---|
| F1 | 新目錄的啟動對話框 | 新目錄、有 `.mcp.json`、帶 `--dangerously-load-development-channels server:agend` | 依序三個：信任資料夾（游標預設在「No, exit」；按 `2` 沒反應，要 `Down`＋`Enter`）→「New MCP server found in this project: agend」（預設「Continue without using this MCP server」）→ development channels 警告（預設「1. I am using this for local development」，`Enter` 即可） |
| F2 | `--settings` 裡的 `enabledMcpjsonServers: ["agend"]` 能不能跳過 MCP 對話框；加 `--setting-sources project,local` 時 `--settings` 的 hooks 還會不會跑 | 另一個新目錄，兩個都加 | MCP 對話框沒出現（信任與 development channels 仍有）；`SessionStart` hook 有觸發 |
| F3 | development channels 警告是不是只跳一次 | 同一個目錄（信任已接受）再起一次 | **每次啟動都跳**，要按 `Enter` |
| F4 | bypass 警告 | `--permission-mode bypassPermissions` | 這台機器上沒有出現（你的 `~/.claude/settings.json` 已有 `skipDangerousModePermissionPrompt: true`）；畫面顯示 `bypass permissions on` |
| F5 | `--permission-mode auto` | 不送 prompt，haiku 與 sonnet 各起一次 | sonnet：`auto mode on`；haiku：`manual mode on`、右下角「auto mode unavailable for this model」，**不報錯、直接退回 manual** |
| F6 | shell 找不找得到 shim | 不設 `ZDOTDIR`，讓 claude 跑 `command -v git pkill killall; echo PATH=$PATH`，再跑 `type pkill` | `git`、`killall` 是假 shim；PATH 原樣、假 shim 在第一個。`pkill` 是 claude 自己的 shell function（在 `~/.claude/shell-snapshots/…` 裡，擋掉會殺到 claude 自己的 pattern），最後呼叫 `command pkill`，也就是 PATH 上的 shim。Bash 工具跑的是 `/bin/zsh -c source <snapshot>`，snapshot 最後一行 `export PATH=<claude 啟動時的 PATH>` |
| F7 | 工具跑到一半按 `Esc` 會觸發哪些 hook | 讓它在前景跑 `python3 -c "import time; time.sleep(25)"`，`PreToolUse` 出現後按 `Esc`，等 30 秒 | 只有 `UserPromptSubmit`、`PreToolUse`；**之後沒有 `PostToolUse`、沒有 `Stop`**，畫面「Interrupted · What should Claude do instead?」 |
| F8 | 背景工具 | 叫它跑 `sleep 25` | claude 自己改成 `run_in_background: true`；`PreToolUse`、`PostToolUse`、`Stop` 立刻出現；25 秒後背景結束時多一輪：`UserPromptSubmit`（prompt 是 `<task-notification>…`）與 `Stop`。前景的 `sleep 25` 被 claude 自己擋掉（「Blocked: standalone sleep 25」），連 `PreToolUse` 都沒有 |
| F9 | `--session-id` 起來、還沒打任何字就被結束，再 `--resume` | 起來等到 `SessionStart`，`tmux kill-server`（SIGHUP），再 `--resume <同一個 id>` | 沒有 transcript 檔；`--resume` 的 stderr：`No conversation found with session ID: <id>`，exit 1。對照：打過 `/exit` 再結束的，transcript 在、`--resume` 接得上（`SessionStart` 的 `source` 是 `resume`） |
| F10 | 從 Claude Code 裡面起的 claude | 第一次沒清環境 | 畫面：「Transcript saving is off — inherited CLAUDE_CODE_CHILD_SESSION marker」。daemon 若把這個變數傳給 agent，session 不會存檔、resume 全壞；第 6 施工關的環境白名單本來就不傳它（`crates/agend-daemon/src/runtime/env.rs` 的 `PASS_THROUGH` 只有 `HOME`、`USER`、`LOGNAME`、`LANG`、`LC_ALL`、`LC_CTYPE`、`TMPDIR`、`TZ`） |


## 下一步

查核選定版本及設定時重驗相關 F／U 項；不沿用原真 CLI 授權。
