# 第 12 施工關：其餘 adapter（`adapters`）

> **TL;DR**
> - 分四段：A claude、B opencode、C forge github、D Telegram。這頁現在只有 **A 段（claude）開工前提案** P1–P10；B 等 A 確認後寫，C、D 等第 10 施工關 merge 後寫。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這一段才算完成。
> - 下一步：逐題確認 P1–P10（P3、P4 是安全決定，沒有預設答案）；確認後 merge 這份提案，等第 9–11 施工關完成再開工 A 段。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**提案中**（2026-09-28）：A 段開工前提案 P1–P10 寫定，等你確認。前一版把四段一起提案（34 題、3 輪 review 沒收斂），使用者 2026-09-28 決定改成比照第 11 施工關分段。依賴：第 7 施工關（送達、`messages` 表、清掃）已驗收；第 9 施工關（CLI、`agend send`、協定 1.2）、第 10 施工關（協定 1.3、`fake-worker`）提案已確認、還沒實作。它們做完時跟這裡的理解不同，改這頁，不改它們。

## 範圍

**A 段 claude driver（D16）——本頁的提案**：channel bridge 與 hooks、啟動設定、權限模式、agent 手上的 gh、啟動對話框、三級忙碌、送達與事件、PATH、清掃、測試（P1–P10）。

**B、C、D 段（還沒寫提案，只列範圍）**：

| 段 | 範圍 | 什麼時候寫 |
|---|---|---|
| B opencode | `opencode serve` 放進 holder、密碼、session、送達與事件、權限設定；「你親自驗收」的三個真 backend 互傳訊息 | A 段提案確認後 |
| C forge github | `gh`、push 與 head 以哪邊為準、merge、main 前進、PR 收尾、GitHub CI 與 checks 裡的 gh 登入 | 第 10 施工關 merge 後（依賴它的 merge 狀態機、`MainAdvanced`、「需要你」來源表） |
| D Telegram | `config.toml`、allowlist、token、HTTP client、core 的 Notifier、發什麼、收什麼、可靠性、G4 已讀狀態 | 第 10 施工關 merge 後（依賴「需要你」的來源與核准流程） |

寫 B、C、D 時的參考：前一版四段一起的提案在 commit `74ced40`（`git show 74ced40:docs/gates/gate-12-adapters.md`）；三輪 review 在協調者的 scratchpad：`g12-review.md`、`g12-review-r2.md`、`g12-review-r3.md`。

### 從其他施工關帶來、A 段要處理的筆記

- **claude Bash 工具的 PATH 重排**（[gate-03-shim 已知限制](gate-03-shim.md#已知限制)、第 7 施工關 P4、K8）→ 已實測，P8。
- **使用者提供：claude 按 ctrl+enter 可以把訊息「強送」給正在工作的 agent**（2026-09-26）→ P6。
- **holder 被 `kill -9` 後 claude 的清掃**（第 7 施工關「已知風險」）→ P9。
- **第 6 施工關 H1 的已知限制**（「`Spawn` 被確認」不等於 claude 已存 session）→ 已實測，列在「A 段不做」。
- **第 10 施工關的 `fake-worker`**（backend 登記成 `claude`、`delivery = inbox`）→ P2。

## 2026-09-28 實測（真 claude，使用者授權）

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
| F10 | 從 Claude Code 裡面起的 claude | 第一次沒清環境 | 畫面：「Transcript saving is off — inherited CLAUDE_CODE_CHILD_SESSION marker」。daemon 若把這個變數傳給 agent，session 不會存檔、resume 全壞；第 6 施工關的環境白名單本來就不傳它 |

## 開工前提案（A 段）

每項：問題 · 建議 · 理由 · 替代方案 · 例子 · 跟既有決定的關係。每題只有一個決定。跟既有決定不同的地方標「**與已確認／已追認的 X 不同，請明確決定**」。安全決定（P3、P4）只列選項與建議，請你選。

A 段的硬規則（使用者 2026-09-28）：**不加新的 core 事件，不加新的偵測或判斷規則**。現有機制（D16、第 6、7、8、10 施工關已確認的做法）做不到的，列進「A 段不做」，寫明你會看到什麼限制。

文件已定、這裡不重問的：claude 用互動 TUI＋hooks＋channel；狀態看 hooks；忙碌時用 Stop hook 排隊（先取出佇列、`stop_hook_active` 防迴圈）；閒置經 channel 送；中斷＝`Esc` 後立刻經 channel 送、不等 Stop；專案 CLAUDE.md 說明訊息來源（D16）。送達四個狀態、只有一套冪等、推送帶完整內容（[delivery](../architecture/delivery.md)、第 7 施工關 P5）。「不支援插入就改中斷」（`policy::busy::effective_level`）。holder 死掉 5 秒後重起、3 次後 `failed`、絕不自動全新啟動，claude 第一次用 `--session-id`、之後 `--resume`（第 6 施工關 P6、H1、H2）。agent 環境白名單（第 6 施工關 H3）。清掃的條件（第 7 施工關 P2）。shim 的威脅模型：防手滑、不防故意（[第 3 施工關](gate-03-shim.md)）。

### P1：claude 的訊息與 hook 怎麼到 daemon

- 問題：claude 的訊息從 channel（claude 自己起的 MCP server）進去，狀態從 hooks（claude 跑的指令）出來。這兩個程式是什麼？daemon 不在時怎麼辦？
- 建議：兩個都是 `agend` 的內部子命令：`agend channel --instance <id>`（MCP stdio server）與 `agend hook <事件名>`。都走 client 協定 **1.4** 的新請求（`channel_attach`、`channel_message`、`channel_written`、`hook_event`；1.2 是第 9 關、1.3 是第 10 關）。daemon 不在時 hook 寫到 `$AGEND_HOME/spool/hooks/<instance>/<序號>.json`、Stop 回 `{}`（不 block），daemon 開機照序號補送（`ingest` 模組已寫的做法）。hook 的 timeout 設 10 秒（官方預設 600 秒，[hooks](https://code.claude.com/docs/en/hooks)）。
- 理由：同一個 binary、同一套有版本的協定（ARCHITECTURE 程序模型第 4 條）；hooks 丟掉就一直被當成 busy（V1-LESSONS #9）。
- 替代方案：bridge 另開私有協定（多一套要管版本）；hook 打 HTTP（daemon 多開 server）；daemon 不在時丟掉。
- 例子：`agend send g12-c "hi"` → bridge 寫 `notifications/claude/channel {content:"From: …\n\nhi", meta:{delivery_id:"m-7"}}` → `sent`。
- 關係：協定型別在 core（D11），所以**這是 A 段唯一的 core 改動**（只加請求，不是 pipeline 事件；D26 只加不改）。照 D22 先改 core，並「重過第 1 施工關」：`cargo test -p agend-core`、`cargo xtask check-deps`、`cargo xtask accept core` 全過，你再跑一次第 1 施工關的「你親自驗收」。
- [ ] 使用者確認

### P2：claude 讀哪些設定、我們的檔放哪

- 問題：hooks、`.mcp.json`、CLAUDE.md 寫在哪？要不要讀你自己 `~/.claude/settings.json` 裡的 hooks 與 `defaultMode: "auto"`？第 10 施工關登記成 `claude` 的假 agent 要不要也套？
- 建議：
  - 啟動帶 `--setting-sources project,local --settings $AGEND_HOME/claude/<id>/settings.json`：不讀你的使用者設定（你的 hooks、`defaultMode` 不會跑到 agent 身上），我們的 hooks 與 `enabledMcpjsonServers: ["agend"]` 在 `--settings` 裡（F2 證實兩者一起用時 hooks 會跑、MCP 對話框不出現）。
  - `.mcp.json` 與 CLAUDE.md 一定要在工作目錄，寫進 workspace；DB 記它們的 sha256，檔案存在但雜湊不是我們記的 → 不覆蓋、instance `failed`（原因寫出是哪個檔）。
  - 只套在 `delivery = push` 的 claude；`delivery = inbox`（`fake-worker`）不寫檔、不加旗標、不建 driver。
- 理由：agent 的行為不隨你改自己的設定而變；錄製器也是這樣跑（RECORDER.md）。
- 替代方案：讀你的使用者設定（你的 hooks 每一輪都會在每個 agent 上跑；`defaultMode` 會變成 agent 的權限模式）；隔離的 `CLAUDE_CONFIG_DIR`（要重新登入）。
- 例子：`--dir ~/proj` 而 `~/proj/CLAUDE.md` 是你的 → `g12-c failed: CLAUDE.md exists and was not written by agend`。
- 關係：D16（CLAUDE.md 說明來源）照做；第 10 施工關的 `delivery = inbox` 不受影響。
- [ ] 使用者確認

### P3：claude 的權限模式（安全決定，請選一個）

- 問題：claude 沒有人在旁邊按「允許」。用哪個權限模式？
- 官方摘要（[permission modes](https://code.claude.com/docs/en/permission-modes)）：bypass「只該在隔離環境用」、「對 prompt injection 沒有保護」；deny 規則在 bypass 下仍然有效；auto 用另一個模型審查每個動作，預設擋「沒人核准的 PR merge」，但「不保證安全」。
- 風險：agent 在你的 uid、你的網路下跑；進來的文字（其他 agent 的 `agend send`、它讀到的檔案與網頁）都可能帶 prompt injection。
- 選項：
  - A. bypass：什麼都不擋，跟 codex 的 full-access 一樣。
  - B. bypass ＋ deny 規則：`--settings` 裡擋 `Bash(gh pr merge*)`、`Bash(gh api*merge*)`、`Bash(gh pr review*--approve*)`，以及讀 `$AGEND_HOME/secrets`、`$AGEND_HOME/run`（daemon 寫成展開後的絕對路徑 `Read(//<絕對路徑>/**)`）。字串比對，擋手滑、不擋故意。
  - C. auto：擋得最多；每個動作多一次模型判斷（慢、花錢）；**只有 sonnet／opus 能用，haiku 會不報錯地退回 manual（F5）**，manual 下的提示沒人按，agent 就停住。
- 建議（請你決定）：B。
- 理由：不增加停住的機會，又用官方保證有效的 deny 規則擋住最直接的風險。
- 替代方案：A、C。
- 例子：選 B 時 agent 跑 `gh pr merge 42` → claude 回 `denied by permission rule`。
- 關係：第 7 施工關 P4（codex full-access）是同一類決定。bypass 警告在你的機器上不會出現（F4）。
- [ ] 使用者確認（選 ＿＿）

### P4：agent 可以用你的 gh 登入直接 merge（安全決定，請選一個）

- 問題：agent 沒有沙箱，PATH 上的 `gh` 用的是你的登入。它可以直接 `gh pr merge 42`，跳過之後流水線的人工核准；shim 只守 git。好意的 agent 很可能手滑打出這行（它以為最後一步就是 merge），照第 3 施工關的判斷方法屬於應該擋的。這件事在 claude 一跑起來就存在，跟 C 段做不做無關。
- 選項（只用 A 段做得到的）：
  - A. shim 也包 `gh`：拒絕 `pr merge`、`pr review --approve`、`api` 打到 merge／reviews 路徑、`auth token`，其餘放行。所有 backend 都有效。
  - B. 只靠 P3 的 deny 規則：只對 claude、只在 P3 選 B 時有效。
  - C. 接受，寫明：本段不處理，請你在 GitHub 對 main 開 branch protection。
- 建議（請你決定）：A。
- 理由：F6 證實 claude 跑的指令會先找到 shim，包一層就能對每個 backend 擋掉最常見的手滑。
- 替代方案：B、C。
- 例子：選 A 時 agent 跑 `gh pr merge 42` → `agend-shim: refused gh pr merge (merges go through agend)`，exit 1。
- 關係：A **擴大第 3 施工關 shim 的範圍（原本只有 git、pkill、killall），請明確決定**。
- [ ] 使用者確認（選 ＿＿）

### P5：claude 的啟動對話框

- 問題：F1、F3：新目錄第一次有信任對話框；**每次啟動**都有 development channels 警告（包括 holder 死掉重起）。沒人按，claude 就停在那裡，也收不到訊息。
- 選項：
  - A. 兩個都用螢幕規則（[delivery](../architecture/delivery.md) 第 3 層：比對畫面 → 按鍵），只做這兩個已知畫面，各附一份真畫面 fixture：development channels 按 `Enter`；信任按 `Down`＋`Enter`。
  - B. development channels 用螢幕規則按 `Enter`；信任由你手動：`agend instance add` 之後印一行「第一次要在 TUI 接受信任」，你 attach 進去按一次（claude 自己記在 `~/.claude.json`，之後不再問）。
  - C. 不用 development channel（沒有 channel，D16 的「閒置經 channel 送」做不到）。
- 建議（請你決定）：A。
- 理由：信任只要做一次，但每新增一個 instance 都要，手動很容易忘；兩個畫面都是固定文字，fixture 就是 F1 的畫面。
- 替代方案：B、C；預寫 `~/.claude.json` 的信任狀態（改你的檔案）。
- 例子：新 instance 第一次起 → log `g12-c: dialog "trust" answered (Down, Enter)`、`dialog "dev-channels" answered (Enter)`。
- 關係：A 的信任要兩個鍵，**與 delivery.md 第 3 層「單一按鍵」不同，請明確決定**；第 3 層本身是架構頁已定的機制，這裡只用在兩個已知畫面，不做未知提示的偵測。
- [ ] 使用者確認（選 ＿＿）

### P6：claude 的三級忙碌與忙／閒

- 問題：queue、steer、interrupt 怎麼落到 claude？daemon 怎麼知道它忙不忙？ctrl+enter 要不要用？
- 建議：照 D16 原文。忙：`UserPromptSubmit`。閒：`Stop`（沒被 block）、`SessionStart`；daemon 自己送 `Esc` 之後。閒置：經 channel 送。`Queue`（忙）：放 daemon 的佇列，下一個 Stop（`stop_hook_active: false`）全部取出、合成一個 reason 回 `block`。`Steer` → core 改成 `Interrupt`。`Interrupt`：holder 送單一 `Esc` → 立刻經 channel 送。`PreToolUse`、`PostToolUse` 只記成事件，不改忙閒。ctrl+enter 不用：官方文件說它送的是「你在 TUI 打字排隊的訊息」（[interactive mode](https://code.claude.com/docs/en/interactive-mode)，`chat:sendNow`），channel 的訊息算不算沒寫（U2）；替代的 `Ctrl+X Ctrl+S` 是兩個鍵。
- 理由：D16 有 spike 證據（忙碌時經 channel 送，模型可能不做，spike C1；Stop hook 3/3）；不加新規則。
- 替代方案：忙碌時也經 channel 送（推翻 D16）；用 ctrl+enter 強送（U2 查證後才能提）。
- 例子：agent 在忙，`agend send --level queue g12-c "m-q"` → `queued`；這輪結束時 Stop hook 回 `block` → 下一個 Stop 帶 `stop_hook_active: true` → `m-q confirmed`。F8：背景工具結束時多一輪（`UserPromptSubmit` 的 prompt 是 `<task-notification>`），照同一套規則是忙→閒，不會誤判。
- 關係：D16 照做。你在 TUI 自己按 `Esc` 的限制見「A 段不做」。
- [ ] 使用者確認

### P7：claude 的送達確認與事件

- 問題：`sent`、`confirmed` 各在什麼時候成立？daemon 不在時的事件（DRV-6）從哪補？
- 建議：`sent`＝bridge 寫進 claude，或 Stop hook 的 block 已回出去。`confirmed`＝channel 送的看 `UserPromptSubmit` 的 prompt 有 `delivery_id="<訊息 id>"`（錄製檔 `one_turn`）；Stop hook 送的看下一個 `stop_hook_active: true` 的 Stop（錄製檔 `busy`）。冪等照第 7 施工關 P5，當掉後不重送。事件：新表 `driver_events`（hook 事件照到達順序存，`seq` 當 cursor），保留 14 天（同事件，D31）；migration 取開工時的下一個空號（目前是 `0005`，但第 9、10 施工關可能先用掉，取當時下一個空號）。
- 理由：hook 是 claude 唯一的結構化事件；第 7 施工關的 `messages` 表與 DRV 契約照用，只多一張存 hook 的表。
- 替代方案：讀 claude 的 transcript 檔當事件日誌（綁它的檔案格式）；開機重送（可能重複一輪，違反 DRV-9）。
- 例子：daemon 停著時 Stop hook 送出排隊的訊息（hook 寫進 spool）→ 開機補送 → `m-q confirmed`。
- 關係：第 7 施工關 P5、P7；D31。
- [ ] 使用者確認

### P8：claude 不需要 `ZDOTDIR`

- 問題：第 7 施工關為 codex 加了 `ZDOTDIR`，因為 macOS login zsh 會把系統路徑排到 shim 前面。claude 要不要也加？
- 建議：**不加**。F6：claude 的 Bash 工具是 `/bin/zsh -c source <snapshot>`（不是 login shell），snapshot 最後把 PATH 設回 claude 啟動時的樣子，shim 在第一個；`pkill` 是 claude 的 shell function，最後也呼叫 PATH 上的 shim。`claude_live` 每次都檢查 `command -v git pkill killall`，哪天 claude 改了就會看到。
- 理由：實測不需要；不改第 6 施工關的環境白名單。
- 替代方案：先加再說（多一個變數、要改第 6 施工關 H3）。
- 例子：F6 的輸出：`/private/tmp/agend-rec-g12-e2/bin/git`、`/private/tmp/agend-rec-g12-e2/bin/killall`、`pkill is a shell function …`。
- 關係：跟第 7 施工關 K8「`ZDOTDIR` 只給 codex」一致。
- [ ] 使用者確認

### P9：holder 死掉後清掃 claude

- 問題：第 7 施工關的清掃只認得 codex 的標記；holder 被 `kill -9` 後 claude 或它的 MCP server、背景工具可能留著。要不要排在 A 段？
- 建議：排在 A 段，完全照第 7 施工關 P2 的條件與時機，只加 claude 的標記：argv 裡 `--session-id`／`--resume` 後面那個元素，或 `agend channel --instance <id>` 的 `<id>`，都要完全相等。
- 理由：同一個機制，只換標記。
- 替代方案：另開一個施工關（claude 死掉可能留孤兒）。
- 例子：`kill -9` holder → log `sweep of agent group 5231 (holder died): SIGKILL sent (claude --resume 3f2a…)` 或 `already gone`。
- 關係：第 7 施工關 P2 與「已知風險」（當時問排在哪）。
- [ ] 使用者確認

### P10：什麼是假的、什麼是真的

- 問題：CI 不能跑真的 claude。對什麼測？真的誰跑？
- 建議：CI 對 `fake-claude`（已有；補 1.4 的 bridge、spool、`--setting-sources`／`--settings`）跑 DRV 契約（DRV-6、DRV-9 四次開機）。錄製檔是 2.1.282、你機器上是 2.1.283：**重錄要你另外核准**（`cargo xtask record claude --sandbox …`，花少量 token）。真的、選做：`claude_live`（`agend-daemon` 的 example，`AGEND_REAL_CLAUDE=1`、在 `record-sandbox.sh` 裡跑、workspace 在 `/private/tmp/agend-rec-live-*`，因為沙箱只准 claude 寫 `~/.claude/projects/-private-tmp-agend-rec-*`；約 3 個短回合）。本 agent 與 verifier 不跑真 claude，除非你另外授權。
- 理由：邏輯全在 CI 驗，只有「真 CLI 接不接受」要真跑（第 7 施工關 P8）。
- 替代方案：CI 跑真 claude（要登入、花錢、不穩）。
- 例子：`contract Driver: claude+fake-claude 9/9 pass`。
- 關係：D9；第 7 施工關 P8；使用者 2026-09-25 的一致性檢查決定。
- [ ] 使用者確認

### A 段不做（你會看到的限制）

- **你在 TUI 按 `Esc` 中斷 claude 後，排隊的訊息要等下一次 Stop 才送出。** F7：工具跑到一半被 `Esc`，之後沒有 `PostToolUse`、沒有 `Stop`，daemon 一直當它在忙。你再打一句話或它下一輪結束，排隊的才會送。
- **第一次啟動、在第一則訊息之前就死掉的 claude instance 會變成 `failed`，按 `retry` 也起不來，要刪掉重建。** F9：claude 還沒存 session，`--resume` 回 `No conversation found`；第 6 施工關規定之後只能 `--resume`。
- 工作中途跳出的提示（P3 選 C 時的授權提示、其他沒見過的畫面）：沒有偵測；agent 停著，你在 TUI 看得到。「卡在未知提示」（delivery.md 第 4 層）不在 A 段。
- `PermissionRequest` hook 不回應（claude 照常顯示對話框）；授權轉給人回答不在 A 段。
- ctrl+enter 強送（P6、U2）。
- 卡住偵測、usage limit、依緊急程度自動選忙碌等級。

### 已知風險（開工時處理）

- 依賴第 9、10 施工關的實作：協定號（1.4）、migration 號、`agend send` 與 `fake-worker` 的確切行為，開工時照實際的改這頁。
- claude 的 development channel 是 research preview，旗標與對話框可能隨版本改（F3 的警告每次都跳）；driver 啟動時 log `claude --version`。
- claude 會自己改指令（F8：`sleep 25` 被改成背景、前景的被擋），`claude_live` 的測試指令要用它不會改的寫法。

**未查證的事實**：

| # | 事實 | 影響 | 怎麼查 |
|---|---|---|---|
| U1 | 2.1.283 的 channel 訊息在 `UserPromptSubmit` 仍帶 `delivery_id`（錄製是 2.1.282） | P7 | 重錄（要你核准） |
| U2 | ctrl+enter（`chat:sendNow`）對 channel 送來的訊息有沒有作用 | P6 | `claude_live` 加一段 |
| U3 | deny 規則在我們的 `--settings` 裡、bypass 模式下實際擋得住 `gh pr merge` | P3 選 B | 官方文件說會；`claude_live` 加一段 |
| U4 | 螢幕規則比對的信任、development channels 畫面在不同終端寬度下文字是否一樣 | P5 | 開工時錄兩個寬度的 fixture |

## 自動驗收（完成定義）

- [ ] P1 的「重過第 1 施工關」：`~/.cargo/bin/cargo test -p agend-core`、`~/.cargo/bin/cargo xtask check-deps`、`~/.cargo/bin/cargo xtask accept core` 通過；你重跑第 1 施工關的「你親自驗收」
- [ ] `~/.cargo/bin/cargo test -p agend-daemon`、`~/.cargo/bin/cargo test -p agend-testkit`、`~/.cargo/bin/cargo test -p agend` 單獨通過，包括：claude driver 對假 claude 跑 DRV-1..9（DRV-6、DRV-9 四次開機）；hook 在 daemon 停著時寫 spool、開機補送；Stop hook 一次取出全部佇列、`stop_hook_active: true` 不再 block；忙碌時不經 channel；雜湊不符的 `.mcp.json`／CLAUDE.md → `failed`；`delivery = inbox` 不寫設定檔；P5 選定的對話框規則對 F1 的畫面 fixture 按對鍵；清掃只殺自己的 group；P3、P4 選定的選項各有一條測試；schema fixture 與 golden 更新
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept adapters` 通過，並印出下方步驟 1 的 demo
- [ ] 真 CLI 一致性檢查（必要；使用者已決定 2026-09-25）：`claude --version` 和 `crates/agend-testkit/transcripts/claude/` 錄製檔 header 的 `version` 相同，不同就先重錄（[RECORDER.md](../../crates/agend-testkit/RECORDER.md#重錄cli-升版時)）；`~/.cargo/bin/cargo test -p agend-testkit --test conformance` 通過
- [ ] `agend-daemon` 的 `README.md`／`TESTING.md` 已更新；[backends/claude-code.md](../backends/claude-code.md) 補上 F1–F10
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

目前只有 A 段的步驟。由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。`<t-N>` 這類尖括號是會變的值；標「開工時細化」的是輸出的確切字樣。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.x.y`（目前是 `agend 0.0.0`）。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

`AGEND_HOME` 一定要設（第 13 施工關之前沒有預設值）。步驟 1–3 自己建暫存 home；步驟 4 起用步驟 4 印出的 home，每個步驟的指令第一行都是 `export AGEND_HOME=<home>`。另開一個「watch 分頁」：先跑開頭那段，再 `export AGEND_HOME=<home>` 與 `agend debug watch`，之後說「watch 印」就是看這個分頁。

1. 跑 demo（對假 claude）。

   **這步在驗什麼**：claude driver 在假 claude 上走得通：三級忙碌、Stop hook 排隊、確認、daemon 停著時的 hook 補送、四次開機。錯了代表後面真的步驟看到的都不可信。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo xtask accept adapters
   ```

   應該看到：`== claude` 與 `== restart` 兩節，倒數第二行 `adapters demo (segment A): all sections passed`，最後一行 `gate 12 (adapters): checks passed`（開工時細化）。

   - [ ] 通過

2. 真 CLI 一致性檢查（必做）。

   **這步在驗什麼**：假 claude 和你機器上真的 claude 形狀一致。壞了的話，driver 對假的全綠、接上真的才出錯（v1 #1483）。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   claude --version
   head -1 crates/agend-testkit/transcripts/claude/one_turn.jsonl
   ~/.cargo/bin/cargo test -p agend-testkit --test conformance
   ```

   應該看到：版本和錄製檔 header 的 `"version"` 相同；最後 `test result: ok.`。版本不同（2026-09-28 時是 2.1.283 對 2.1.282）：先重錄（`~/.cargo/bin/cargo xtask record claude --sandbox ~/Documents/Hack/AgEnD-ops/record-sandbox.sh`，花少量 token，見 [RECORDER.md](../../crates/agend-testkit/RECORDER.md)）再跑；不過就改假 claude，不改錄製檔。

   - [ ] 通過

3. 真 claude 端到端（`claude_live`，約 3 個短回合）。

   **這步在驗什麼**：兩個對話框被規則處理（P5）、P3 選的權限模式生效、channel 訊息被確認、claude 跑的 `git`／`pkill`／`killall` 是 shim（P8）、holder 被 `kill -9` 後沒有 claude 留下、重起後上下文還在。錯了的話，真 claude 會卡在對話框，或 agent 的 git 繞過 shim。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo build -q -p agend --bin agend && ~/.cargo/bin/cargo build -q -p agend-daemon --example claude_live
   AGEND_REAL_CLAUDE=1 ~/Documents/Hack/AgEnD-ops/record-sandbox.sh target/debug/examples/claude_live 2>/tmp/g12-claude.log | tee /tmp/g12-claude.out
   pgrep -fl "agend (holder|daemon|channel)"
   ```

   應該看到（開工時細化）：`dialog "trust" answered`、`dialog "dev-channels" answered`、`m-1 idle → channel → confirmed`、三行 `(the shim)`、（P3 選 B）`gh pr merge → denied by permission rule`、`claude left: []`、`restart 1/3, session <S> resumed`、`dialog "dev-channels" answered`（重起後又跳一次）、`m-2 → confirmed; reply mentions m-1's word: true`、最後 `claude_live: ok`；`pgrep` 沒有輸出。失敗時看 `/tmp/g12-claude.log` 最後 40 行。

   - [ ] 通過
   - [ ] 這次不做（寫進驗收紀錄）

4. 真 daemon 上兩個 claude 互傳訊息。

   **這步在驗什麼**：真的 daemon、真的 claude：一則閒置時送、一則忙碌時排隊，兩則都到 `confirmed`（P6、P7）。錯了的話訊息會停在 `sent` 或 `queued`。

   第一個分頁：

   ```bash
   export AGEND_HOME="$(mktemp -d /tmp/g12.XXXX)" && echo "export AGEND_HOME=$AGEND_HOME"
   agend daemon
   ```

   記下印出的 `export AGEND_HOME=…`，開 watch 分頁。第三個分頁（先跑開頭那段）：

   ```bash
   export AGEND_HOME=<home>
   agend instance add g12-a claude; agend instance add g12-b claude
   agend send g12-a "Count from 1 to 300, one number per line, then reply DONE."
   agend send --level queue g12-a "Reply with exactly: QUEUED-OK"
   agend send g12-b "Reply with exactly: B-OK"
   ```

   應該看到：watch 依序印 `g12-a` 第一則 `confirmed`、第二則 `queued` → 第一輪結束時 `sent`（Stop hook）→ `confirmed`；`g12-b` 那則 `confirmed`（確切字樣開工時細化）。

   - [ ] 通過

5. 故意弄壞：工作目錄裡已經有你自己的 CLAUDE.md。

   **這步在驗什麼**：daemon 不會蓋掉不是它寫的檔（P2）。錯了的話你放在專案裡的 CLAUDE.md 會被默默改掉。

   ```bash
   export AGEND_HOME=<home>
   mkdir -p /tmp/g12-mine && echo "my notes" > /tmp/g12-mine/CLAUDE.md
   agend instance add g12-m claude --dir /tmp/g12-mine
   cat /tmp/g12-mine/CLAUDE.md
   ```

   應該看到：watch 印 `g12-m failed: CLAUDE.md exists and was not written by agend`；`cat` 印 `my notes`。收尾：`agend instance remove g12-m --yes; rm -rf /tmp/g12-mine`。

   - [ ] 通過

6. 收尾。

   **這步在驗什麼**：什麼都不留（第 6 施工關的孤兒巡查照舊）。

   各分頁 Ctrl-C，然後：

   ```bash
   export AGEND_HOME=<home>
   agend instance remove g12-a --yes; agend instance remove g12-b --yes
   pgrep -fl "agend holder g12-"; pgrep -fl "agend channel"
   ```

   （`instance remove` 要 daemon 在跑：先在第一個分頁 `agend daemon`，跑完再 Ctrl-C。）應該看到：兩個 `pgrep` 都不印。最後 `rm -rf "$AGEND_HOME"`。

   - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-28 使用者決定換路：比照第 11 施工關分段，這頁只留 A 段（claude）提案 P1–P10；不加新的 core 事件與偵測規則；經授權在錄製沙箱跑真 claude 2.1.283 查 F1–F10（5 個短回合）。四段一起的前一版在 `74ced40`。
- 2026-09-28 第 2 輪 review REFUTED 後改寫成 P1–P34（`74ced40`）；第 3 輪 review 仍 REFUTED。
- 2026-09-28 第 1 輪 review REFUTED 後改寫成 P1–P26（`0ac05ad`）。
- 2026-09-28 開工前提案 P1–P21 寫定（draft PR #138，branch `docs/gate-12-proposal`）；狀態改為提案中。
- 2026-09-25 使用者決定：真 CLI 一致性檢查（錄製器 + `tests/conformance.rs`）列為必要完成條件（`feat/backend-recorder`）。

## 下一步

```bash
cat docs/gates/gate-12-adapters.md
~/.cargo/bin/cargo xtask accept adapters
```
