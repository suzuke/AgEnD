# 第 12A 提案：P6–P9 送達與清掃

> **TL;DR**
> - 三級忙碌、送達確認、PATH 與孤兒清掃。
> - 都是待確認的 Claude 接入方案；現有 Codex 行為維持已驗範圍。
> - 下一步：依序確認 P6–P9。

## P6：claude 的三級忙碌與忙／閒

- 問題：queue、steer、interrupt 怎麼落到 claude？daemon 怎麼知道它忙不忙？ctrl+enter 要不要用？
- 建議：照 D16 原文。忙：`UserPromptSubmit`。閒：`Stop`（沒被 block）、`SessionStart`；daemon 自己送 `Esc` 之後。閒置：經 channel 送。`Queue`（忙）：放 daemon 的佇列，下一個 Stop（`stop_hook_active: false`）全部取出、合成一個 reason 回 `block`。`Steer` → core 改成 `Interrupt`。`Interrupt`：holder 送單一 `Esc` → 立刻經 channel 送。`PreToolUse`、`PostToolUse` 只記成事件，不改忙閒。ctrl+enter 不用：官方文件說它送的是「你在 TUI 打字排隊的訊息」（[interactive mode](https://code.claude.com/docs/en/interactive-mode)，`chat:sendNow`），channel 的訊息算不算沒寫（U2）；替代的 `Ctrl+X Ctrl+S` 是兩個鍵。
- 理由：D16 有 spike 證據（忙碌時經 channel 送，模型可能不做，spike C1；Stop hook 3/3）；不加新規則。
- 替代方案：忙碌時也經 channel 送（推翻 D16）；用 ctrl+enter 強送（U2 查證後才能提）。
- 例子：agent 在忙，`agend send --level queue g12-c "m-q"` → `queued`；這輪結束時 Stop hook 回 `block` → 下一個 Stop 帶 `stop_hook_active: true` → `m-q confirmed`。F8：背景工具結束時多一輪（`UserPromptSubmit` 的 prompt 是 `<task-notification>`），照同一套規則是忙→閒，不會誤判。
- 關係：D16 照做。你在 TUI 自己按 `Esc` 的限制見「A 段不做」。
- [ ] 使用者確認

## P7：claude 的送達確認與事件

- 問題：`sent`、`confirmed` 各在什麼時候成立？daemon 不在時的事件（DRV-6）從哪補？
- 建議：`sent`＝bridge 寫進 claude，或 Stop hook 的 block 已回出去。`confirmed`＝channel 送的看 `UserPromptSubmit` 的 prompt 有 `delivery_id="<訊息 id>"`（錄製檔 `one_turn`）；Stop hook 送的看下一個 `stop_hook_active: true` 的 Stop（錄製檔 `busy`）。冪等照第 7 施工關 P5，當掉後不重送。事件：新表 `driver_events`（hook 事件照到達順序存，`seq` 當 cursor），保留 14 天（同事件，D31）；migration 取開工時的下一個空號；本次 baseline 已有 `0005_pipeline`、`0006_codex_input_threads`，下一個可用號是 `0007`，尚未建立或核准。
- 理由：hook 是 claude 唯一的結構化事件；第 7 施工關的 `messages` 表與 DRV 契約照用，只多一張存 hook 的表。
- 替代方案：讀 claude 的 transcript 檔當事件日誌（綁它的檔案格式）；開機重送（可能重複一輪，違反 DRV-9）。
- 例子：daemon 停著時 Stop 回 `{}`，hook 事件存 spool；開機補事件與狀態後，仍未送出的 queue 要等符合 P6 的送達時機。不能把補事件本身當成 `confirmed`。
- 關係：第 7 施工關 P5、P7；D31。
- [ ] 使用者確認

## P8：claude 不需要 `ZDOTDIR`

- 問題：第 7 施工關為 codex 加了 `ZDOTDIR`，因為 macOS login zsh 會把系統路徑排到 shim 前面。claude 要不要也加？
- 建議：**不加**。2026-09-28／2.1.283 的 F6：claude 的 Bash 工具是 `/bin/zsh -c source <snapshot>`（不是 login shell），snapshot 最後把 PATH 設回 claude 啟動時的樣子，shim 在第一個；`pkill` 是 claude 的 shell function，最後也呼叫 PATH 上的 shim。`claude_live` 每次都檢查 `command -v git pkill killall`，哪天 claude 改了就會看到。
- 理由：實測不需要；不改第 6 施工關的環境白名單。
- 替代方案：先加再說（多一個變數、要改第 6 施工關 H3）。
- 例子：F6 的輸出：`/private/tmp/agend-rec-g12-e2/bin/git`、`/private/tmp/agend-rec-g12-e2/bin/killall`、`pkill is a shell function …`。
- 關係：跟第 7 施工關 K8「`ZDOTDIR` 只給 codex」一致。
- [ ] 使用者確認

## P9：holder 死掉後清掃 claude

- 問題：第 7 施工關的清掃只認得 codex 的標記；holder 被 `kill -9` 後 claude 或它的 MCP server、背景工具可能留著。要不要排在 A 段？
- 建議：排在 A 段，完全照第 7 施工關 P2 的條件與時機，只加 claude 的標記：argv 裡 `--session-id`／`--resume` 後面那個元素，或 `agend channel --instance <id>` 的 `<id>`，都要完全相等。
- 理由：同一個機制，只換標記。
- 替代方案：另開一個施工關（claude 死掉可能留孤兒）。
- 例子：`kill -9` holder → log `sweep of agent group 5231 (holder died): SIGKILL sent (claude --resume 3f2a…)` 或 `already gone`。
- 關係：第 7 施工關 P2 與「已知風險」（當時問排在哪）。
- [ ] 使用者確認

## 送達仍須補證據

P7 的 Stop 續行確認要能對應本次取出的訊息 id，普通 Stop、其他 prompt 或背景 task 的 Stop 都不能代確認。斷線發生在「持久化／stdout 寫出 block／收到下一個 Stop」的各個位置時，須證明不漏、不重與未確認狀態；詳細時序要在 P1、P7 說明及實作提案確認前寫定，不由本次文件整理默認一種做法。

## 既有整合要保持

- P6 的 daemon 單鍵中斷與第 11C 的人工 PTY owner 同時存在，控制權與拒絕行為須有回歸；不能照搬 Codex clientId 規則來宣稱 Claude 訊息已確認。
- `pipeline_runtime.rs` 目前組裝的是 `CodexDriver`；Claude 接入後要驗 task dispatch／review 的 Driver 路徑，不能只測單獨 `send`。
- P9 沿用 codex sweep 的 pgid／身分條件與時機，只加 Claude 的 exact argv marker；已有其他 backend 程序或 pid 重用時要拒絕誤殺。

## 下一步

回到[施工關入口](gate-12-adapters.md)，依序解釋 P6–P9，等使用者決定。
