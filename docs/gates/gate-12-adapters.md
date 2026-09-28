# 第 12 施工關：其餘 adapter（`adapters`）

> **TL;DR**
> - claude driver、opencode driver、forge github、Telegram 通知；分四段做，各段自己驗收（P1）。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：開工前提案 P1–P26 等你逐題確認（P4、P10、P17、P18 是安全決定，沒有預設答案）；確認後 merge 這份提案，等第 9–11 施工關完成再開工。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**提案中**（2026-09-28）：開工前提案 P1–P26 寫定，等你確認（第 1 輪 review REFUTED 後改寫）。依賴：第 7 施工關（送達、`messages` 表、`sh` 包裝、清掃）已驗收；第 9 施工關（CLI、`agend send`、`doctor`、協定 1.2）、第 10 施工關（流水線、forge local、「需要你」的來源、沙箱、協定 1.3）提案已確認、還沒實作；第 11 施工關 B 段進行中。這些施工關做完時跟這裡的理解不同，改這頁，不改它們。

## 範圍

- claude driver（D16）：bridge 與 hooks、設定檔、權限模式、PATH、啟動對話框、三級忙碌、送達與事件、resume 與清掃（P2–P9）
- opencode driver：程序與密碼、session、送達與事件、權限設定（P10–P13）
- forge github：`gh`、head 以哪邊為準與 push、merge、agent 的 gh 權限與 GitHub CI（P14–P17）
- notifier：Telegram 的 token、HTTP client、core 的改動、發什麼、收什麼、可靠性（P18–P23）
- 第 8 施工關移來的 G4 已讀狀態（P24）
- 其他施工關記給本關的事，以及 `agend doctor` 延到本關的幾列（P25）
- 什麼是假的、什麼是真的（P26）

### 從其他施工關帶來的筆記（已併進提案）

- **G4 已讀狀態**（[gate-08-client P6](gate-08-client.md#p6真-daemon-本關做哪些請求g4-移走)）→ P24。
- **claude Bash 工具的 PATH 重排**（[gate-03-shim 已知限制](gate-03-shim.md#已知限制)、第 7 施工關 P4、K8）→ P5、U5。
- **使用者提供、未查證：claude 按 ctrl+enter 可以把訊息「強送」給正在工作的 agent**（2026-09-26）→ P7、U2。官方文件有這個鍵，但說的是「你在 TUI 打字排隊的訊息」，對 channel 送來的訊息有沒有用沒寫。
- **holder 被 `kill -9` 後其他 backend 的清掃**（第 7 施工關「已知風險」，問排在本關還是另開）→ P9、P10。
- **第 10 施工關移來**：forge github、GitHub CI、逾時「通知」送 Telegram、卡住偵測／usage limit／自動選忙碌等級（建議移來本關）→ P14–P17、P25。第 10 施工關的假 agent（`fake-worker`，backend 登記成 `claude`、`delivery = inbox`）→ P3。
- **第 9 施工關移來**：`doctor` 的 backend、gh、Telegram 幾列；第一個讀 `config.toml` 的施工關負責建它 → P18、P25。

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子 · 跟既有決定的關係。跟既有決定不同的地方標「**與已確認／已追認的 X 不同，請明確決定**」。**安全決定**（P4、P10、P17、P18）只列選項與建議，沒有預設答案，請你選。

文件已定、這裡不重問的：claude 用互動 TUI＋hooks＋channel、忙碌時用 Stop hook 排隊、中斷＝`Esc` 後立刻送（D16）；opencode 用 `serve` 的 HTTP＋SSE、SSE 沒有重播、授權以 `GET /permission` 為準（[backends/opencode.md](../backends/opencode.md)）；送達狀態四個、只有一套冪等、推送帶完整內容、不支援插入就改中斷（[delivery](../architecture/delivery.md)、`policy::busy::effective_level`）；附屬程序由 holder 持有、用 `sh` 包裝放在同一個 process group（第 7 施工關 P2）；daemon 先建 session、先存 DB 再讓 TUI 起來（第 7 施工關 P3 的做法）；GitHub CI 用 `command` 關卡接（D29）；Telegram 一個「需要你」topic＋每個 team 一個（D13）；「需要你」的項目與 `resolve_attention` 只收操作者（第 8 施工關 P2、P5；第 10 施工關 P8）；daemon 的 git 經 `Runner`、`-c core.hooksPath=/dev/null`（第 10 施工關 P5）；checks 在寫入沙箱裡跑（第 10 施工關 P6）；agent 可以 push 自己綁定的 branch（第 3 施工關 T7）；shim 的威脅模型：防好意但會犯錯的 agent、不防故意繞過，硬保證是 hook＋forge 端的 branch protection（[第 3 施工關](gate-03-shim.md)）；真 CLI 一致性檢查是必要完成條件（使用者 2026-09-25）。

事實來源：[BACKEND-BEHAVIORS](../BACKEND-BEHAVIORS.md)、[backends/](../backends/claude-code.md)、`crates/agend-testkit/transcripts/{claude,opencode}/` 的錄製檔（claude 2.1.282、opencode 1.18.31）、官方文件（附 URL）。標 **未查證** 的集中在「未查證的事實」表（U1–U22）。本 agent 沒有跑任何真的 claude、opencode、gh 或 Telegram。

### P1：本關怎麼分段

- 問題：四樣東西彼此幾乎不相干，全部做完才驗收，一次要審的量太大。要不要分段？
- 建議：同一個施工關、分四段，**每段一個 PR、各自驗收**：A claude（P2–P9）、B opencode（P10–P13）、C forge github（P14–P17）、D Telegram（P18–P24）。P25、P26 跟著各段走。四段彼此不依賴（D 段的核准按鈕用第 10 施工關 forge local 產生的項目就能驗），順序可以任意。「你親自驗收」步驟 5（三個真 backend）要 A、B 都完成，歸 B 段驗收。四段都驗收完，第 12 施工關才算完成。
- 理由：前面每關都要審 3–4 輪；分段後每輪只看一塊。第 13 關數目不變。
- 替代方案：拆成 12a–12d 四個施工關（ROADMAP 與 D22、D24 的「13 個施工關」要改）；不分段（一次審四樣）。
- 例子：A 段 PR `feat/gate-12a-claude` 驗收完 merge，頁面進度紀錄寫「A 段完成」，狀態維持「實作中」直到四段都完成。
- 關係：D22（每關確認後才開下一關）不變，分段只在關內。
- [ ] 使用者確認

### P2：claude 的訊息與 hook 怎麼到 daemon（協定 1.4）

- 問題：claude 的訊息從 channel（claude 自己起的 MCP server）進去，狀態從 hooks（claude 跑的指令）出來。這兩個程式是什麼？怎麼連 daemon？daemon 不在時怎麼辦？
- 建議：
  - 兩個都是 `agend` 的內部子命令（不在 `--help`）：`agend channel --instance <id>`（MCP stdio server）、`agend hook <事件名>`（讀 stdin 的 JSON）。環境裡已有 `AGEND_INSTANCE`，照第 8 施工關 P2 當成那個 agent。
  - 都走 client 協定，加 agent 限定的請求，是 **1.4**（1.2 第 9 關、1.3 第 10 關）：`channel_attach`（daemon 之後推 `channel_message {id, content, meta}`，bridge 寫進 claude 後回 `channel_written {id}`）、`hook_event {event, payload}`（Stop 的回覆帶「要不要 block、reason」，P7）。協定型別在 core（D11），所以照 D22 先改 core、重跑第 1 施工關的協定相容與 golden 測試（P20 一起）。
  - daemon 不在：`agend hook` 把事件寫到 `$AGEND_HOME/spool/hooks/<instance>/<序號>.json` 就結束（Stop 回 `{}`、不 block）；daemon 開機時照序號補送（`ingest` 模組開頭寫的做法）。`agend channel` 每秒重連。
  - hook 的 timeout 在設定裡寫 10 秒（官方預設 600 秒，daemon 卡住時 claude 會等 10 分鐘，[hooks](https://code.claude.com/docs/en/hooks)）。
- 理由：同一個 binary、同一套有版本的協定（ARCHITECTURE 程序模型第 4 條：bridge 升級前跑舊 binary，協定必須向後相容）；hooks 丟掉就一直被當成 busy（V1-LESSONS #9）。
- 替代方案：bridge 另開一條私有 socket 協定（多一套要管版本的協定）；hook 用 `curl` 打 HTTP（daemon 要多開 HTTP server）；daemon 不在時 hook 直接丟（v1 的坑）。
- 例子：`agend send g12-c "hi"` → bridge 收到 `channel_message m-7` → 寫 `notifications/claude/channel {content:"From: …\n\nhi", meta:{delivery_id:"m-7"}}` → 回 `channel_written m-7` → `messages.state = sent`。
- 關係：D7（CLI 為主）不變，MCP 只用在 claude 要求的 channel；D11、D22（core 先改）。
- [ ] 使用者確認

### P3：claude 的設定檔放哪、假 agent 怎麼辦

- 問題：hooks、`.mcp.json`、「訊息來源說明」寫在哪？會不會蓋到你的檔案？第 10 施工關登記成 `claude` 的假 agent 要不要也套這些？
- 建議：
  - hooks 放 daemon 的檔案，用 `--settings $AGEND_HOME/claude/<id>/settings.json` 帶進去（官方文件：`--settings` 優先於專案設定）。
  - `.mcp.json` 與 `CLAUDE.md` 一定要在工作目錄（`--mcp-config` 會讓 `server:<name>` 找不到，backends/claude-code 陷阱），寫進 instance 的 workspace。**判斷「是不是我們寫的」用 DB 記的 sha256**（每次寫入時記在 instance 列）：檔案存在、雜湊不是我們記的 → 不覆蓋，instance `failed`，原因寫出是哪個檔。`.mcp.json` 是 JSON、不能放註解，所以不用標記行。
  - 用你自己的 `~/.claude` 設定（不加 `--setting-sources`，跟 codex 用你的 `~/.codex` 一樣）；daemon 不寫 `~/.claude.json` 或 `~/.claude/` 裡任何檔（claude 自己寫的不算）。
  - **只套在 `delivery = push` 的 claude instance**。第 10 施工關的 `delivery = inbox`（`fake-worker`）不寫這些檔、不加 P4／P6 的旗標、不建 driver；只保留第 6 施工關的 `--session-id`／`--resume`（`fake-worker` 忽略它們）與 P9 的重起規則。
- 理由：hooks 放 daemon 自己的目錄，不動你的 repo；MCP 與 CLAUDE.md 沒有別的放法；雜湊不需要改檔案內容就能辨認。
- 替代方案：隔離的 `CLAUDE_CONFIG_DIR`（spike 時要重新登入，BLOCKED）；`--append-system-prompt` 代替 CLAUDE.md（spike F 沒測過，效果未查證）；有檔案就直接覆蓋（會蓋掉你放在 `--dir` 的東西）。
- 例子：`agend instance add g12-c claude --dir ~/proj`，`~/proj/CLAUDE.md` 已經是你的 → `g12-c failed: CLAUDE.md exists and was not written by agend`。
- 關係：D16（CLAUDE.md 說明來源）照做；第 10 施工關 P11 的 `delivery = inbox` 不受影響。
- [ ] 使用者確認

### P4：claude 的權限模式（安全決定，請選一個）

- 問題：claude 沒有人在旁邊按「允許」。用哪個權限模式？
- 官方原文（[permission modes](https://code.claude.com/docs/en/permission-modes)）：
  - bypass：「Only use this mode in isolated environments like containers, VMs, or dev containers without internet access, where Claude Code cannot damage your host system.」「`bypassPermissions` offers no protection against prompt injection or unintended actions. For background safety checks with far fewer permission prompts, use auto mode instead.」bypass 也會寫入 protected paths（`.git`、`.claude` 等）；對 critical path 的 `rm` 仍會跳提示。第一次互動啟動 bypass 會跳一個警告對話框，接受後寫進 user settings。
  - deny 規則：「Deny rules block in every mode, including `bypassPermissions`.」
  - auto：「A separate classifier model reviews actions before they run, blocking anything that escalates beyond your request, targets unrecognized infrastructure, or appears driven by hostile content Claude read.」預設擋的清單裡有「Merging a pull request no human has approved」。v2.1.283 起是互動模式的預設；模型要 Opus／Sonnet 4.6 以上（haiku 不行）；寫在專案設定的 `auto` 不生效。「Auto mode reduces permission prompts but does not guarantee safety.」
- 風險：agent 在你的 uid、你的網路、你的 gh 登入下跑（P17）。進來的文字有三個來源：其他 agent 的 `agend send`、你在 Telegram 回的請示、它讀到的檔案與網頁；任何一個都可能帶 prompt injection。shim 只守 git／pkill／killall（第 3 施工關）。
- 選項：
  - **A. bypass**：跟 codex 的 `never`＋`danger-full-access` 對齊；什麼都不擋。第一次要你自己在終端機跑一次 `claude --permission-mode bypassPermissions` 接受警告（daemon 不寫你的設定）。
  - **B. bypass ＋ deny 規則**：`--settings` 裡加 `permissions.deny`，至少擋 `Bash(gh pr merge*)`、`Bash(gh api*merge*)`、`Bash(gh pr review*--approve*)`、讀 `$AGEND_HOME/secrets/**`、`$AGEND_HOME/run/**`。deny 是字串比對，刻意改寫就繞得過（在第 3 施工關威脅模型之內：擋手滑，不擋故意）。
  - **C. auto**：`--permission-mode auto`。有 classifier 擋越權與 injection，而且預設就擋未核准的 merge；代價：每個動作多一次模型判斷（慢、花錢）、模型限制、還是可能跳提示（沒人按就卡住，要靠螢幕 hard gate 轉成「需要你」，第 11 施工關之後才有人回答）。從 CLI 旗標帶 `auto` 在 daemon 起的互動 session 是否生效**未查證**（U6）。
  - D. 預設（manual）＋ `PermissionRequest` 全部轉「需要你」：最安全，但本關還沒有回答的 UI，agent 幾乎每一步都會停。
- 建議（請你決定）：**B**，C 等 U6 查證、模型確定後再考慮換。理由：B 不增加卡住的機會，又補上最直接的一條（agent 自己 merge），跟 P17 的處理互相補。
- 理由：B 不增加卡住的機會，又用官方保證「在 bypass 也有效」的 deny 規則補上最直接的風險（agent 自己 merge、讀 secret）；C 的保護最強，但有模型限制與卡住的風險，現在驗不到。
- 替代方案：上面的 A、C、D。
- 例子：選 B 時 `ps` 看到 `claude … --permission-mode bypassPermissions --settings …/g12-c/settings.json`，settings 裡 `"deny": ["Bash(gh pr merge*)", …]`；agent 跑 `gh pr merge 42` → claude 回 `denied by permission rule`。
- 關係：D16 沒規定權限；第 7 施工關 P4（codex full-access）是同一類決定。
- [ ] 使用者確認（選 ＿＿）

### P5：claude、opencode 跑的指令找不找得到 shim（PATH）

- 問題：macOS 的 login zsh 會把系統路徑排到 shim 前面（第 7 施工關 P4）。claude 的 Bash 工具、opencode 的 bash 工具也這樣嗎？
- 建議：第 7 施工關 K8 的 `ZDOTDIR`（在 `path_helper` 之後把 `$AGEND_HOME/bin` 放回最前面）**也給 claude 與 opencode**，先做再用 `claude_live`、`opencode_live` 的 `command -v git pkill killall` 驗（U5、U11）。**與已追認的第 7 施工關 K8「`ZDOTDIR` 只給 codex」不同，也再改一次第 6 施工關 H3 的環境白名單，請明確決定**。
- 理由：shim 是 git／pkill／killall 唯一的防護，先修再驗；不是 login shell 時 `ZDOTDIR` 沒有作用、也沒有壞處。
- 替代方案：等 U5、U11 查完再決定（中間這段時間 shim 可能被繞過）；只給 claude。
- 例子：`claude_live` 印 `git: <H>/bin/git (the shim)`。
- 關係：第 7 施工關 P4、K8；第 6 施工關 H3。
- [ ] 使用者確認

### P6：claude 的啟動對話框

- 問題：信任資料夾、專案 MCP server、development channels 警告（選 bypass 時還有 bypass 警告）。沒人按就卡住。
- 建議：先「避免」：`settings.json` 放 `enabledMcpjsonServers: ["agend"]`（能不能跳過 MCP 對話框**未查證**，U3）。其他用螢幕規則檔（[delivery](../architecture/delivery.md) 第 3 層：比對畫面 → 單一按鍵），每條附真畫面 fixture（錄製情境 `startup_dialogs` 錄）。信任對話框預設游標在「No, exit」，規則要明確選「Yes」那一項，不能按 Enter（陷阱 6）。bypass 警告不自動按：選 P4 的 A、B 時由你事先接受一次；沒接受就停在那個畫面，出現「需要你」「卡在未知提示」。
- 理由：對話框的答案是資料，不寫進你的設定檔；bypass 的責任聲明應該是你自己按。
- 替代方案：預寫 `~/.claude.json` 的信任狀態（改你的檔案）；規則連 bypass 警告也自動接受（等於替你簽責任聲明）。
- 例子：新 workspace 第一次起 → log `g12-c: startup dialog "trust" answered (rule claude/trust, key 1)`。
- 關係：delivery.md 的四層；BACKEND-BEHAVIORS 陷阱 6。
- [ ] 使用者確認

### P7：claude 的三級忙碌與忙／閒

- 問題：queue、steer、interrupt 怎麼落到 claude？daemon 怎麼知道 claude 忙不忙？你自己在 TUI 按 `Esc` 時沒有任何 hook，daemon 會一直以為它在忙。
- 建議：
  - 忙／閒看 hooks：`UserPromptSubmit` → 忙；`Stop`（沒被 block）、`SessionStart` → 閒。daemon 自己送 `Esc` 時自己記成閒（錄製檔 `interrupt`：`Esc` 之後沒有 hook）。
  - 閒置：經 channel 送。`Steer` → core 改成 `Interrupt`。`Interrupt`：holder 送單一 `Esc` → 立刻經 channel 送（D16）。
  - `Queue`（忙碌）：訊息放 daemon 的佇列；下一個 `Stop`（`stop_hook_active: false`）時全部取出、合成一個 reason 回 `block`（D16）。
  - **等不到 Stop 的退路**（你在 TUI 按 `Esc`、claude 當掉又被 holder 留著）：排隊超過 60 秒、這段時間沒有任何 hook → 改經 channel 送。官方文件說 channel 訊息在忙碌時會排隊、這輪結束後處理（U1），所以就算它其實還在忙也不會丟。**與 D16「忙碌時一律用 Stop hook 排隊」不同（只在等不到 Stop 時），請明確決定**。
  - ctrl+enter 本關不用：官方文件說它送的是「你在 TUI 打字排隊的訊息」（`chat:sendNow`），channel 的訊息算不算沒寫（U2）；替代的 `Ctrl+X Ctrl+S` 是兩個鍵，違反「PTY 只送單一控制鍵」。
- 理由：Stop hook 是 D16 的主路（錄製 3/3）；退路補的是「hook 永遠不來」這種 V1-LESSONS #9 的同類問題，靠的是官方寫明的 channel 行為。
- 替代方案：用螢幕分類器認「Interrupted」畫面判斷閒置（**與 delivery.md「螢幕只認 hard gate」不同**）；不做退路（你按一次 `Esc`，排隊的訊息就卡到下一次它自己忙完）；忙碌時一律經 channel（推翻 D16）。
- 例子：你在 TUI 按 `Esc` 打斷 g12-c，此時有一則 `m-q` 在排隊 → 60 秒後 log `g12-c: no hook for 60 s while m-q waits; sent via channel` → `UserPromptSubmit delivery_id="m-q"` → `confirmed`。
- 關係：D16；D30 去抖動只給畫面用，driver 不經去抖動（第 7 施工關 P6 同理）。
- [ ] 使用者確認

### P8：claude 的送達確認、冪等、事件與 cursor

- 問題：`sent`、`confirmed` 各在什麼時候成立？`Driver::events(after_cursor)` 要補回 daemon 不在時的事件（DRV-6），事件從哪來？
- 建議：
  - `sent`：bridge 回 `channel_written`，或 Stop hook 的 block 回覆已交給 claude。
  - `confirmed`：channel 送的 → `UserPromptSubmit` 的 prompt 有 `delivery_id="<訊息 id>"`（錄製檔 `one_turn` 已證實）；Stop hook 送的 → 下一個 `stop_hook_active: true` 的 Stop（錄製檔 `busy` 只有這個證據）。
  - 冪等照第 7 施工關 P5（`messages` 表）；當掉後不重送，停在原狀態，hook 補回後自然確認。
  - 事件：新表 `driver_events`（instance、`seq`、種類、摘要、時間），hook 事件照到達順序寫入；cursor＝`seq`。保留 14 天（同事件，D31），列進第 5 施工關 P8 的規則表。
  - migration：取開工時的下一個空號（目前是 `0005`，但第 9、10 施工關可能先用掉，取當時下一個空號）；本關所有 schema 變動放同一個。
- 理由：hook 是 claude 唯一的結構化事件來源，自己存一份最簡單。
- 替代方案：讀 `transcript_path` 的 jsonl 當事件日誌（綁 claude 的檔案格式，U7）；開機時重送（可能重複一個 turn，違反 DRV-9）。
- 例子：daemon 停著時，排隊的訊息在 Stop hook 被處理（hook 寫進 spool）；開機補送 → `driver_events` 多兩列 → `m-q confirmed`。
- 關係：第 7 施工關 P5、P7。
- [ ] 使用者確認

### P9：claude 的 session、resume、清掃

- 問題：第 6 施工關 H1：「`Spawn` 被確認」不等於 claude 已把 session 存檔；沒送過訊息就死掉，`--resume <id>` 可能找不到。holder 被 `kill -9` 後 claude 或它的子程序可能留著。
- 建議：
  - `--resume <id>` 起來幾秒就結束、畫面有「找不到 session」類的字（確切字樣未查證，U4），**而且**從沒有 `sent` 以上的訊息 → 同一個 id 改用 `--session-id` 再起一次。其他情況 `failed`。**與已確認的第 6 施工關 P6「絕不自動全新啟動」不同，請明確決定**（跟 2026-09-26 對 codex 核准的例外同理：空 session 沒有上下文可丟）。
  - 清掃**排在本關**：照第 7 施工關 P2 的條件，標記是 argv 裡 `--session-id`／`--resume` 後面那個元素，或 `agend channel --instance <id>` 的 `<id>`，都要完全相等。
- 理由：跟 codex 同一套規則，只換標記。
- 替代方案：找不到一律 `failed`；清掃另開一個施工關。
- 例子：假 claude 第一次起來就被 `kill -9` → 重起時 `--resume` 找不到 → `started again with --session-id 3f2a…`。
- 關係：第 6 施工關 P6、H1；第 7 施工關 P2、P3。
- [ ] 使用者確認

### P10：opencode 的程序、密碼、以及 checks 能不能連到它（安全決定，請選一個）

- 問題：`opencode serve` 聽 loopback 的 TCP port。第 10 施工關的 checks 在沙箱裡跑 agent 寫的程式碼，而沙箱**允許 IP 網路**。checks 能不能連上 opencode、叫它在沙箱外跑指令？
- 程序：照第 7 施工關 P2 的 `sh` 包裝：背景 `opencode serve --hostname 127.0.0.1 --port 0`（輸出寫 log，daemon 從 log 讀實際 port，U9）→ `GET /global/health` 就緒 → 建／接 session（P11）→ `$GO` → `exec opencode attach http://127.0.0.1:<port> --session <id> --dir <workspace>`（[CLI 文件](https://opencode.ai/docs/cli/)）。清掃標記：argv 裡 `--session` 後面那個元素完全相等。
- 現況（第 1 輪 review 指出，前一版寫錯）：每個 instance 一個隨機密碼（`OPENCODE_SERVER_PASSWORD`，官方：serve 用 HTTP basic auth）**在 Linux 擋得住**（沙箱用 tmpfs 蓋掉 `$AGEND_HOME/run`），**在 macOS 擋不住**：macOS 的沙箱設定是 `(allow default)`＋`(deny file-write*)`，只擋寫、不擋讀；checks 的工作目錄在 `$AGEND_HOME/checks/<run>/`，往上兩層就是 `$AGEND_HOME`，讀得到密碼檔。另外 `ps eww` 可能讀得到同一個使用者程序的環境變數（未查證，U19）。同樣的路也通到任何聽 loopback 的服務。
- 選項：
  - **A. 密碼＋接受 macOS 的限制**：Linux 擋住；macOS 寫進已知風險。
  - **B. 密碼＋macOS 沙箱加一條 `(deny file-read* (subpath "$AGEND_HOME/run"))`**：擋住讀密碼檔；擋不住 U19 的環境變數那條（若成立）。改到第 10 施工關 P6 的沙箱設定。
  - **C. 沙箱擋掉 loopback 連線**（macOS `(deny network-outbound (remote ip "localhost:*"))`；Linux 要另找只擋 127.0.0.0/8 的做法，開工時查）：一次擋掉 opencode 與任何本機服務；代價是需要連本機資料庫或本機服務的 checks 跑不了。改到第 10 施工關 P6。
- 建議（請你決定）：**B＋密碼**，另把 U19 查清楚；C 最乾淨但會擋到正常的 checks，等有具體需要再加。選 B 或 C 都是**修改已確認的第 10 施工關 P6，請明確決定**。密碼經環境變數給 serve 與 attach，**多一個變數是修改第 6 施工關 H3 白名單，請明確決定**。
- 理由：B 只多一行沙箱規則，就把 macOS 補到跟 Linux 差不多；C 會擋到正常用到本機服務的 checks。
- 替代方案：上面的 A、C；或不設密碼（Linux 也擋不住）。
- 例子：選 B 時在 checks 裡 `cat $AGEND_HOME/run/holders/*.opencode.pw` → `Operation not permitted`；`curl http://127.0.0.1:<port>/session` → `401`。
- 關係：第 7 施工關 P2；第 10 施工關 P6；第 6 施工關 H3。
- [ ] 使用者確認（選 ＿＿）

### P11：opencode 的 session、資料目錄、舊的 instance

- 問題：第 6 施工關說 opencode「沒有 session id，死了就 `failed`」。session 誰建、存哪？用哪個資料目錄？
- 建議：
  - daemon 在 serve 就緒後 `POST /session`，**先存 `instances.session_id` 再寫 `$GO`**（第 7 施工關 P3）。重起：同一個資料目錄、`attach --session <id>`（spike O4）。
  - 資料目錄用你原本的（`~/.local/share/opencode`），登入共用；daemon 不寫你的 opencode 設定；不加 `--pure`（你的 plugin 照常載入）。
  - 第 12 關之前建的 opencode instance，`session_started = 1` 而 `session_id` 是 NULL → migration 時標 `failed`，**沿用第 7 施工關的 `legacy_no_thread` 欄位**（不改名；欄位說明改成「migration 時判定沒有 session 可接的舊列」），holder 不動；`new` 的照常啟動。第 8 施工關的重試表那一列跟著改：有 session id 就能 `retry`。
- 理由：跟 codex 完全同一條路；沿用欄位不必動第 7 施工關已驗收的程式。
- 替代方案：改名成 `legacy_no_session`（SQLite 可以 `RENAME COLUMN`，但要改第 7 施工關的程式與 fixture）；每個 instance 自己的 `XDG_DATA_HOME`＋複製 `auth.json`。
- 例子：`g12-o: session ses_4a… created`、`go (attach --session ses_4a…)`；硬殺 holder → `restart 1/3, session ses_4a… resumed`。
- 關係：第 6 施工關 H2；第 8 施工關重試表；第 7 施工關 P3。
- [ ] 使用者確認

### P12：opencode 的送達、忙碌、事件

- 問題：三級怎麼對到 opencode？怎麼確認？SSE 不重播，事件從哪補？
- 建議：
  - 閒置與 `Queue`：`prompt_async`（server 自己排，spike O3）。`Steer` → core 改 `Interrupt`。`Interrupt`：`POST /session/:id/abort` → 立刻 `prompt_async`（abort 後會 idle 兩次，第二次不當成新的一輪）。
  - 冪等：送出帶 `messageID`（[server 文件](https://opencode.ai/docs/server/)列在 prompt 的欄位），daemon 產生、存在 `messages.turn_id`；格式要求未查證（U8）。`sent`＝204；`confirmed`＝`GET /session/:id/message` 有這個 id 的 user message。
  - 事件：**opencode 的訊息歷史就是事件日誌**（第 7 施工關 P7）：cursor＝`<message id>:<part 序號>`；SSE 只更新「現在忙不忙」與觸發一次讀取。
- 理由：跟 codex 一樣以 backend 自己的歷史為準；`messageID` 讓確認不必比內容。
- 替代方案：確認改用「內容完全相同」比對（U8 不成立時的退路）；SSE 事件自己存表（SSE 會漏，spike O5）。
- 例子：`m-9 busy → prompt_async (messageID msg_agend_m9…) → sent → confirmed`。
- 關係：delivery 三級表；第 7 施工關 P5、P7。
- [ ] 使用者確認

### P13：opencode 的權限設定

- 問題：opencode 預設哪些要問？沒人回答時怎麼辦？
- 建議：
  - 官方 [permissions](https://opencode.ai/docs/permissions/)：「Most permissions default to `"allow"`. `doom_loop` and `external_directory` default to `"ask"`.」agent 在 worktree（`$AGEND_HOME/worktrees/…`，在 `--dir` 外面）工作，`external_directory` 會一直問，所以一定要改。
  - 用官方文件寫明的機制：環境變數 `OPENCODE_CONFIG_CONTENT='{"permission":"allow"}'`（CLI 文件的環境變數表；permissions 文件：「You can also set all permissions at once: `"permission": "allow"`」）。這個 JSON 會不會蓋掉你的 opencode 設定、還是合併，未查證（U12）。
  - 每 2 秒 `GET /permission`；還有被問的就 `reject`、log，並出現「需要你」`permission-rejected:<instance>`（`acknowledge`），讓你知道它為什麼停了。
  - 跟 P4 一樣，這等於不擋；agent 的 `gh` 風險在 P17。
- 理由：只用查得到的機制；自動拒絕加上「需要你」，不會靜靜卡住。
- 替代方案：`OPENCODE_PERMISSION`（第 1 次讀 CLI 文件時看到環境變數表有這一列，review 查不到，列為未查證，U12）；只開 `external_directory` 給 worktree 路徑（規則寫法未查證）。
- 例子：`opencode_live` 裡 agent 在 worktree 寫檔 → 沒有任何 `permission.asked`。
- 關係：第 7 施工關 P4 的 codex 設定是同一類；第 10 施工關 P8 的「需要你」多一種來源。
- [ ] 使用者確認

### P14：forge github 怎麼接 GitHub、怎麼選

- 問題：用 `gh` 還是直接打 REST API？哪個 team 用 GitHub？
- 建議：daemon 一律用 `gh`（你已登入的那個），經 `Runner`、每次 60 秒 timeout（D28）。repo 從 canonical checkout 的 `origin` 用 `gh repo view --json nameWithOwner` 讀一次。用哪個 forge 由 **workflow 的 submit 關卡**決定（core 已有 `forge = "github"` 欄位），merge 用同一個；team 不加欄位。
- 理由：不存 token、不另寫 HTTP 與登入；`doctor` 本來就規定查 gh 登入。
- 替代方案：daemon 用 HTTP client 打 REST、token 放 `config.toml`（多一份 secret）；team 設定 `forge`（跟 workflow 兩份）。
- 例子：workflow `gh-demo` 的 `submit(forge = "github")` → log `forge github: suzuke/agend-sandbox (gh logged in)`。
- 關係：D4、D28、D29。
- [ ] 使用者確認

### P15：head 以哪邊為準、誰 push

- 問題：agent 可以自己 push 它的 branch（第 3 施工關 T7），外面的人也可能往 PR 推。GitHub 上的 head 跟 worktree 裡的 head 不同時，以哪邊為準？
- 建議：
  - **以 worktree 裡的本機 branch 為準**，GitHub 上的必須跟它一樣才 merge。`Forge::head()` 回本機 `refs/heads/<branch>`（跟 FRG-1、FRG-4、FRG-9 一致）。
  - **同步點**：submit、每次送 merge 前、GitHub 回 409 之後，daemon `git fetch origin <branch>`，比較遠端 R 與本機 L：
    1. 遠端沒有這個 branch → `git push origin L:refs/heads/<branch>`（不 force）。
    2. R == L → 不動（agent 自己 push 過的情況）。
    3. R 是 L 的祖先（本機比較新，含 agent 在 submit 之後又 commit 的情況，FRG-5）→ fast-forward push（不 force）。
    4. 其他（遠端有本機沒有的 commit：外面有人推、agent 從別處推）→ **不覆蓋**：task 回 work，持有者收到 `origin/agend/<t>/<slug> has commits not in your worktree (<R 前 7 碼>); run git pull --ff-only (or rebase), then agend done <ticket>`。
  - 只有 D14 的 rebase 會 force：`--force-with-lease=<branch>:<剛 fetch 到的 R>`。
  - push 用你自己的 git 憑證；SSH remote 要 `SSH_AUTH_SOCK`，所以 push 這一種呼叫保留它（HTTPS 走 `gh auth setup-git`，U14）。**與已確認的第 10 施工關 P5「daemon 的 git 一律 `env_clear`」不同，請明確決定**。
  - 開 PR：`gh pr list --head <branch> --state all` 有就沿用，沒有才 `gh pr create`（body 附 `Agend-Task: <task>`）；change id＝PR 編號。
  - github 的 team，worktree 從 `git fetch origin main` 後的 `origin/main` 開。**與已確認的第 10 施工關 P4 第 2 步「從 main 的 SHA 建」不同，請明確決定**：github 的 merge 發生在遠端，本機 main 不會前進。
- 理由：checks、審查、核准都是對本機 head 做的；遠端多出來的 commit 沒人測過、沒人審過，只能退回 work 讓持有者拉進來重走。
- 替代方案：以遠端為準（daemon 把遠端 fetch 進 worktree；agent 正在改的 worktree 可能被動到）；遠端多出來的直接覆蓋（丟掉別人推的東西）。
- 例子：你核准後有人往 PR 推了一個 commit → merge 回 409 → fetch 發現情況 4 → log `t-5: origin has commits not in the worktree (9f8e…); back to work`，PR 沒 merge。
- 關係：第 3 施工關 T7；FRG-1、FRG-5；第 10 施工關 P4、P5、P7。
- [ ] 使用者確認

### P16：merge、main 前進、收尾

- 問題：怎麼保證 GitHub merge 的是核准的那個 head？已經 merge 過怎麼知道？取消時 PR 怎麼辦？
- 建議：
  - merge 前先照 P15 同步。merge：`gh api -X PUT repos/<o>/<r>/pulls/<n>/merge -f sha=<核准的 head> -f merge_method=merge -f commit_message="Agend-Task: <task>"`。官方 [Merge a pull request](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request)：`sha` 不符回 **409**；不能 merge 回 **405**。409 → 照 P15 重新同步（多半變成情況 4）；405（branch protection、要 review、只准 squash）→ `MergeFailed`＋「需要你」`merge-blocked:<task>`，寫出 GitHub 的訊息。一律 merge commit，不 squash（V1-LESSONS #6）。
  - main 前進（D14）：`git fetch origin main`，head 不含 `origin/main` → 照第 10 施工關 P7 rebase、比 patch-id，再照 P15 force-with-lease。
  - 「已經 merge 了嗎」：`gh api repos/<o>/<r>/pulls/<n>` 的 `merged`、`merge_commit_sha`（第 10 施工關 P7 說 GitHub 在本關另外處理）。
  - 收尾：merge 後、task 取消或失敗時，關 PR、刪遠端 branch。Forge trait 沒有這個方法，所以**由 daemon 在 trait 外面直接跑 `gh`**（`forge::github` 模組的一個函式，pipeline 釋放 binding 時呼叫）。
- 理由：GitHub 自己比對 SHA，等於 forge local 的 CAS；PR 狀態是 GitHub 的真相。
- 替代方案：Forge trait 加第 4 個方法 `close`（**與 D29「forge 維持 3 個方法」不同**，契約多一條、forge local 也要實作一個空的）；PR 留著不關（v1 的 137 個 branch）。
- 例子：`t-5 merge: PR #42 merged (merge commit 7c1e…); remote branch deleted`。
- 關係：D14、D29、FRG-5..8；第 10 施工關 P7、P8。
- [ ] 使用者確認（收尾選 trait 外／加方法：＿＿）

### P17：agent 手上的 gh 登入與 GitHub CI（安全決定，請選一個）

- 問題：現況是什麼、要不要處理？
- 現況：agent 在你的 uid 下跑、**沒有沙箱**（claude 見 P4、codex 是 `danger-full-access`、opencode 見 P13），PATH 上的 `gh` 用的是你的登入（`~/.config/gh`＋keychain）。所以任何 agent 都能直接 `gh pr merge 42` 或 `gh api …/merge`，**跳過 `approval(by = "human")`**；也能 `gh pr review --approve`、改 repo 設定。shim 只守 git（第 3 施工關）。照第 3 施工關的威脅模型，「好意但手滑」的 agent 很可能打出 `gh pr merge`（它以為工作的最後一步就是 merge），這屬於應該擋的那一類；真正的硬保證在 GitHub 端的 branch protection（第 3 施工關）。前一版 P13 說「不把 token 放進 checks 環境」能擋住什麼，是錯的：checks 讀得到 gh 的登入，就能 `gh auth token`。
- 選項：
  - **A. agent 的環境拿掉 gh 登入**：agent 與 checks 的環境加 `GH_CONFIG_DIR=<空目錄>`，gh 找不到登入（keychain 裡的 token 綁在 hosts 設定上，空目錄時 gh 會不會還去讀，未查證，U20）；只有 daemon 拿真的設定。代價：agent 不能用 gh 看 issue、開 PR；`command` 關卡的 `gh pr checks` 也拿不到登入 → GitHub CI 要改成 daemon 在沙箱外等（**與 D29「GitHub CI 用 `command` 關卡接」不同**）。故意去讀 keychain 仍可行（威脅模型之外）。
  - **B. shim 也包 `gh`**：`gh` 經 shim，拒絕 `pr merge`、`pr review --approve`、`api` 打到 `…/merge`／`…/reviews`／設定類路徑、`auth token`；其餘放行。agent 照常用 gh；D29 不變。改寫 `gh api` 的方法很多，擋的是手滑（第 3 施工關的定位）。改到第 3 施工關的 shim 範圍。
  - **C. 接受風險、寫明**：本關不處理；文件要求你在 GitHub 開 branch protection（main 要 1 個 review）。注意：daemon 用的也是你的帳號，required review 會讓 daemon 自己的 merge 也 405，除非你是 admin 並允許 bypass。
- 建議（請你決定）：**B**，並在 `doctor` 加一列提醒「main 沒有 branch protection」（`warn`）。B 保住 D29 與 agent 的正常 gh 用法，擋掉最常見的手滑；A 最乾淨但要改 D29。選 A 或 B 都是擴大第 3 施工關的範圍，**請明確決定**。
- GitHub CI：照 D29，workflow 寫 `gh pr checks {pr} --watch`、`timeout_ms` 寫明（例如 30 分鐘）。沙箱裡 gh 讀不讀得到 keychain 的 token 未查證（U13）；選 A 時這條路不能用。
- 理由：`gh pr merge` 是好意 agent 最可能手滑打出的一條，照第 3 施工關的判斷方法應該擋；B 擋得到又不改 D29。
- 替代方案：上面的 A、C。
- 例子：選 B 時 agent 跑 `gh pr merge 42` → `agend-shim: refused gh pr merge (merges go through agend; ask for approval instead)`，exit 1。
- 關係：第 3 施工關威脅模型、T12；D29；P4（claude 的 deny 規則是同一件事的另一層）。
- [ ] 使用者確認（選 ＿＿）

### P18：Telegram 的設定與 token（安全決定，請選一個）

- 問題：bot token、群組、誰可以操作，寫在哪？token 被 agent 讀到會怎樣？
- 設定：本關建 `config.toml`（第 9 施工關 P9：第一個讀它的施工關負責建）。你手寫：

  ```toml
  [telegram]
  chat_id = -1001234567890          # 開了 topic 的 supergroup
  allowed_user_ids = [123456789]    # 誰可以按按鈕、回答
  token_file = "secrets/telegram.token"   # 相對 AGEND_HOME；檔案必須 0600
  ```

  - allowlist 用**發訊者的 user id**，不是 chat id（群組裡每個人的 chat id 都一樣，擋不了人）。**與原頁步驟 4、[tui-and-setup](../architecture/tui-and-setup.md#安裝與設定)「自動取得 chat id 並加入 allowlist」不同，請明確決定**（第 13 關的 `telegram setup` 跟著改成加 user id）。
  - 沒有 `[telegram]` → notifier 關閉，`doctor` 那一列 `skip`。`allowed_user_ids` 空 → daemon 不收、每次開機 log 一行 error，`doctor` 失敗 exit 1（v1 #2207）。
- token 的風險：0600 只擋其他使用者。同一個 uid 的 agent（P4、P13、codex 都沒有沙箱）讀得到；checks 沙箱也讀得到（兩個平台都只擋寫）。拿到 token 就能呼叫 `getUpdates` 把你的按鈕與回覆搶走（offset 被推進，daemon 收不到），也能用 bot 的名義在群組發假訊息。
- 選項：
  - **A. token 放 `$AGEND_HOME/secrets/`（0600）＋接受 agent 讀得到**；寫進已知風險。
  - **B. A＋checks 沙箱不准讀 `$AGEND_HOME/secrets`**（macOS `deny file-read*`、Linux `--tmpfs`）：擋住 checks；agent 本身仍讀得到。**修改已確認的第 10 施工關 P6**。
  - **C. token 放 macOS keychain**（`security add-generic-password`），daemon 讀：agent 用同一個 uid 呼叫 `security find-generic-password` 會不會跳授權視窗未查證（U21）；Linux 沒有對應（仍用檔案）。
- 建議（請你決定）：**B**；再加 P4 的 deny 規則擋 claude 讀 `secrets/`（手滑層級）。要真正擋住同一個 uid 的 agent，只能讓 daemon 用另一個 uid，本關不做。token 放 `config.toml` 以外的檔，**與 [tui-and-setup](../architecture/tui-and-setup.md#設定與目錄d8)「config.toml 放 Telegram 等連線設定」字面不同，請明確決定**。
- 理由：checks 跑的是 agent 寫的程式碼，最容易無意間讀到；同一個 uid 的 agent 本身擋不住，只能寫明。
- 替代方案：上面的 A、C；token 寫在 `config.toml`（貼設定時容易外流）；環境變數（v1 #2005 變數名不一致）。
- 例子：`allowed_user_ids = []` → `agend doctor` → `FAIL telegram: allowed_user_ids is empty; every message would be dropped. Fix: add your user id to [telegram] allowed_user_ids in <home>/config.toml`，`exit=1`。
- 關係：D13；第 9 施工關 P8、P9；第 10 施工關 P6；第 13 施工關 `telegram setup`。
- [ ] 使用者確認（選 ＿＿）

### P19：新的依賴：HTTP client

- 問題：Telegram 要 HTTPS；opencode 要 HTTP＋SSE。用什麼？跑在哪？
- 建議：`agend-daemon` 加一個**阻塞式** HTTP client（建議 `ureq`＋rustls）。Telegram 的 long poll 與每個 opencode 的 SSE 各一條自己的 std thread（第 6 施工關 H7、第 7 施工關 `tungstenite` 的做法），不碰 tokio 的巢狀 `block_on`（v1 #1476）。`check-deps` 加一條：`agend-client`、`agend-shim` 不能依賴它。
- 理由：一個 crate 滿足兩邊；跟第 7 施工關一致。
- 替代方案：`reqwest`（async，依賴多）；自己寫 HTTP（testkit 的 `http.rs` 不做 TLS）；`teloxide`（v1 runtime 問題的來源）。
- 例子：`cargo xtask check-deps` 多一條規則仍 `ok`。
- 關係：D10、D11。
- [ ] 使用者確認

### P20：要改 core 的地方（Notifier 與協定）

- 問題：core 的 `Notifier` 只有 `notify(Notification{severity, title, body, task_id})`。Telegram 要分 topic、帶按鈕、解決後改訊息、收回覆，這些 core 都沒有。協定 1.4（P2）的型別也在 core。
- 建議（D27：trait 只放用到的最少方法）：
  - `Notification` 加欄位：`route`（`NeedsYou` 或 `Team(id)`）、`attention_id`、`actions`、`recap`（D37）。
  - `Notifier` 加一個方法 `resolved(attention_id, by)`：把那則訊息改成「已解決」、拿掉按鈕。
  - 收訊息**不進 trait**：Telegram 的 poller 是 daemon 的 adapter，把按鈕與回覆轉成跟 TUI 一樣的 `resolve_attention`／`answer_ask` 事件（跟 hook 走 `ingest` 同一類）。
  - 協定 1.4：`channel_attach`、`channel_message`、`channel_written`、`hook_event`（P2）；只加欄位與請求（D26）。
  - **照 D22 先改 core**：重跑第 1 施工關的 `cargo test -p agend-core`、協定相容與 golden 測試、`check-deps` 的 no-std 建置；第 2 施工關的 `FakeNotifier` 與 NTF 契約補新規則（`resolved` 後同一個 id 不再發；route 原樣送達）。
- 理由：只加用得到的兩樣；收訊息走 daemon 內部，trait 不必知道 Telegram。
- 替代方案：`Notifier` 加 `poll()` 收訊息（trait 綁 long poll 的形狀）；不改 core，Telegram 直接讀 daemon 內部型別（`notifier` 不再能對假實作測，違反 D9）。
- 例子：`contract Notifier: fake 6/6 pass`、`telegram+fake-telegram 6/6 pass`。
- 關係：D9、D11、D22、D26、D27、D37。
- [ ] 使用者確認

### P21：Telegram 發什麼、發到哪

- 問題：哪些事會吵你的手機？topic 誰建？內容太長怎麼辦？
- 建議：
  - 「需要你」topic：每個項目一則，開頭四行脈絡摘要（D37：目標、目前的決定、在問什麼、之後會發生什麼），按鈕就是它的 `actions`。`callback_data` 上限 64 bytes（[官方](https://core.telegram.org/bots/api#sendmessage)），所以存短代號、對照表在 DB。
  - team topic：只發 task merged、failed、cancelled 與逾時動作「通知」（第 10 施工關記給本關）。Info 不發。個別 instance topic 不做（D13 非預設）。
  - topic 第一次要用時才 `createForumTopic`，id 存 DB（bot 要哪個管理員權限未查證，U16）。
  - 純文字、不用 `parse_mode`（NTF-3）；超過 4096 字依序切成多則（NTF-2、NTF-4）。
  - 在 TUI 解決了 → `resolved`（P20）改訊息、拿掉按鈕。
- 理由：只有例外會叫你；按鈕跟 TUI 走同一條 `resolve_attention`。
- 替代方案：每個事件都發（吵）；啟動時一次建好所有 topic；長內容傳成檔案。
- 例子：手機「需要你」topic：`approval t-5/approve/1` ＋四行摘要 ＋ `[approve] [request changes]`。
- 關係：D13、D35、D37；NTF-1..4；第 10 施工關 P8。
- [ ] 使用者確認

### P22：Telegram 收什麼、誰能按

- 問題：手機上按按鈕、回覆文字，daemon 怎麼收？怎麼防別人？
- 建議：
  - `getUpdates` long poll，`offset` 存 DB。設了 webhook 時 `getUpdates` 不能用（官方）→ log＋`doctor` 失敗。
  - 只接受 `allowed_user_ids`（P18）；其他人忽略、每人只 log 一次。
  - 按鈕 → 以操作者身分 `resolve_attention`（同一條路，核准一樣綁 head）。`request_changes` 要理由：bot 回「請回覆這則寫理由」，你回覆的文字當 `note`。回覆請示那則 → `answer_ask`（D35）。
  - 你回的文字會原樣送到 agent：它也是 prompt injection 的入口之一（P4），只有 allowlist 裡的人能送。
  - 同一個 bot 只能一個 daemon 收（兩個會互搶更新；Telegram 回什麼錯誤未查證）；跟 v1 並行那一週用另一個 bot（ROADMAP）。
- 理由：跟 TUI 同一條權限與解決路徑。
- 替代方案：webhook（要對外開 port）；群組裡任何人都能按。
- 例子：你按 `approve` → watch 印 `attention_resolved approval:t-5/approve/1 by telegram:123456789`。
- 關係：第 8 施工關 P2、D35。
- [ ] 使用者確認

### P23：通知的可靠性

- 問題：daemon 當掉、Telegram 連不上、被限流，通知會不會掉、會不會重複？
- 建議：通知先寫 DB 的 outbox（`notifications`：id、項目、狀態、Telegram message id），再送；開機補送沒送成功的。429 照回覆等（欄位名未完整查證，U18）。送出與記錄之間當掉 → 可能重送一次（Bot API 沒有冪等 key），接受。保留 30 天：**D31 沒列這種資料，這是擴充 D31，請明確決定**（比照訊息的 30 天）。
- 理由：手機是例外的最後一道通知，不能靜靜掉；重複一則比漏一則好。
- 替代方案：只存記憶體；保留 14 天（比照事件）。
- 例子：Telegram 連不上 10 分鐘 → log `telegram: 3 notifications waiting (network)`；恢復後依序送出。
- 關係：D31；NTF-4。
- [ ] 使用者確認

### P24：G4 已讀狀態

- 問題：第 8 施工關把 G4（daemon 記已讀、TUI 與 Telegram 共用）移來本關。可是 Telegram 能告訴我們你讀了嗎？
- 建議：**不做 G4**，TUI 維持本機已讀（第 11 施工關 T4、T17）。Bot API 裡找不到「使用者讀了哪則」的資料（未完整查證，U17），Telegram 只能提供「按了、回了」＝已解決，共用的理由不成立。**與已追認的第 11 施工關 G4 與第 8 施工關 P6 不同，請明確決定**。
- 理由：做一張表、一組協定，卻沒有第二個會寫已讀的來源。
- 替代方案：照 G4 做：daemon 存已讀、TUI 寫、Telegram 不參與（多台 TUI 會同步；協定 1.4 多一組請求與事件）。
- 例子：你在 TUI 展開一項 → 不再粗體；手機上同一項照樣顯示，按了才消失。
- 關係：第 8 施工關 P6、第 11 施工關 G4、T4、T17。
- [ ] 使用者確認

### P25：其他施工關記給本關的事

- 問題：前面幾關「記給第 12 施工關」的事，哪些做？
- 建議：

  | 事 | 從哪來 | 建議 |
  |---|---|---|
  | `doctor`：backend 安裝／版本／登入、gh 登入、main 的 branch protection、Telegram | 第 9 施工關 P8 | 做；版本與錄製檔 header 不同是 `warn`；登入只用不花 token 的查法（claude、opencode 怎麼查未查證，U22） |
  | 逾時「通知」送 Telegram | 第 10 施工關 | 做（P21） |
  | claude、opencode 的清掃 | 第 7 施工關 | 做（P9、P10） |
  | 卡住偵測、usage limit、自動選忙碌等級 | 第 10 施工關 | **不做**，之後另排。**與第 10 施工關「建議移到第 12 施工關」不同，請明確決定** |
  | 你自己的 MCP server／plugin 在每個 agent 生效 | 第 7 施工關 | 不做，維持記錄 |

- 理由：本關已有四大塊；卡住與額度偵測要真 backend 的長期資料（V1-LESSONS #3）。
- 替代方案：卡住與 usage limit 一起做（每個 backend 要螢幕規則與 fixture，範圍約加一倍）。
- 例子：`agend doctor` 多幾列：`claude 2.1.282 (tested)`、`gh: logged in`、`github main: no branch protection (warn)`、`telegram: ok`。
- 關係：第 7、9、10 施工關；D24。
- [ ] 使用者確認

### P26：什麼是假的、什麼是真的

- 問題：CI 不能跑真的 claude、opencode、GitHub、Telegram。各自對什麼測？真的誰跑？
- 建議：
  - **假的**（CI）：`fake-claude`、`fake-opencode-serve`（補密碼、`messageID`）；新的 `fake-gh`（testkit bin：`pr list/create/view/checks`、`api …/merge`、`repo view`，對本機 bare repo 當 origin；409／405 可編排）；新的 `fake-telegram`（HTTP 假 Bot API，`api_base` 只在測試用的環境變數裡能改）。契約：DRV 對兩個 driver（DRV-6、DRV-9 四次開機）；FRG 對 github＋`fake-gh`；NTF 對 Telegram＋`fake-telegram`。
  - **錄製**（你核准才跑、花少量 token）：claude `resume_empty`、`startup_dialogs`；opencode `message_id`、`password`，並用新版重錄既有 5 個。
  - **真的、選做**：`claude_live`、`opencode_live`（`AGEND_REAL_CLAUDE=1`／`AGEND_REAL_OPENCODE=1`、在 `record-sandbox.sh` 裡跑；workspace 放在 `/private/tmp/agend-rec-live-*`，因為沙箱只准 claude 寫 `~/.claude/projects/-private-tmp-agend-rec-*`）。GitHub 用你的 sandbox repo；Telegram 用你的 bot。
  - 本 agent 與 verifier **都不跑**真的 claude、opencode、gh（寫入類）、Telegram。
- 理由：邏輯全部在 CI 驗，只有「真 CLI 接不接受」要真跑（第 7 施工關 P8）。
- 替代方案：CI 用真的 GitHub（要 token、會留下 PR）；不做 `fake-gh`（CI 驗不到 409／405）。
- 例子：`contract Forge: github+fake-gh 10/10 pass`。
- 關係：D9；第 2 施工關契約；第 7 施工關 P8；使用者 2026-09-25 的一致性檢查決定。
- [ ] 使用者確認

### 本關不做（明確列出）

- 卡住偵測、usage limit、自動選忙碌等級（P25）。
- 授權請求轉給人回答（claude 的 channel permission relay、opencode 的 permission、codex 的 approval）；本關一律拒絕並記 log。
- Telegram 個別 instance topic、`agend telegram setup`（第 13 施工關）。
- `ctrl+enter` 強送（P7、U2）。
- daemon 用另一個 uid 跑、讓 agent 讀不到 secret（P18）。

### 已知風險（開工時處理）

- 依賴第 9、10、11 施工關的實作：協定號（1.4）、migration 號（取當時下一個空號）、`agend send`／`doctor`／`task create`／`debug watch` 的確切輸出，開工時照實際的改這頁。
- claude 的 development channel 是 research preview，旗標與對話框可能隨版本改；driver 啟動時 log `claude --version`。
- 用你的 `~/.claude` 設定時，你自己的 hooks 也會跑，很慢會拖慢每一輪。
- agent 沒有沙箱：P4、P10、P13、P17、P18 的選項都只擋到「手滑」這一層，故意繞過不在第 3 施工關的威脅模型內。
- GitHub 的 branch protection、required review、只准 squash 都會讓 merge 405；本關只把它變成「需要你」。
- Telegram 送出與記錄之間當掉會重送一則（P23）。

**未查證的事實**（本 agent 沒有跑任何真 CLI；「怎麼查」裡不花 token 的你可以自己跑，其餘等你核准錄製或 `*_live`）：

| # | 事實 | 影響 | 怎麼查 |
|---|---|---|---|
| U1 | claude 忙碌時送到的 channel 訊息會自己排隊，這輪結束後處理 | P7（退路靠它） | **部分查證**：官方 [channels reference](https://code.claude.com/docs/en/channels-reference)「Events queue into the session and are processed in order」；2.1.282 錄製一次 |
| U2 | `ctrl+enter`（`chat:sendNow`）對 channel 送來、正在排隊的訊息有沒有作用 | P7 | 官方 [interactive mode](https://code.claude.com/docs/en/interactive-mode) 只寫「你排隊的訊息」；真跑：`claude_live` 加一段 |
| U3 | `enabledMcpjsonServers` 能不能跳過「專案 MCP server」對話框；三個對話框的畫面與單鍵 | P6 | 錄製情境 `startup_dialogs`（`agend-record startup-check claude`，不送 prompt） |
| U4 | 沒送過訊息就被殺，`--resume <id>` 的錯誤字樣與 exit | P9 | 錄製情境 `resume_empty`（不送 prompt） |
| U5 | claude 的 Bash 工具是不是 login shell、shim 在不在第一個 | P5 | `claude_live` 的 `command -v git pkill killall` |
| U6 | `--permission-mode auto` 在 daemon 起的互動 session 生不生效；我們用的模型支不支援 | P4 選 C | 官方 [permission modes](https://code.claude.com/docs/en/permission-modes) 寫了模型限制；真跑 `startup-check` 加這個旗標 |
| U7 | `transcript_path` 的 jsonl 格式（只有選替代方案才需要） | P8 | 讀一個錄製時留下的 transcript |
| U8 | opencode 的 `messageID` 格式要求 | P12 | 錄製情境 `message_id` |
| U9 | 真 `opencode serve --port 0` 會印出實際 port | P10 | `opencode serve --hostname 127.0.0.1 --port 0` 看第一行（不花 token） |
| U10 | 設了 `OPENCODE_SERVER_PASSWORD` 之後 `attach` 連得上；serve 死了 TUI 會不會自己結束 | P10 | 官方 [CLI 文件](https://opencode.ai/docs/cli/)寫 `--password` 預設讀這個變數；`opencode_live` |
| U11 | opencode 的 bash 工具用什麼 shell、shim 在不在第一個 | P5 | `opencode_live` |
| U12 | `OPENCODE_CONFIG_CONTENT` 是蓋掉還是合併你的設定、`"permission":"allow"` 之後 `GET /permission` 是否永遠空；`OPENCODE_PERMISSION` 這個變數存不存在 | P13 | 開 [CLI 文件](https://opencode.ai/docs/cli/)的環境變數表確認；錄製情境 `approval` 在這個設定下重錄 |
| U13 | macOS 沙箱裡 `gh` 讀不讀得到 keychain 的 token | P17 | 第 10 施工關做完後在沙箱裡跑 `gh auth status` |
| U14 | 你的 sandbox repo 的 remote 是 SSH 還是 HTTPS | P15 | `git -C <clone> remote -v` |
| U15 | GitHub merge API 帶 `sha` 不符回 409、不能 merge 回 405 | P16 | **已查證**：[Merge a pull request](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request) |
| U16 | `createForumTopic` 要 bot 有哪個管理員權限 | P21 | [Bot API](https://core.telegram.org/bots/api#createforumtopic)（頁面被截斷，沒讀到） |
| U17 | Bot API 沒有「已讀」資訊 | P24 | [Bot API](https://core.telegram.org/bots/api) 讀到的部分沒有；未完整查證 |
| U18 | 訊息上限 4096 字、`callback_data` 1–64 bytes；429 的 `retry_after` | P21、P23 | 前兩個**已查證**（[sendMessage](https://core.telegram.org/bots/api#sendmessage)）；`retry_after` 沒讀到 |
| U19 | macOS 上 `ps eww` 讀不讀得到同一個使用者其他程序的環境變數 | P10 | 開一個 `env FOO=bar sleep 60 &`，另一個終端 `ps eww -p <pid>`（不花 token） |
| U20 | `GH_CONFIG_DIR` 指到空目錄時 gh 是否當成沒登入（不去讀 keychain） | P17 選 A | `GH_CONFIG_DIR=$(mktemp -d) gh auth status`（唯讀） |
| U21 | 同一個 uid 用 `security find-generic-password` 讀 daemon 存的項目會不會跳授權視窗 | P18 選 C | 開工時用測試項目試 |
| U22 | claude、opencode 不花 token 的「登入了沒」查法 | P25 | 各自 `--help`、`auth` 類子命令 |

## 自動驗收（完成定義）

每段（P1）做完就跑自己那幾項；四段都過才算本關完成。

- [ ] `~/.cargo/bin/cargo test -p agend-core` 單獨通過（P20 改了 core：第 1 施工關的協定相容、golden、狀態機測試照跑）
- [ ] `~/.cargo/bin/cargo test -p agend-daemon`、`~/.cargo/bin/cargo test -p agend-testkit`、`~/.cargo/bin/cargo test -p agend` 單獨通過，包括：兩個 driver 對假 agent 跑 DRV-1..9（DRV-6、DRV-9 四次開機）；hook 在 daemon 停著時寫 spool、開機補送；Stop hook 一次取出全部佇列、`stop_hook_active: true` 不再 block；沒有 hook 60 秒改經 channel；已存在、雜湊不符的 `.mcp.json`／`CLAUDE.md` → `failed`；`delivery = inbox` 的 claude instance 不寫任何設定檔；`--resume` 找不到且沒送過 → `--session-id` 再起，送過 → `failed`；opencode 沒密碼 → 401，選 B 時沙箱裡讀密碼檔被拒；清掃只殺自己的 group；github 對 `fake-gh` 跑 FRG-1..10、P15 四種情況各一條、409／405、PR 不重開、開機用 PR 狀態判斷已 merge、收尾關 PR；選 B 時 shim 拒絕 `gh pr merge`；notifier 對 `fake-telegram` 跑 NTF 契約、切段、空 allowlist 不收、非 allowlist 忽略、outbox 補送、`resolved` 改訊息；schema fixture 與 golden 更新
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過），含 P19 的新規則
- [ ] `~/.cargo/bin/cargo xtask accept adapters` 通過，並印出下方「你親自驗收」步驟 1 的 demo
- [ ] 真 CLI 一致性檢查（必要；使用者已決定 2026-09-25）：`claude --version`、`opencode --version` 和 `crates/agend-testkit/transcripts/{claude,opencode}/` 錄製檔 header 的 `version` 相同，不同就先重錄（[RECORDER.md](../../crates/agend-testkit/RECORDER.md#重錄cli-升版時)）；`~/.cargo/bin/cargo test -p agend-testkit --test conformance` 通過
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新；[BACKEND-BEHAVIORS](../BACKEND-BEHAVIORS.md) 與 backends 分頁照 U 的結果更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出；`<t-N>` 這類尖括號是會變的值。每一步標了歸哪一段（P1），在那一段驗收時做。步驟 3、4、5 跑真的 backend、花少量 token；步驟 6、7 用你的 GitHub sandbox repo；步驟 8–10 用你的 Telegram bot。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.x.y`（目前是 `agend 0.0.0`）。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

`AGEND_HOME` 一定要設（第 13 施工關之前沒有預設值），沒設的話 `agend` 會拒絕執行。步驟 1–4 自己建暫存 home。步驟 5 起用同一個暫存 home：每個步驟的指令第一行都是 `export AGEND_HOME=<home>`，把 `<home>` 換成步驟 5 印出的路徑（同一個分頁設過一次就好，新分頁要再設）。步驟 5 起另開一個「watch 分頁」：先跑開頭那段，再 `export AGEND_HOME=<home>` 與 `agend debug watch`，之後說「watch 印」就是看這個分頁。

1. 跑 demo（全部對假的；每段各跑自己那一節）。

   **這步在驗什麼**：四段在假 agent、`fake-gh`、`fake-telegram` 上都走得通：三級忙碌與確認、四次開機補回事件、github 的四種 head 情況與 409／405、Telegram 切段與空 allowlist。錯了代表後面真的步驟看到的都不可信。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo xtask accept adapters
   ```

   應該看到：依序 `== claude`、`== opencode`、`== github`、`== telegram`、`== restart`，倒數第二行 `adapters demo: all sections passed`，最後一行 `gate 12 (adapters): checks passed`（確切輸出開工時細化；分段時還沒做的段印 `skipped (segment not built)`）。

   - [ ] 通過

2. 真 CLI 一致性檢查（必做；A、B 段）。

   **這步在驗什麼**：driver 測試用的假 claude／假 opencode 和你機器上真的 CLI 形狀一致。壞了的話，driver 對假的全綠、接上真的才出錯（v1 #1483）。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   claude --version; opencode --version
   head -1 crates/agend-testkit/transcripts/claude/one_turn.jsonl crates/agend-testkit/transcripts/opencode/one_turn.jsonl
   ~/.cargo/bin/cargo test -p agend-testkit --test conformance
   ```

   應該看到：兩個版本各自和錄製檔 header 的 `"version"` 相同；最後 `test result: ok.`。版本不同：先重錄那個 backend（`~/.cargo/bin/cargo xtask record claude --sandbox ~/Documents/Hack/AgEnD-ops/record-sandbox.sh`，或 `opencode`；會跑真的 CLI、花少量 token，見 [RECORDER.md](../../crates/agend-testkit/RECORDER.md)）再跑一次；檢查不過就改假 agent，不改錄製檔。

   - [ ] 通過

3. 真 claude 端到端（`claude_live`，A 段，約 3 個短 turn）。

   **這步在驗什麼**：假的驗不到的事：啟動對話框被規則處理（U3）、P4 選的權限模式生效（選 B 時 `gh pr merge` 被 deny 規則擋）、channel 訊息被確認、claude 跑的 `git`／`pkill`／`killall` 是 shim（U5）、holder 被 `kill -9` 後沒有 claude 留下、重起後上下文還在。錯了的話，真 claude 會卡在對話框，或 agent 的 git 繞過 shim。

   P4 選 A 或 B 的話，**先在你自己的終端機跑一次** `claude --permission-mode bypassPermissions`，接受警告後 `/exit`（官方：接受紀錄存在 user settings、只跳一次；沙箱不准寫 `~/.claude/settings*.json`，所以要在沙箱外先做）。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo build -q -p agend --bin agend && ~/.cargo/bin/cargo build -q -p agend-daemon --example claude_live
   AGEND_REAL_CLAUDE=1 ~/Documents/Hack/AgEnD-ops/record-sandbox.sh target/debug/examples/claude_live 2>/tmp/g12-claude.log | tee /tmp/g12-claude.out
   pgrep -fl "agend (holder|daemon|channel)"
   ```

   應該看到（開工時細化）：`workspace /private/tmp/agend-rec-live-…`、`startup dialogs handled: …`、`m-1 idle → channel → confirmed`、三行 `git: <H>/bin/git (the shim)`…、（選 B）`gh pr merge → denied by permission rule`、`kill -9 holder` 之後 `claude left: []`、`restart 1/3, session <S> resumed`、`m-2 → confirmed; reply mentions m-1's word: true`、最後 `claude_live: ok`；`pgrep` 沒有輸出。失敗時看 `/tmp/g12-claude.log` 最後 40 行。

   - [ ] 通過
   - [ ] 這次不做（寫進驗收紀錄）

4. 真 opencode 端到端（`opencode_live`，B 段，約 3 個短 turn，用免費模型）。

   **這步在驗什麼**：真 opencode 接受 `--port 0`、密碼、`attach --session`（U9、U10）；`messageID` 收得下、確認得到（U8）；worktree 裡寫檔不被問權限（U12）；bash 工具找到的是 shim（U11）；沒有密碼連不上。錯了的話，driver 在真 opencode 上確認不了訊息，或 agent 一直卡在權限。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo build -q -p agend --bin agend && ~/.cargo/bin/cargo build -q -p agend-daemon --example opencode_live
   AGEND_REAL_OPENCODE=1 ~/Documents/Hack/AgEnD-ops/record-sandbox.sh target/debug/examples/opencode_live 2>/tmp/g12-oc.log | tee /tmp/g12-oc.out
   pgrep -fl "opencode (serve|attach)"
   ```

   應該看到（開工時細化）：`serve ready on 127.0.0.1:<port> (auth on)`、`request without password → 401`、`session <S> created`、`m-1 idle → prompt_async → confirmed (messageID accepted)`、`permission asked: 0`、三行 `(the shim)`、`restart 1/3, session <S> resumed`、最後 `opencode_live: ok`；`pgrep` 沒有輸出。

   - [ ] 通過
   - [ ] 這次不做（寫進驗收紀錄）

5. 三個真 backend 互傳訊息（B 段；A、B 都完成才做）。

   **這步在驗什麼**：同一個真 daemon 上，claude、codex、opencode 各收到一則、各自回覆，三則都到 `confirmed`（里程碑「三個 backend」）。錯了的話某個 backend 的訊息會停在 `sent` 或 `queued`。

   第一個分頁：

   ```bash
   export AGEND_HOME="$(mktemp -d /tmp/g12.XXXX)" && echo "export AGEND_HOME=$AGEND_HOME"
   agend daemon
   ```

   記下第一行印出的 `export AGEND_HOME=…`，照上面的說明開 watch 分頁。第三個分頁（先跑開頭那段）：

   ```bash
   export AGEND_HOME=<home>
   agend instance add g12-c claude; agend instance add g12-x codex; agend instance add g12-o opencode
   agend send g12-c "Reply with exactly: C-OK"; agend send g12-x "Reply with exactly: X-OK"; agend send g12-o "Reply with exactly: O-OK"
   ```

   應該看到：三個 `accepted`；watch 印三行 `… confirmed`（確切字樣開工時細化，照第 9 施工關的輸出）。

   - [ ] 通過

6. forge github：在你的 sandbox repo 跑完整流水線（C 段）。

   **這步在驗什麼**：真的 GitHub 上：daemon 只推 `agend/…`、開一個 PR、`gh pr checks` 關卡等到 CI、你核准後帶著核准的 SHA merge，遠端 branch 被刪（P15、P16）。錯了的話不是 merge 不了，就是 merge 了沒核准的 head。

   先準備：一個你自己的 GitHub repo `<owner>/<repo>`（有一個會通過的 GitHub Actions），clone 到本機 `<clone>`（這是 team 的 canonical checkout）。第三個分頁：

   ```bash
   export AGEND_HOME=<home>
   ~/.cargo/bin/cargo run -q -p agend-daemon --example adapters_probe -- github-setup <clone>
   agend task create --team g12gh --role dev --workflow gh-demo "hello"
   ```

   （`github-setup` 建 team `g12gh`、兩個第 10 施工關的 `fake-worker`（dev、reviewer）、workflow `gh-demo`：work → submit(github) → `gh pr checks {pr} --watch` → review → approve(human) → merge；開工時細化。）第二行印出 task id `<t-N>`。等 watch 印 `attention_required approval:<t-N>/approve/1`，然後：

   ```bash
   gh pr list --repo <owner>/<repo> --head agend/<t-N>/hello --state all --json number,url
   ~/.cargo/bin/cargo run -q -p agend-client --example client_probe -- resolve approval:<t-N>/approve/1 approve
   gh pr view --repo <owner>/<repo> <pr> --json state,mergeCommit,headRefName
   git -C <clone> ls-remote origin "refs/heads/agend/*"
   ```

   `<pr>` 是第一行印出的 `number`。應該看到：`"state":"MERGED"`；merge commit 的訊息有 `Agend-Task: <t-N>`；`ls-remote` 什麼都不印。

   - [ ] 通過

7. 故意弄壞：核准之後、merge 之前，有人從外面往 PR 推了一個 commit（C 段）。

   **這步在驗什麼**：GitHub 只 merge 你核准的 SHA；遠端多了本機沒有的 commit 時，daemon 不 merge、也不覆蓋，而是退回 work（P15、P16）。錯了的話，核准後被推進來的任何東西都會跟著 merge，或被 daemon 默默蓋掉。

   照步驟 6 再開一個 task（`"second"`），watch 印 `approval:<t-M>/approve/1` 時**先不要按**。從**另一份 clone**（不是 `<clone>`：那個 branch 已經被 daemon 的 worktree checkout 著）推一個 commit：

   ```bash
   git clone -q "$(git -C <clone> remote get-url origin)" /tmp/g12-other
   git -C /tmp/g12-other switch -q agend/<t-M>/second
   echo extra > /tmp/g12-other/extra.txt && git -C /tmp/g12-other add extra.txt && git -C /tmp/g12-other commit -qm extra && git -C /tmp/g12-other push -q origin HEAD
   ~/.cargo/bin/cargo run -q -p agend-client --example client_probe -- resolve approval:<t-M>/approve/1 approve
   ```

   應該看到：watch 印 `<t-M> merge: GitHub 409 …`，接著 `<t-M>: origin has commits not in the worktree (…); back to work`；`gh pr view --repo <owner>/<repo> <pr2> --json state` 是 `OPEN`（`<pr2>` 照步驟 6 第一行的查法）。收尾：`agend task cancel <t-M>` → watch 印 `<t-M> cancelled`、PR 變 `CLOSED`、遠端 branch 不見；`rm -rf /tmp/g12-other`。

   - [ ] 通過

8. Telegram：手機收到「需要你」，在手機上核准（D 段）。

   **這步在驗什麼**：「需要你」推到手機、手機上按的核准走的是跟 TUI 同一條路，TUI 那邊跟著變成已解決（P21、P22）。錯了的話你離開電腦就不知道有事卡著，或手機按了沒用。

   先準備：一個 bot（token 存成 `<home>/secrets/telegram.token`，`chmod 600`）、一個開了 topic 的 supergroup（bot 是管理員）、你的 user id。照 P18 的格式寫 `<home>/config.toml`，重開 daemon（第一個分頁 Ctrl-C 後 `agend daemon`）。第三個分頁開一個會停在人工核准的 task（開工時細化：用第 10 施工關 `pipeline_probe setup` 建的 `demo` workflow，或步驟 6 的 `gh-demo`）。

   應該看到：手機的「需要你」topic 出現 `approval <t-K>/approve/1`、四行摘要與兩個按鈕；按 `approve` → watch 印 `attention_resolved approval:<t-K>/approve/1 by telegram:<你的 id>`；手機那則變成「已解決」、按鈕消失；team topic 出現 `<t-K> merged`。

   - [ ] 通過

9. Telegram：在手機上回答請示（D 段）。

   **這步在驗什麼**：請示是對話（D35）：手機上回覆的文字送到 agent，那一項變成已解決（P22）。錯了的話你在手機上回了，agent 還在等。

   第三個分頁（開工時細化：讓假 agent 發一則請示）：

   ```bash
   export AGEND_HOME=<home>
   ~/.cargo/bin/cargo run -q -p agend-daemon --example adapters_probe -- ask g12-c "Pick a color?"
   ```

   在手機上**回覆**那則訊息寫 `blue`。應該看到：watch 印 `ask <id> answered by telegram:<你的 id>: blue`；那一項從「需要你」消失；送給 agent 的訊息 `confirmed`。

   - [ ] 通過

10. 故意弄壞：Telegram 的 allowlist 是空的（D 段）。

    **這步在驗什麼**：設定錯了不會靜靜不動：`doctor` 失敗並給修正方式，daemon 開機也大聲說（v1 #2207、P18）。錯了的話你以為手機會收到，其實全部被丟掉。

    把 `<home>/config.toml` 的 `allowed_user_ids` 改成 `[]`：

    ```bash
    export AGEND_HOME=<home>
    agend doctor; echo "exit=$?"
    ```

    應該看到：Telegram 那一列 `FAIL`：allowlist 是空的、所有訊息會被丟棄，附修正方式（在 `<home>/config.toml` 的 `allowed_user_ids` 加回你的 user id）；`exit=1`。重開 daemon 時 log 有一行 `telegram: allowed_user_ids is empty; not receiving`。改回來後 `doctor` 那一列 `ok`。第 13 施工關之後，修正方式改成指向 `agend telegram setup`。

    - [ ] 通過

11. 收尾（每段最後都做）。

    **這步在驗什麼**：什麼都不留（第 6 施工關的孤兒巡查照舊）。

    操作：各分頁 Ctrl-C，然後在設了 `AGEND_HOME` 的分頁 `~/.cargo/bin/cargo run -q -p agend-daemon --example adapters_probe -- teardown`（開工時細化）。

    應該看到：`pgrep -fl "agend holder g12-"`、`pgrep -fl "opencode (serve|attach)"` 都不印；`<home>` 不見了；sandbox repo 上沒有 `agend/*` branch、沒有 open 的 PR（`gh pr list --repo <owner>/<repo>` 空的）。

    - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-28 第 1 輪 review REFUTED（5 HIGH、10 MEDIUM、4 LOW）後改寫：P 項拆成 P1–P26；bypass 獨立成 P4（附官方原文、auto 與 deny 規則）；opencode 密碼在 macOS 擋不住的事照實寫、改成選項（P10）；新增 P17（agent 手上的 gh 登入）；P15 定義 head 以本機為準與四種情況；token 風險（P18）；core 的 Notifier 與協定改動（P20）；補上「請明確決定」的標記；步驟 3、6、7、8、9 改成照抄能跑；U 表改成 U1–U22。
- 2026-09-28 開工前提案 P1–P21 寫定（draft PR #138，branch `docs/gate-12-proposal`），待使用者逐題確認；「你親自驗收」改成 11 步；狀態改為提案中。
- 2026-09-25 使用者決定：真 CLI 一致性檢查（錄製器 + `tests/conformance.rs`）列為必要完成條件（`feat/backend-recorder`）。

## 下一步

```bash
cat docs/gates/gate-12-adapters.md
~/.cargo/bin/cargo xtask accept adapters
```
