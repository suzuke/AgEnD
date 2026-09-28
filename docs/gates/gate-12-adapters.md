# 第 12 施工關：其餘 adapter（`adapters`）

> **TL;DR**
> - claude driver、opencode driver、forge github、Telegram 通知；分四段做，各段自己驗收（P1）。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：開工前提案 P1–P34 等你逐題確認（每題只有一個決定；P4、P12、P22、P23、P25 是安全決定，沒有預設答案）；確認後 merge 這份提案，等第 9–11 施工關完成再開工。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**提案中**（2026-09-28）：開工前提案 P1–P34 寫定，等你確認（第 2 輪 review REFUTED 後改寫，每題拆成一個決定）。依賴：第 7 施工關（送達、`messages` 表、`sh` 包裝、清掃）已驗收；第 9 施工關（CLI、`agend send`、`doctor`、協定 1.2）、第 10 施工關（流水線、forge local、「需要你」的來源、沙箱、協定 1.3）提案已確認、還沒實作；第 11 施工關 B 段進行中。這些施工關做完時跟這裡的理解不同，改這頁，不改它們。

## 範圍

- A 段 claude driver（D16）：P2–P10
- B 段 opencode driver：P11–P15
- C 段 forge github：P16–P23
- D 段 Telegram：P24–P31
- 共用：core 的改動（P27，任何一段開工前先做）、其他施工關記給本關的事（P32、P33）、什麼是假的什麼是真的（P34）

### 從其他施工關帶來的筆記（已併進提案）

- **G4 已讀狀態**（[gate-08-client P6](gate-08-client.md#p6真-daemon-本關做哪些請求g4-移走)）→ P31。
- **claude Bash 工具的 PATH 重排**（[gate-03-shim 已知限制](gate-03-shim.md#已知限制)、第 7 施工關 P4、K8）→ P5、U5。
- **使用者提供、未查證：claude 按 ctrl+enter 可以把訊息「強送」給正在工作的 agent**（2026-09-26）→ P8、U2。官方文件有這個鍵，但說的是「你在 TUI 打字排隊的訊息」，對 channel 送來的訊息有沒有用沒寫。
- **holder 被 `kill -9` 後其他 backend 的清掃**（第 7 施工關「已知風險」）→ P10、P11。
- **第 10 施工關移來**：forge github、GitHub CI、逾時「通知」送 Telegram → P16–P23、P28；卡住偵測／usage limit／自動選忙碌等級 → P33；假 agent（`fake-worker`，backend 登記成 `claude`、`delivery = inbox`）→ P3。
- **第 9 施工關移來**：`doctor` 的 backend、gh、Telegram 幾列；第一個讀 `config.toml` 的施工關負責建它 → P24、P25、P32。

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子 · 跟既有決定的關係。**每題只有一個決定**。跟既有決定不同的地方標「**與已確認／已追認的 X 不同，請明確決定**」。**安全決定**（P4、P12、P22、P23、P25）列出選項與我的建議，請你選。

文件已定、這裡不重問的：claude 用互動 TUI＋hooks＋channel、忙碌時用 Stop hook 排隊、中斷＝`Esc` 後立刻送（D16）；opencode 用 `serve` 的 HTTP＋SSE、SSE 沒有重播、授權以 `GET /permission` 為準（[backends/opencode.md](../backends/opencode.md)）；送達狀態四個、只有一套冪等、推送帶完整內容、不支援插入就改中斷（[delivery](../architecture/delivery.md)、`policy::busy::effective_level`）；附屬程序由 holder 持有、用 `sh` 包裝放在同一個 process group（第 7 施工關 P2）；daemon 先建 session、先存 DB 再讓 TUI 起來（第 7 施工關 P3）；GitHub CI 用 `command` 關卡接（D29）；Telegram 一個「需要你」topic＋每個 team 一個（D13）；「需要你」與 `resolve_attention` 只收操作者（第 8 施工關 P2、P5；第 10 施工關 P8）；daemon 的 git 經 `Runner`、`-c core.hooksPath=/dev/null`（第 10 施工關 P5）；checks 在寫入沙箱裡跑（第 10 施工關 P6）；agent 可以 push 自己綁定的 branch（第 3 施工關 T7）；shim 的威脅模型：防好意但會犯錯的 agent、不防故意繞過，硬保證是 hook＋forge 端的 branch protection（[第 3 施工關](gate-03-shim.md)）；client 在 TUI 打的字經 daemon 轉給 holder，client 不直接連 holder（第 8 施工關）；真 CLI 一致性檢查是必要完成條件（使用者 2026-09-25）。

事實來源：[BACKEND-BEHAVIORS](../BACKEND-BEHAVIORS.md)、[backends/](../backends/claude-code.md)、`crates/agend-testkit/transcripts/{claude,opencode}/` 的錄製檔（claude 2.1.282、opencode 1.18.31）、官方文件（附 URL）。標 **未查證** 的集中在「未查證的事實」表（U1–U22）。本 agent 沒有跑任何真的 claude、opencode、gh 或 Telegram。

### P1：本關怎麼分段

- 問題：四樣東西彼此幾乎不相干，全部做完才驗收，一次要審的量太大。要不要分段？
- 建議：同一個施工關、分四段，**每段一個 PR、各自驗收**：A claude、B opencode、C github、D Telegram（範圍那一節的題號）。P27（core）在第一段開工前先做。四段彼此不依賴，順序任意；「你親自驗收」步驟 5（三個真 backend）要 A、B 都完成，歸 B 段。四段都驗收完，第 12 施工關才算完成。
- 理由：前面每關都要審 3–4 輪；分段後每輪只看一塊。
- 替代方案：拆成 12a–12d 四個施工關（ROADMAP 與 D22、D24 的「13 個施工關」要改）；不分段。
- 例子：A 段 PR `feat/gate-12a-claude` 驗收完 merge，進度紀錄寫「A 段完成」，狀態維持「實作中」直到四段都完成。
- 關係：D22 不變，分段只在關內。
- [ ] 使用者確認

### P2：claude 的訊息與 hook 怎麼到 daemon

- 問題：claude 的訊息從 channel（claude 自己起的 MCP server）進去，狀態從 hooks（claude 跑的指令）出來。這兩個程式是什麼？daemon 不在時怎麼辦？
- 建議：兩個都是 `agend` 的內部子命令：`agend channel --instance <id>`（MCP stdio server）、`agend hook <事件名>`。都走 client 協定 1.4 的新請求（P27）。daemon 不在時 hook 寫到 `$AGEND_HOME/spool/hooks/<instance>/<序號>.json`，Stop 回 `{}`（不 block），daemon 開機照序號補送；channel 每秒重連。hook 的 timeout 設 10 秒（官方預設 600 秒，[hooks](https://code.claude.com/docs/en/hooks)）。訂閱的 hook：`SessionStart`、`UserPromptSubmit`、`PreToolUse`、`PostToolUse`、`PermissionRequest`、`Stop`、`SessionEnd`。
- 理由：同一個 binary、同一套有版本的協定（ARCHITECTURE 程序模型第 4 條）；hooks 丟掉就一直被當成 busy（V1-LESSONS #9）。
- 替代方案：bridge 另開私有協定（多一套要管版本）；hook 打 HTTP（daemon 多開 server）；daemon 不在時丟掉（v1 的坑）。
- 例子：`agend send g12-c "hi"` → bridge 寫 `notifications/claude/channel {content:"From: …\n\nhi", meta:{delivery_id:"m-7"}}` → `sent`。
- 關係：D7 不變（MCP 只用在 claude 要求的 channel）。
- [ ] 使用者確認

### P3：claude 的設定檔放哪、假 agent 怎麼辦

- 問題：hooks、`.mcp.json`、「訊息來源說明」寫在哪？會不會蓋到你的檔案？第 10 施工關登記成 `claude` 的假 agent 要不要也套？
- 建議：hooks 用 `--settings $AGEND_HOME/claude/<id>/settings.json` 帶進去。`.mcp.json` 與 `CLAUDE.md` 一定要在工作目錄（`--mcp-config` 會讓 `server:<name>` 找不到），寫進 workspace；「是不是我們寫的」看 DB 記的 sha256，存在但不是我們寫的 → 不覆蓋、instance `failed`。用你自己的 `~/.claude` 設定，daemon 不寫 `~/.claude*`。只套在 `delivery = push` 的 claude；`delivery = inbox`（`fake-worker`）不寫檔、不加旗標、不建 driver。
- 理由：不動你的 repo；`.mcp.json` 是 JSON 不能放註解，雜湊不必改內容就能辨認。
- 替代方案：隔離的 `CLAUDE_CONFIG_DIR`（要重新登入，BLOCKED）；`--append-system-prompt` 代替 CLAUDE.md（效果未查證）；直接覆蓋。
- 例子：`--dir ~/proj` 而 `~/proj/CLAUDE.md` 是你的 → `g12-c failed: CLAUDE.md exists and was not written by agend`。
- 關係：D16（CLAUDE.md 說明來源）照做。
- [ ] 使用者確認

### P4：claude 的權限模式（安全決定，請選一個）

- 問題：claude 沒有人在旁邊按「允許」。用哪個權限模式？
- 官方摘要（[permission modes](https://code.claude.com/docs/en/permission-modes)）：bypass「只該在隔離環境（container、VM）用」、「對 prompt injection 沒有保護」，官方建議改用 auto；deny 規則在 bypass 下仍然有效；auto 用另一個模型審查每個動作，預設就擋「沒人核准的 PR merge」，但「不保證安全」，而且要 Opus／Sonnet 4.6 以上。
- 風險：agent 在你的 uid、你的網路、你的 gh 登入下跑；進來的文字（其他 agent 的 `agend send`、你在 Telegram 回的話、它讀到的檔案與網頁）都可能帶 prompt injection。
- 選項：
  - A. bypass：什麼都不擋，跟 codex 的 full-access 一樣。
  - B. bypass ＋ deny 規則：擋 `Bash(gh pr merge*)`、`Bash(gh api*merge*)`、`Bash(gh pr review*--approve*)`，以及讀 secret 的路徑（daemon 產生展開後的絕對路徑，寫成 `Read(//<絕對路徑>/**)`；官方：單一斜線開頭是相對於設定檔，`$AGEND_HOME` 不會展開）。字串比對，擋手滑、不擋故意。
  - C. auto：擋得最多；每個動作多一次模型判斷（慢、花錢）；還是可能跳提示（靠 P7 轉成「需要你」）；從旗標帶 `auto` 能不能用未查證（U6）。
  - D. manual：每一步都停下來等人，本關沒有回答的 UI，實際上不能用。
- 建議（請你決定）：B。
- 理由：不增加卡住的機會，又用官方保證有效的 deny 規則擋住最直接的風險（agent 自己 merge、讀 secret）。
- 替代方案：A、C、D。
- 例子：選 B 時 agent 跑 `gh pr merge 42` → claude 回 `denied by permission rule`。
- 關係：第 7 施工關 P4（codex full-access）是同一類決定。選 A、B 時第一次啟動會跳 bypass 警告（官方：接受後存在 user settings、只跳一次），由你自己先接受一次（步驟 3）。
- [ ] 使用者確認（選 ＿＿）

### P5：claude、opencode 跑的指令找不找得到 shim

- 問題：macOS 的 login zsh 會把系統路徑排到 shim 前面（第 7 施工關 P4）。claude、opencode 的 shell 工具也這樣嗎？
- 建議：第 7 施工關 K8 的 `ZDOTDIR` 也給 claude 與 opencode，先做，再用 `claude_live`、`opencode_live` 的 `command -v git pkill killall` 驗（U5、U11）。
- 理由：shim 是 git／pkill／killall 唯一的防護；不是 login shell 時 `ZDOTDIR` 沒作用也沒壞處。
- 替代方案：等 U5、U11 查完再決定；只給 claude。
- 例子：`claude_live` 印 `git: <H>/bin/git (the shim)`。
- 關係：**與已追認的第 7 施工關 K8「`ZDOTDIR` 只給 codex」不同，也再改一次第 6 施工關 H3 白名單，請明確決定**。
- [ ] 使用者確認

### P6：claude 的啟動對話框

- 問題：信任資料夾、專案 MCP server、development channels 警告，沒人按就卡住。
- 建議：先試 `settings.json` 的 `enabledMcpjsonServers: ["agend"]` 避開 MCP 對話框（U3）；其他用螢幕規則檔（[delivery](../architecture/delivery.md) 第 3 層：比對畫面 → 單一按鍵），每條附真畫面 fixture。信任對話框要明確選「Yes」，不能按 Enter（陷阱 6）。bypass 警告不自動按（P4 由你先接受）；其他規則認不出的畫面交給 P7。
- 理由：答案是資料，不寫進你的設定；責任聲明應該由你按。
- 替代方案：預寫 `~/.claude.json` 的信任狀態（改你的檔案）；規則也自動接受 bypass 警告。
- 例子：log `g12-c: startup dialog "trust" answered (rule claude/trust, key 1)`。
- 關係：delivery.md 第 3 層；BACKEND-BEHAVIORS 陷阱 6。
- [ ] 使用者確認

### P7：「卡在未知提示」由本關做（最小版）

- 問題：P4 選 C、P6 認不出的畫面、沒接受的 bypass 警告，都要靠 delivery.md 第 4 層「卡在未知提示 → 請人處理」。這一層沒有任何施工關負責，第 10 施工關 P8 的「需要你」來源表也沒有它。
- 建議：本關做最小版：instance 啟動後 60 秒還沒進 ready（claude：`SessionStart` 沒來；opencode：`/global/health` 不通）而且畫面 10 秒沒變 → 出現「需要你」`stuck-prompt:<instance>`，附當下畫面文字；`actions` 是 `retry`（重起）。你在 TUI 看畫面、自己打字處理（第 11 施工關 B 段的 `terminal_input`）。畫面存成候選 fixture。「人的回答以單一按鍵送出」與自動學規則不做。
- 理由：沒有它，P4、P6 的「卡住」會變成靜靜不動（V1-LESSONS #14 同類）。
- 替代方案：留給之後的施工關（本關的 claude 可能靜靜卡在對話框）；做完整版（含按鍵回答與規則學習）。
- 例子：bypass 警告沒接受 → 「需要你」`stuck-prompt:g12-c — no ready after 60 s; screen: "…accept responsibility…"`。
- 關係：**修改已確認的第 10 施工關 P8「需要你」來源表（加一種 `stuck-prompt`），請明確決定**；delivery.md 第 4 層。
- [ ] 使用者確認

### P8：claude 的三級忙碌與忙／閒

- 問題：queue、steer、interrupt 怎麼落到 claude？daemon 怎麼知道它忙不忙？你在 TUI 自己按 `Esc` 時沒有任何 hook，daemon 會一直以為它在忙，排隊的訊息等不到 Stop。
- 建議：
  - 忙：`UserPromptSubmit`、`PreToolUse`、`PostToolUse`（工具 hook 當心跳，[backends/claude-code](../backends/claude-code.md) 已觀察到會觸發）。閒：`Stop`（沒被 block）、`SessionStart`。
  - `Esc`：daemon 自己送的，或經 daemon 轉給 holder 的你的按鍵（TUI 打的字都經過 daemon）→ 3 秒內沒有任何 hook 就記成閒。之後排隊的訊息照「閒置」經 channel 送（D16 本來就規定閒置走 channel、中斷後不等 Stop）。
  - `Queue`（忙）：放 daemon 的佇列，下一個 Stop（`stop_hook_active: false`）全部取出、合成一個 reason 回 `block`。`Steer` → core 改 `Interrupt`；`Interrupt`：送單一 `Esc` → 立刻經 channel 送。
  - **忙碌時絕不經 channel 送**（spike C1：送達了、`UserPromptSubmit` 也觸發，但模型可能不做，而且看不出來；前一版的「60 秒退路」已拿掉）。
  - ctrl+enter 不用（U2；替代的 `Ctrl+X Ctrl+S` 是兩個鍵）。
- 理由：只用 hook 與 daemon 自己看得到的按鍵，不讀畫面；完全在 D16 裡面。
- 替代方案：讀畫面上的「Interrupted」判斷閒置（**與 delivery.md「螢幕只認 hard gate」不同**）；不處理你按 `Esc` 的情況（排隊的訊息卡到它下次自己忙完）。
- 例子：你在 TUI 按 `Esc` 打斷 g12-c，`m-q` 在排隊 → 3 秒沒有 hook → log `g12-c: interrupted by operator; idle` → `m-q` 經 channel → `confirmed`。
- 關係：D16 照做；D30 去抖動只給畫面用。
- [ ] 使用者確認

### P9：claude 的送達確認、冪等、事件

- 問題：`sent`、`confirmed` 各在什麼時候成立？daemon 不在時的事件（DRV-6）從哪補？
- 建議：`sent`＝bridge 寫進 claude，或 Stop hook 的 block 已回出去。`confirmed`＝channel 送的看 `UserPromptSubmit` 的 `delivery_id="<id>"`（錄製 `one_turn` 已證實）；Stop hook 送的看下一個 `stop_hook_active: true` 的 Stop（錄製 `busy`）。因為忙碌時不經 channel（P8），`UserPromptSubmit` 出現時 claude 是閒置、這則是新的一輪。冪等照第 7 施工關 P5，當掉後不重送。事件：新表 `driver_events`（`seq` 當 cursor），保留 14 天（同事件，D31）。migration 取開工時的下一個空號（目前是 `0005`，但第 9、10 施工關可能先用掉，取當時下一個空號）。
- 理由：hook 是 claude 唯一的結構化事件，自己存一份最簡單。
- 替代方案：讀 `transcript_path` 當事件日誌（綁檔案格式，U7）；開機重送（可能重複一輪，違反 DRV-9）。
- 例子：daemon 停著時 Stop hook 送出排隊的訊息（寫進 spool）→ 開機補送 → `m-q confirmed`。
- 關係：第 7 施工關 P5、P7。
- [ ] 使用者確認

### P10：claude 空 session 找不到時重起

- 問題：第 6 施工關 H1：「`Spawn` 被確認」不等於 claude 已把 session 存檔；沒送過訊息就死掉，`--resume <id>` 可能找不到（U4）。
- 建議：`--resume` 很快結束、畫面有「找不到 session」類的字，**而且**從沒有 `sent` 以上的訊息 → 同一個 id 改用 `--session-id` 再起一次；其他情況 `failed`。清掃也在本關做：標記是 argv 裡 `--session-id`／`--resume` 後面那個元素，或 `agend channel --instance <id>`。
- 理由：空 session 沒有上下文可丟；跟 2026-09-26 對 codex 核准的例外同理。
- 替代方案：一律 `failed`（第一則訊息前死掉也要你處理）。
- 例子：第一次起來就被 `kill -9` → `started again with --session-id 3f2a…`。
- 關係：**與已確認的第 6 施工關 P6「絕不自動全新啟動」不同，請明確決定**。
- [ ] 使用者確認

### P11：opencode 怎麼放進 holder、密碼經環境變數

- 問題：`opencode serve` 跟 TUI 怎麼放進 holder？daemon 怎麼知道 port？
- 建議：照第 7 施工關 P2 的 `sh` 包裝：背景 `opencode serve --hostname 127.0.0.1 --port 0`（daemon 從 log 讀實際 port，U9）→ `/global/health` 就緒 → 建／接 session（P13）→ `$GO` → `exec opencode attach http://127.0.0.1:<port> --session <id> --dir <workspace>`（[CLI 文件](https://opencode.ai/docs/cli/)）。每個 instance 一個隨機密碼，經 `OPENCODE_SERVER_PASSWORD` 給 serve 與 attach（官方：basic auth，attach 預設讀這個變數）。清掃標記：argv 裡 `--session` 後面那個元素。
- 理由：同一套包裝與清掃；密碼擋住同機器上其他人與沒拿到密碼的程式（macOS 上 checks 拿得到，見 P12）。
- 替代方案：unix socket（官方沒寫 serve 支援）；密碼放 argv（`ps` 看得到）；不設密碼。
- 例子：`curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:<port>/session` → `401`。
- 關係：第 7 施工關 P2；**多一個環境變數是修改第 6 施工關 H3 白名單，請明確決定**。
- [ ] 使用者確認

### P12：checks 能不能連到本機的服務（安全決定，請選一個）

- 問題：第 10 施工關的 checks 在沙箱裡跑 agent 寫的程式碼，沙箱允許 IP 網路。checks 能不能連上 opencode（或任何聽 loopback 的服務）叫它在沙箱外做事？
- 現況：密碼**在 Linux 擋得住**（沙箱用 tmpfs 蓋掉 `$AGEND_HOME/run`），**在 macOS 擋不住**：沙箱只擋寫、不擋讀，checks 往上兩層就是 `$AGEND_HOME`，讀得到密碼檔；`ps eww` 可能讀得到環境變數（U19）。
- 選項：
  - A. 接受 macOS 的限制，寫進已知風險。
  - B. macOS 沙箱加 `(deny file-read* (subpath "$AGEND_HOME/run"))`：擋住讀密碼檔；U19 若成立仍擋不住。
  - C. 沙箱擋掉 loopback（macOS `(deny network-outbound (remote ip "localhost:*"))`；Linux 做法開工時查）：擋掉所有本機服務，但需要本機資料庫的 checks 跑不了。
- 建議（請你決定）：B，並把 U19 查清楚。
- 理由：B 只多一行規則就把 macOS 補到接近 Linux；C 會擋到正常的 checks。
- 替代方案：A、C。
- 例子：選 B 時 checks 裡 `cat $AGEND_HOME/run/holders/*.opencode.pw` → `Operation not permitted`。
- 關係：選 B、C 都**修改已確認的第 10 施工關 P6，請明確決定**。
- [ ] 使用者確認（選 ＿＿）

### P13：opencode 的 session 與舊的 instance

- 問題：第 6 施工關說 opencode「沒有 session id，死了就 `failed`」。session 誰建、存哪？第 12 關之前建的怎麼辦？
- 建議：daemon 在 serve 就緒後 `POST /session`，先存 `instances.session_id` 再寫 `$GO`；重起用同一個資料目錄 `attach --session <id>`（spike O4）。用你原本的資料目錄與 plugin（不設 `XDG_*`、不加 `--pure`）。之前建的、`session_started = 1` 而沒有 session id 的列 → migration 時標 `failed`，沿用第 7 施工關的 `legacy_no_thread` 欄位（不改名），holder 不動；第 8 施工關的重試表那一列跟著改。
- 理由：跟 codex 同一條路；沿用欄位不動已驗收的程式。
- 替代方案：改名 `legacy_no_session`（要改第 7 施工關的程式與 fixture）；每個 instance 自己的資料目錄（登入要逐個處理）。
- 例子：硬殺 holder → `restart 1/3, session ses_4a… resumed`。
- 關係：第 6 施工關 H2；第 7 施工關 P3；第 8 施工關重試表。
- [ ] 使用者確認

### P14：opencode 的送達、忙碌、事件

- 問題：三級怎麼對到 opencode？怎麼確認？SSE 不重播，事件從哪補？
- 建議：閒置與 `Queue` → `prompt_async`（server 自己排）；`Steer` → `Interrupt`；`Interrupt` → `abort` 後立刻 `prompt_async`（abort 後 idle 兩次，第二次不算新的一輪）。送出帶 `messageID`（[server 文件](https://opencode.ai/docs/server/)），`confirmed`＝`GET /session/:id/message` 有這個 id。事件：訊息歷史就是事件日誌，cursor＝`<message id>:<part 序號>`；SSE 只看忙不忙。
- 理由：以 backend 自己的歷史為準（第 7 施工關 P7）；`messageID` 讓確認不必比內容。
- 替代方案：確認改比內容（U8 不成立時的退路）；SSE 自己存表（會漏）。
- 例子：`m-9 busy → prompt_async (messageID …) → sent → confirmed`。
- 關係：delivery 三級表；第 7 施工關 P5、P7。
- [ ] 使用者確認

### P15：opencode 的權限設定

- 問題：官方 [permissions](https://opencode.ai/docs/permissions/)：「Most permissions default to `"allow"`. `doom_loop` and `external_directory` default to `"ask"`.」agent 在 worktree（`--dir` 外面）工作，會一直被問。怎麼設？
- 建議：只放行 worktree：`OPENCODE_CONFIG_CONTENT='{"permission":{"external_directory":{"<AGEND_HOME 展開後>/worktrees/**":"allow"}}}'`（官方 CLI 文件的環境變數表有 `OPENCODE_CONFIG_CONTENT` 與 `OPENCODE_PERMISSION`；寫法照官方 `"~/projects/personal/**": "allow"` 的範例）。官方 [config](https://opencode.ai/docs/config/)：設定是合併、只蓋衝突的鍵，所以你其他的設定與預設的「拒絕讀 `.env`」應該保留（合併到 `permission` 底下哪一層未查證，U12）。還被問的（例如 `doom_loop`）→ `reject`、log、「需要你」`permission-rejected:<instance>`。
- 理由：開最小的一個口，不動其他預設。
- 替代方案：`"permission":"allow"`（最省事，但會蓋掉「拒絕讀 `.env`」的預設）；`opencode --auto`（官方：TUI 與 `run` 的旗標，「Auto-approve permissions that are not explicitly denied」；`attach` 有沒有、對 serve 生不生效未查證）。
- 例子：agent 在 worktree 寫檔 → 沒有 `permission.asked`；讀 `.env` → 照預設被擋。
- 關係：第 10 施工關 P8 的「需要你」多一種來源（`permission-rejected`，跟 P7 一起加）。
- [ ] 使用者確認

### P16：forge github 怎麼接 GitHub、怎麼選

- 問題：用 `gh` 還是直接打 REST API？哪個 team 用 GitHub？
- 建議：daemon 一律用 `gh`（你已登入的那個），經 `Runner`、每次 60 秒 timeout（D28）。用哪個 forge 由 workflow 的 submit 關卡決定（core 已有 `forge = "github"`），merge 用同一個；team 不加欄位。
- 理由：不存 token、不另寫 HTTP 與登入。
- 替代方案：HTTP client 打 REST、token 放設定（多一份 secret）；team 設定 `forge`（跟 workflow 兩份）。
- 例子：`submit(forge = "github")` → log `forge github: suzuke/agend-sandbox (gh logged in)`。
- 關係：D4、D28、D29。
- [ ] 使用者確認

### P17：GitHub 上的 branch 跟本機不一樣時，以本機為準

- 問題：agent 可以自己 push（第 3 施工關 T7），外面的人也可能往 PR 推。以哪邊為準？
- 建議：**以 worktree 的本機 branch 為準**；`Forge::head()` 回本機 head（跟 FRG-1、FRG-4、FRG-9 一致）。submit 與每次送 merge 前，daemon `git fetch` 那個 branch，比較遠端 R 與本機 L：
  1. 遠端沒有 → push（不 force）。
  2. R == L → 不動（agent 自己推過）。
  3. R 是 L 的祖先（本機比較新，含 submit 後又 commit，FRG-5）→ fast-forward push。
  4. 其他（遠端有本機沒有的 commit）→ 不覆蓋、不 merge，task 回 work，持有者收到「`origin/agend/<t>/<slug>` 有你沒有的 commit，`git pull --ff-only` 後再 `agend done`」。core 用新事件 `RemoteDiverged`（P27）；已經送出 merge 後才發現（409）時先餵 `MergeFailed` 再餵它。
  只有 D14 的 rebase 會 force（`--force-with-lease=<branch>:<剛 fetch 到的 R>`）。開 PR 前先查有沒有（`gh pr list --head`），沒有才開。
- 理由：checks、審查、核准都是對本機 head 做的；遠端多出來的沒人測過、審過。
- 替代方案：以遠端為準（daemon 要改 agent 正在用的 worktree）；直接覆蓋遠端（丟掉別人推的東西）；情況 4 借用 `MainAdvanced{conflict: true}` 回 work（不改 core，但語意是「main 前進」，log 與 TUI 會寫錯原因）。
- 例子：你核准後有人往 PR 推了一個 commit → 送 merge 前的同步發現情況 4 → log `t-5: origin has commits not in the worktree (9f8e…); back to work`，沒有呼叫 merge。
- 關係：第 3 施工關 T7；FRG-1、FRG-5；P27（core 新事件）。
- [ ] 使用者確認

### P18：push 時保留 `SSH_AUTH_SOCK`

- 問題：daemon 用你的 git 憑證 push。第 10 施工關 P5 的 `env_clear` 拿掉了 `SSH_AUTH_SOCK`，SSH remote 會推不上去。
- 建議：只有 push 這一種 git 呼叫額外保留 `SSH_AUTH_SOCK`；HTTPS remote 走 `gh auth setup-git` 的 credential helper（U14）。
- 理由：只開 push 需要的那一個變數。
- 替代方案：一律用 `gh` 的 token 走 HTTPS（不管 remote 設定，要改寫 URL）。
- 例子：`git remote -v` 是 `git@github.com:…` → push 成功，log 沒有 `Permission denied (publickey)`。
- 關係：**與已確認的第 10 施工關 P5「daemon 的 git 一律 `env_clear`」不同，請明確決定**。
- [ ] 使用者確認

### P19：github 的 team 從 `origin/main` 開 worktree

- 問題：github 的 merge 發生在遠端，本機 main 不會前進。新的 worktree 從哪開？
- 建議：綁定時先 `git fetch origin main`，從 `origin/main` 開；本機 main 不動。
- 理由：不然每個新 task 都從越來越舊的 main 開始。
- 替代方案：daemon 也 fast-forward 本機 main（要處理 checkout 著的 main，第 10 施工關 P7 那一整套）。
- 例子：你在 GitHub merge 了 3 個 PR 之後開新 task → worktree 的起點是 `origin/main` 最新的那個。
- 關係：**與已確認的第 10 施工關 P4 第 2 步「從 main 的 SHA 建」不同，請明確決定**。
- [ ] 使用者確認

### P20：merge 與 main 前進

- 問題：怎麼保證 GitHub merge 的是核准的那個 head？已經 merge 過怎麼知道？
- 建議：先照 P17 同步；再 `gh api -X PUT repos/<o>/<r>/pulls/<n>/merge -f sha=<核准的 head> -f merge_method=merge`（官方 [Merge a pull request](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request)：`sha` 不符回 409，不能 merge 回 405）。409（同步後又被推）→ 照 P17 情況 4；405 → `MergeFailed`＋「需要你」`merge-blocked`。一律 merge commit。main 前進（D14）：`origin/main` 不在 head 裡 → rebase、比 patch-id、force-with-lease。已 merge 與否看 PR 的 `merged`。
- 理由：GitHub 自己比 SHA，等於 forge local 的 CAS；PR 狀態是真相。
- 替代方案：自己讀 head 再 merge（有競態）；照 repo 設定 squash（V1-LESSONS #6）。
- 例子：`t-5 merge: PR #42 merged (merge commit 7c1e…)`。
- 關係：D14、FRG-5..8；第 10 施工關 P7、P8。
- [ ] 使用者確認

### P21：關 PR、刪遠端 branch 放在哪

- 問題：task 取消、失敗、merge 後要關 PR、刪遠端 branch。Forge trait 只有 3 個方法，沒有這個。
- 建議：daemon 在 trait 外面直接跑 `gh`（`forge::github` 的一個函式，釋放 binding 時呼叫）。
- 理由：只有 github 需要，不為它改 trait 與契約。
- 替代方案：Forge trait 加第 4 個方法 `close`（契約多一條，forge local 要實作空的）。
- 例子：`agend task cancel t-6` → `PR #43 closed; remote branch deleted`。
- 關係：選替代方案就是**與 D29「forge 維持 3 個方法」不同**。
- [ ] 使用者確認（trait 外／加方法：＿＿）

### P22：agent 手上的 gh 登入（安全決定，請選一個）

- 問題：agent 在你的 uid 下跑、沒有沙箱，PATH 上的 `gh` 用你的登入。它可以直接 `gh pr merge 42`，跳過你的人工核准；shim 只守 git。好意的 agent 很可能手滑打出這行（它以為最後一步就是 merge），照第 3 施工關的判斷方法屬於應該擋的。
- 選項：
  - A. agent 的環境加 `GH_CONFIG_DIR=<空目錄>`，gh 找不到登入（keychain 還會不會被讀，U20）；只有 daemon 用真的設定。agent 就不能用 gh 看 issue。
  - B. shim 也包 `gh`：拒絕 `pr merge`、`pr review --approve`、`api` 打到 merge／reviews／設定類路徑、`auth token`，其餘放行。擋手滑。
  - C. 接受，只要求你在 GitHub 開 branch protection（daemon 用你的帳號，required review 也會讓 daemon 的 merge 405，除非你是 admin 可 bypass）。
- 建議（請你決定）：B（P4 選 B 時 claude 還多一層 deny 規則）。
- 理由：擋掉最常見的手滑，agent 照常能用 gh 的其他功能。
- 替代方案：A、C。
- 例子：選 B 時 agent 跑 `gh pr merge 42` → `agend-shim: refused gh pr merge (merges go through agend)`，exit 1。
- 關係：選 A、B 都擴大第 3 施工關 shim 的範圍，**請明確決定**。
- [ ] 使用者確認（選 ＿＿）

### P23：checks 裡的 gh 與 GitHub CI（安全決定，請選一個）

- 問題：D29 讓 GitHub CI 用 `command` 關卡跑 `gh pr checks {pr} --watch`，所以 checks 要讀得到 gh 的登入。但 checks 跑的是 agent 寫的程式碼，而且 checks 的 PATH 沒有 shim（第 10 施工關 P6），P22 的 B 管不到：任何測試都能 `gh auth token` 拿走你的 token。
- 選項：
  - A. 照 D29，接受：checks 讀得到 gh 登入，寫進已知風險。
  - B. checks 的環境一律 `GH_CONFIG_DIR=<空目錄>`；等 GitHub CI 改成 daemon 在沙箱外跑 `gh pr checks`（workflow 寫法不變，runner 認出 `gh pr checks` 就由 daemon 跑）。
  - C. 同 B，但 workflow 改用新的關卡種類「等 forge CI」。
- 建議（請你決定）：B。
- 理由：token 不進跑 agent 程式碼的地方；workflow 寫法不變。
- 替代方案：A、C。
- 例子：選 B 時 checks 裡 `gh auth token` → `no oauth token found`；CI 那一關 log `ci t-5/ci/1: waited outside the sandbox (3 checks passed)`。
- 關係：B **與 D29「GitHub CI 用 `command` 關卡接」的做法不同**（寫法相同、執行位置不同），C 要改 D29 與 core 的關卡種類，**請明確決定**。
- [ ] 使用者確認（選 ＿＿）

### P24：Telegram 的 allowlist 用 user id

- 問題：誰可以按按鈕、回答？用 chat id 還是發訊者的 user id？
- 建議：`config.toml` 的 `[telegram]` 寫 `chat_id`（群組）與 `allowed_user_ids`（誰能操作）；只看發訊者的 user id。空的 → daemon 不收、開機 log error、`doctor` 失敗 exit 1（v1 #2207）。沒有 `[telegram]` → 關閉。本關建 `config.toml`（第 9 施工關 P9：第一個讀它的施工關負責建）。
- 理由：群組裡每個人的 chat id 都一樣，擋不了人。
- 替代方案：用 chat id（群組裡任何人都能核准 merge）。
- 例子：`allowed_user_ids = []` → `FAIL telegram: allowed_user_ids is empty; every message would be dropped. Fix: add your user id …`，`exit=1`。
- 關係：**與原頁步驟 4、[tui-and-setup](../architecture/tui-and-setup.md#安裝與設定)「chat id 加入 allowlist」不同，請明確決定**（第 13 關 `telegram setup` 跟著改）。
- [ ] 使用者確認

### P25：Telegram token 放哪（安全決定，請選一個）

- 問題：token 被拿到，就能搶走你的按鈕與回覆（`getUpdates` 推進 offset，daemon 收不到），也能用 bot 發假訊息。同一個 uid 的 agent 與 checks 都讀得到 0600 的檔。
- 選項：
  - A. `$AGEND_HOME/secrets/telegram.token`（0600），接受 agent 與 checks 讀得到。
  - B. 同 A，再讓 checks 沙箱不准讀 `$AGEND_HOME/secrets`（agent 本身仍讀得到）。
  - C. macOS keychain（`security`），agent 讀取時會不會跳授權視窗未查證（U21）；Linux 仍用檔案。
- 建議（請你決定）：B（P4 選 B 時 claude 另有 deny 規則）。真正擋住同一個 uid 的 agent 要讓 daemon 用另一個 uid，本關不做。
- 理由：checks 最容易無意間讀到；agent 本身只能寫明。
- 替代方案：A、C；token 寫在 `config.toml`（貼設定時容易外流）。
- 例子：選 B 時 checks 裡 `cat $AGEND_HOME/secrets/telegram.token` → `Operation not permitted`。
- 關係：token 不在 `config.toml`，**與 [tui-and-setup](../architecture/tui-and-setup.md#設定與目錄d8)「config.toml 放 Telegram 連線設定」字面不同**；B **修改已確認的第 10 施工關 P6**；請明確決定。
- [ ] 使用者確認（選 ＿＿）

### P26：新的依賴：HTTP client

- 問題：Telegram 要 HTTPS；opencode 要 HTTP＋SSE。用什麼？
- 建議：`agend-daemon` 加阻塞式的 `ureq`＋rustls；Telegram 的 long poll 與每個 opencode 的 SSE 各一條 std thread（第 7 施工關 `tungstenite` 的做法），不碰 tokio 的巢狀 `block_on`（v1 #1476）。`check-deps` 加一條：`agend-client`、`agend-shim` 不能依賴它。
- 理由：一個 crate 滿足兩邊。
- 替代方案：`reqwest`（async，依賴多）；自己寫 HTTP（不做 TLS）；`teloxide`（v1 問題來源）。
- 例子：`cargo xtask check-deps` 多一條規則仍 `ok`。
- 關係：D10、D11。
- [ ] 使用者確認

### P27：要改 core 的地方，以及「重過第 1 關」包含什麼

- 問題：core 現在的通知只能送「標題＋內容」，不知道要送到哪個 topic、要不要附按鈕、解決後要改掉；pipeline 也沒有「遠端多了 commit」這個事件；協定 1.4 的型別也在 core。D22 說要改 core 就先改、再重過第 1 關。
- 建議（白話）：
  - 通知多帶三樣：送到「需要你」還是哪個 team、附哪些按鈕、跟哪一項「需要你」有關（加 D37 的摘要）；多一個動作「這一項解決了」（Telegram 用它把訊息改成已解決）。收手機上的回覆不經 core，由 daemon 直接轉成跟 TUI 一樣的「解決／回答」。
  - pipeline 多一個事件 `RemoteDiverged`：task 回 work、把原因告訴持有者（P17）。
  - 協定 1.4：claude 的 channel 與 hook 用的四個請求與事件（P2）；只加不改（D26）。
  - 「重過第 1 關」＝ `cargo test -p agend-core`、`cargo xtask check-deps`（no-std 建置）、`cargo xtask accept core` 全過，第 2 施工關的假實作與契約補新規則，**你再跑一次第 1 施工關的「你親自驗收」**。
- 理由：只加用得到的東西（D27）；一次改完，四段都用。
- 替代方案：不改 core、daemon 直接用自己的型別（Telegram 就不能對假實作測，違反 D9）；「重過」只跑自動測試、你不重驗（比 D22 字面少）。
- 例子：「需要你」出現一項核准 → 通知帶 `送到: 需要你`、`按鈕: approve, request_changes` → 手機上看到兩個按鈕；你在 TUI 核准 → 「這一項解決了」→ 手機那則變成「已解決（TUI）」、按鈕消失。
- 關係：D9、D11、D22、D26、D27、D37。
- [ ] 使用者確認

### P28：Telegram 發什麼、發到哪

- 問題：哪些事會吵你的手機？內容太長怎麼辦？
- 建議：「需要你」topic：每項一則，開頭四行摘要（D37），按鈕是它的 `actions`（`callback_data` 上限 64 bytes，[官方](https://core.telegram.org/bots/api#sendmessage)，存短代號）。team topic：task merged、failed、cancelled 與逾時「通知」。Info 不發；個別 instance topic 不做（D13）。topic 第一次用時才建（U16）。純文字；超過 4096 字依序切段（NTF-2、NTF-4）。
- 理由：只有例外會叫你。
- 替代方案：每個事件都發；長內容傳成檔案。
- 例子：手機「需要你」topic：`approval t-5/approve/1`＋四行摘要＋`[approve] [request changes]`。
- 關係：D13、D35、D37；NTF-1..4。
- [ ] 使用者確認

### P29：Telegram 收什麼

- 問題：手機上按按鈕、回覆文字，daemon 怎麼收？
- 建議：`getUpdates` long poll，`offset` 存 DB（設了 webhook 時不能用 → `doctor` 失敗）。按鈕 → 以操作者身分 `resolve_attention`；`request_changes` 要你回覆一則理由。回覆請示 → `answer_ask`（D35）。非 allowlist 的人忽略、每人只 log 一次。同一個 bot 只能一個 daemon 收（跟 v1 並行那週用另一個 bot）。
- 理由：跟 TUI 同一條權限與解決路徑。
- 替代方案：webhook（要對外開 port）。
- 例子：你按 `approve` → watch 印 `attention_resolved approval:t-5/approve/1 by telegram:123456789`。
- 關係：第 8 施工關 P2、D35。
- [ ] 使用者確認

### P30：通知紀錄保留 30 天

- 問題：通知先寫 DB 的 outbox 再送（daemon 當掉、Telegram 連不上都不會漏；送出與記錄之間當掉可能重送一則）。這張表留多久？D31 沒列這種資料。
- 建議：30 天，比照訊息。
- 理由：跟訊息一樣是「送出去的東西」，查問題時要對得上。
- 替代方案：14 天（比照事件）。
- 例子：30 天前的通知紀錄在每日清理時刪掉。
- 關係：**這是擴充 D31，請明確決定**。
- [ ] 使用者確認

### P31：G4 已讀狀態不做

- 問題：第 8 施工關把 G4（daemon 記已讀、TUI 與 Telegram 共用）移來本關。Telegram 能告訴我們你讀了嗎？
- 建議：不做，TUI 維持本機已讀（第 11 施工關 T4、T17）。Bot API 裡找不到已讀資訊（U17），Telegram 只能提供「按了、回了」＝已解決。
- 理由：沒有第二個會寫已讀的來源。
- 替代方案：照 G4 做（多台 TUI 會同步；協定 1.4 多一組請求）。
- 例子：TUI 展開一項 → 不再粗體；手機上同一項照樣顯示。
- 關係：**與已追認的第 11 施工關 G4 與第 8 施工關 P6 不同，請明確決定**。
- [ ] 使用者確認

### P32：其他施工關記給本關、本關照做的事

- 問題：前面幾關「記給第 12 施工關」、沒有爭議的事，一次確認。
- 建議：`doctor` 加：backend 安裝／版本（跟錄製檔不同是 `warn`）／登入（不花 token 的查法，U22）、gh 登入、github main 沒有 branch protection（`warn`）、Telegram；逾時「通知」送 Telegram（P28）；claude、opencode 的清掃（P10、P11）；你自己的 MCP server／plugin 在每個 agent 生效：只記錄、不處理。
- 理由：都已有來源規定或只是補完。
- 替代方案：`doctor` 的幾列延到第 13 關。
- 例子：`agend doctor` 多 `claude 2.1.282 (tested)`、`gh: logged in`、`github main: no branch protection (warn)`、`telegram: ok`。
- 關係：第 7、9、10 施工關；D24。
- [ ] 使用者確認

### P33：卡住偵測、usage limit、自動選忙碌等級不在本關

- 問題：第 10 施工關把這三件「建議移到第 12 施工關」。要做嗎？
- 建議：不做，之後另排。
- 理由：本關已有四大塊；這三件要真 backend 的長期資料與每個 backend 的螢幕規則（V1-LESSONS #3）。
- 替代方案：本關一起做（範圍約加一倍）。
- 例子：本關結束時，claude 撞到 usage limit 只會在畫面上看到，不會出現在「需要你」。
- 關係：**與第 10 施工關「建議移到第 12 施工關」不同，請明確決定**。
- [ ] 使用者確認

### P34：什麼是假的、什麼是真的

- 問題：CI 不能跑真的 claude、opencode、GitHub、Telegram。各自對什麼測？
- 建議：假的：`fake-claude`、`fake-opencode-serve`（補密碼、`messageID`）、新的 `fake-gh`（對本機 bare repo，409／405 可編排）、新的 `fake-telegram`。契約：DRV（四次開機）、FRG、NTF。錄製（你核准才跑）：claude `resume_empty`、`startup_dialogs`；opencode `message_id`、`password`。真的、選做：`claude_live`、`opencode_live`（`record-sandbox.sh` 裡跑；workspace 在 `/private/tmp/agend-rec-live-*`，因為沙箱只准 claude 寫 `~/.claude/projects/-private-tmp-agend-rec-*`）。本 agent 與 verifier 都不跑真的。
- 理由：邏輯全在 CI 驗，只有「真 CLI 接不接受」要真跑（第 7 施工關 P8）。
- 替代方案：CI 用真的 GitHub（要 token、會留下 PR）；不做 `fake-gh`。
- 例子：`contract Forge: github+fake-gh 10/10 pass`。
- 關係：D9；第 7 施工關 P8。
- [ ] 使用者確認

### 本關不做（明確列出）

- 卡住偵測、usage limit、自動選忙碌等級（P33）。
- 授權請求轉給人回答（claude 的 channel permission relay、opencode 的 permission、codex 的 approval）；本關一律拒絕並記 log。
- 「卡在未知提示」的按鍵回答與規則學習（P7 只做最小版）。
- Telegram 個別 instance topic、`agend telegram setup`（第 13 施工關）。
- `ctrl+enter` 強送（P8、U2）。
- daemon 用另一個 uid 跑（P25）。

### 已知風險（開工時處理）

- 依賴第 9、10、11 施工關的實作：協定號（1.4）、migration 號、`agend send`／`doctor`／`task create`／`debug watch` 的確切輸出，開工時照實際的改這頁。
- claude 的 development channel 是 research preview，旗標與對話框可能隨版本改。
- 用你的 `~/.claude` 設定時，你自己的 hooks 也會跑。
- agent 沒有沙箱：P4、P12、P22、P23、P25 的選項都只擋到「手滑」這一層。
- GitHub 的 branch protection、required review、只准 squash 都會讓 merge 405，本關只把它變成「需要你」。
- Telegram 送出與記錄之間當掉會重送一則（P30）。

**未查證的事實**（本 agent 沒有跑任何真 CLI；「怎麼查」裡不花 token 的你可以自己跑）：

| # | 事實 | 影響 | 怎麼查 |
|---|---|---|---|
| U1 | claude 忙碌時送到的 channel 訊息會自己排隊 | 只是背景（P8 不依賴它） | **部分查證**：官方 [channels reference](https://code.claude.com/docs/en/channels-reference)「Events queue into the session and are processed in order」；spike C1 看到模型可能不做 |
| U2 | `ctrl+enter`（`chat:sendNow`）對 channel 送來的訊息有沒有作用 | P8 | 官方 [interactive mode](https://code.claude.com/docs/en/interactive-mode) 只寫「你排隊的訊息」；`claude_live` 加一段 |
| U3 | `enabledMcpjsonServers` 能不能跳過 MCP 對話框；三個對話框的畫面與單鍵 | P6 | 錄製 `startup_dialogs`（`agend-record startup-check claude`，不送 prompt） |
| U4 | 沒送過訊息就被殺，`--resume <id>` 的錯誤字樣 | P10 | 錄製 `resume_empty`（不送 prompt） |
| U5 | claude 的 Bash 工具是不是 login shell、shim 在不在第一個 | P5 | `claude_live` |
| U6 | `--permission-mode auto` 在 daemon 起的互動 session 能不能用、模型支不支援 | P4 選 C | [permission modes](https://code.claude.com/docs/en/permission-modes) 寫了模型限制；`startup-check` 加旗標 |
| U7 | `transcript_path` 的 jsonl 格式 | P9 替代方案 | 讀一個錄製留下的 transcript |
| U8 | opencode 的 `messageID` 格式要求 | P14 | 錄製 `message_id` |
| U9 | 真 `opencode serve --port 0` 會印出實際 port | P11 | `opencode serve --hostname 127.0.0.1 --port 0` 看第一行（不花 token） |
| U10 | 設了密碼後 `attach` 連得上；serve 死了 TUI 會不會自己結束 | P11 | [CLI 文件](https://opencode.ai/docs/cli/)寫 `--password` 預設讀這個變數；`opencode_live` |
| U11 | opencode 的 bash 工具用什麼 shell、shim 在不在第一個 | P5 | `opencode_live` |
| U12 | `external_directory` 放行 worktree 之後，worktree 裡的工作還會不會被問 | P15 | **機制已查證**：[CLI 文件](https://opencode.ai/docs/cli/)列了 `OPENCODE_CONFIG_CONTENT`、`OPENCODE_PERMISSION`；[config](https://opencode.ai/docs/config/) 寫明合併規則。未查證：合併是不是深到 `permission` 底下（`.env` 預設規則還在不在）、效果如何；`opencode_live` 讀一次 `.env` |
| U13 | macOS 沙箱裡 `gh` 讀不讀得到 keychain 的 token | P23 選 A | 第 10 施工關做完後在沙箱裡跑 `gh auth status` |
| U14 | 你的 sandbox repo 的 remote 是 SSH 還是 HTTPS | P18 | `git -C <clone> remote -v` |
| U15 | GitHub merge API 帶 `sha` 不符回 409、不能 merge 回 405 | P20 | **已查證**：[Merge a pull request](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request) |
| U16 | `createForumTopic` 要 bot 有哪個管理員權限 | P28 | [Bot API](https://core.telegram.org/bots/api#createforumtopic)（沒讀到） |
| U17 | Bot API 沒有已讀資訊 | P31 | [Bot API](https://core.telegram.org/bots/api) 讀到的部分沒有；未完整查證 |
| U18 | 訊息上限 4096 字、`callback_data` 1–64 bytes；429 的 `retry_after` | P28、P30 | 前兩個**已查證**（[sendMessage](https://core.telegram.org/bots/api#sendmessage)）；`retry_after` 沒讀到 |
| U19 | macOS 上 `ps eww` 讀不讀得到同一個使用者其他程序的環境變數 | P12 | `env FOO=bar sleep 60 &`，另一個終端 `ps eww -p <pid>`（不花 token） |
| U20 | `GH_CONFIG_DIR` 指到空目錄時 gh 是否當成沒登入 | P22 選 A、P23 選 B | `GH_CONFIG_DIR=$(mktemp -d) gh auth status`（唯讀） |
| U21 | 同一個 uid 讀 keychain 項目會不會跳授權視窗 | P25 選 C | 開工時用測試項目試 |
| U22 | claude、opencode 不花 token 的「登入了沒」查法 | P32 | 各自 `--help` |

## 自動驗收（完成定義）

每段（P1）做完就跑自己那幾項；四段都過才算本關完成。

- [ ] P27 的「重過第 1 關」：`~/.cargo/bin/cargo test -p agend-core`、`~/.cargo/bin/cargo xtask accept core` 通過；你重跑第 1 施工關的「你親自驗收」
- [ ] `~/.cargo/bin/cargo test -p agend-daemon`、`~/.cargo/bin/cargo test -p agend-testkit`、`~/.cargo/bin/cargo test -p agend` 單獨通過，包括：兩個 driver 對假 agent 跑 DRV-1..9（DRV-6、DRV-9 四次開機）；hook spool 補送；Stop hook 一次取出全部佇列、`stop_hook_active: true` 不再 block；忙碌時絕不經 channel；轉給 holder 的 `Esc` 之後 3 秒沒有 hook 記成閒；雜湊不符的 `.mcp.json`／`CLAUDE.md` → `failed`；`delivery = inbox` 不寫設定檔；空 session 重起規則；`stuck-prompt` 出現；opencode 沒密碼 → 401；清掃只殺自己的 group；github 對 `fake-gh` 跑 FRG-1..10、P17 四種情況各一條、`RemoteDiverged`、409／405、PR 不重開、已 merge 用 PR 狀態、關 PR；選了的安全選項各有一條測試（例如 P22 選 B：shim 拒絕 `gh pr merge`）；notifier 對 `fake-telegram` 跑 NTF 契約、切段、空 allowlist、非 allowlist、outbox 補送、「已解決」改訊息；schema fixture 與 golden 更新
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過），含 P26 的新規則
- [ ] `~/.cargo/bin/cargo xtask accept adapters` 通過，並印出下方「你親自驗收」步驟 1 的 demo
- [ ] 真 CLI 一致性檢查（必要；使用者已決定 2026-09-25）：`claude --version`、`opencode --version` 和 `crates/agend-testkit/transcripts/{claude,opencode}/` 錄製檔 header 的 `version` 相同，不同就先重錄（[RECORDER.md](../../crates/agend-testkit/RECORDER.md#重錄cli-升版時)）；`~/.cargo/bin/cargo test -p agend-testkit --test conformance` 通過
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新；[BACKEND-BEHAVIORS](../BACKEND-BEHAVIORS.md) 與 backends 分頁照 U 的結果更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。`<t-N>` 這類尖括號是會變的值；標「開工時細化」的是輸出的確切字樣。每一步標了歸哪一段（P1），在那一段驗收時做。步驟 3、4、5 跑真的 backend、花少量 token；步驟 6、7 用你的 GitHub sandbox repo；步驟 8–10 用你的 Telegram bot。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.x.y`（目前是 `agend 0.0.0`）。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

`AGEND_HOME` 一定要設（第 13 施工關之前沒有預設值），沒設的話 `agend` 會拒絕執行。步驟 1–4 自己建暫存 home。步驟 5–10 共用一個暫存 home（步驟 5 建；只做 C 或 D 段時，用步驟 5 的第一段建 home、起 daemon，跳過加 instance 的部分）：每個步驟的指令第一行都是 `export AGEND_HOME=<home>`。另開一個「watch 分頁」：先跑開頭那段，再 `export AGEND_HOME=<home>` 與 `agend debug watch`，之後說「watch 印」就是看這個分頁。

1. 跑 demo（全部對假的；每段各跑自己那一節）。

   **這步在驗什麼**：四段在假 agent、`fake-gh`、`fake-telegram` 上都走得通。錯了代表後面真的步驟看到的都不可信。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo xtask accept adapters
   ```

   應該看到：依序 `== claude`、`== opencode`、`== github`、`== telegram`、`== restart`，倒數第二行 `adapters demo: all sections passed`，最後一行 `gate 12 (adapters): checks passed`（開工時細化；還沒做的段印 `skipped (segment not built)`）。

   - [ ] 通過

2. 真 CLI 一致性檢查（必做；A、B 段）。

   **這步在驗什麼**：假 claude／假 opencode 和你機器上真的 CLI 形狀一致。壞了的話，driver 對假的全綠、接上真的才出錯（v1 #1483）。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   claude --version; opencode --version
   head -1 crates/agend-testkit/transcripts/claude/one_turn.jsonl crates/agend-testkit/transcripts/opencode/one_turn.jsonl
   ~/.cargo/bin/cargo test -p agend-testkit --test conformance
   ```

   應該看到：兩個版本各自和錄製檔 header 的 `"version"` 相同；最後 `test result: ok.`。版本不同：先重錄那個 backend（`~/.cargo/bin/cargo xtask record claude --sandbox ~/Documents/Hack/AgEnD-ops/record-sandbox.sh`，或 `opencode`；花少量 token，見 [RECORDER.md](../../crates/agend-testkit/RECORDER.md)）再跑；不過就改假 agent，不改錄製檔。

   - [ ] 通過

3. 真 claude 端到端（`claude_live`，A 段，約 3 個短 turn）。

   **這步在驗什麼**：啟動對話框被規則處理（U3）、P4 選的權限模式生效、channel 訊息被確認、claude 跑的 `git`／`pkill`／`killall` 是 shim（U5）、holder 被 `kill -9` 後沒有 claude 留下、重起後上下文還在。錯了的話，真 claude 會卡在對話框，或 agent 的 git 繞過 shim。

   P4 選 A 或 B 的話，先在沙箱外、你自己的終端機跑一次 `claude --permission-mode bypassPermissions`，接受警告後 `/exit`（沙箱不准寫 `~/.claude/settings*.json`，接受紀錄存不下來）。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo build -q -p agend --bin agend && ~/.cargo/bin/cargo build -q -p agend-daemon --example claude_live
   AGEND_REAL_CLAUDE=1 ~/Documents/Hack/AgEnD-ops/record-sandbox.sh target/debug/examples/claude_live 2>/tmp/g12-claude.log | tee /tmp/g12-claude.out
   pgrep -fl "agend (holder|daemon|channel)"
   ```

   應該看到（開工時細化）：`workspace /private/tmp/agend-rec-live-…`、`startup dialogs handled: …`、`m-1 idle → channel → confirmed`、三行 `git: <H>/bin/git (the shim)`…、（P4 選 B）`gh pr merge → denied by permission rule`、`claude left: []`、`restart 1/3, session <S> resumed`、`m-2 → confirmed; reply mentions m-1's word: true`、最後 `claude_live: ok`；`pgrep` 沒有輸出。失敗時看 `/tmp/g12-claude.log` 最後 40 行。

   - [ ] 通過
   - [ ] 這次不做（寫進驗收紀錄）

4. 真 opencode 端到端（`opencode_live`，B 段，約 3 個短 turn，免費模型）。

   **這步在驗什麼**：真 opencode 接受 `--port 0`、密碼、`attach --session`（U9、U10）；`messageID` 確認得到（U8）；worktree 裡不被問權限（U12）；bash 找到 shim（U11）；沒有密碼連不上。錯了的話 driver 在真 opencode 上確認不了，或 agent 一直卡在權限。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo build -q -p agend --bin agend && ~/.cargo/bin/cargo build -q -p agend-daemon --example opencode_live
   AGEND_REAL_OPENCODE=1 ~/Documents/Hack/AgEnD-ops/record-sandbox.sh target/debug/examples/opencode_live 2>/tmp/g12-oc.log | tee /tmp/g12-oc.out
   pgrep -fl "opencode (serve|attach)"
   ```

   應該看到（開工時細化）：`serve ready on 127.0.0.1:<port> (auth on)`、`request without password → 401`、`m-1 idle → prompt_async → confirmed (messageID accepted)`、`permission asked: 0`、三行 `(the shim)`、`restart 1/3, session <S> resumed`、`opencode_live: ok`；`pgrep` 沒有輸出。

   - [ ] 通過
   - [ ] 這次不做（寫進驗收紀錄）

5. 三個真 backend 互傳訊息（B 段；A、B 都完成才做）。

   **這步在驗什麼**：同一個真 daemon 上，claude、codex、opencode 各收到一則、各自回覆，三則都到 `confirmed`。錯了的話某個 backend 的訊息會停在 `sent` 或 `queued`。

   第一個分頁：

   ```bash
   export AGEND_HOME="$(mktemp -d /tmp/g12.XXXX)" && echo "export AGEND_HOME=$AGEND_HOME"
   agend daemon
   ```

   記下印出的 `export AGEND_HOME=…`，開 watch 分頁。第三個分頁（先跑開頭那段）：

   ```bash
   export AGEND_HOME=<home>
   agend instance add g12-c claude; agend instance add g12-x codex; agend instance add g12-o opencode
   agend send g12-c "Reply with exactly: C-OK"; agend send g12-x "Reply with exactly: X-OK"; agend send g12-o "Reply with exactly: O-OK"
   ```

   應該看到：三個 `accepted`；watch 印三行 `… confirmed`（開工時細化）。

   - [ ] 通過

6. forge github：在你的 sandbox repo 跑完整流水線（C 段）。

   **這步在驗什麼**：真的 GitHub 上：daemon 只推 `agend/…`、開一個 PR、`gh pr checks` 等到 CI、你核准後帶著核准的 SHA merge、遠端 branch 被刪（P17、P20、P21）。錯了的話不是 merge 不了，就是 merge 了沒核准的 head。

   先準備：你自己的 GitHub repo `<owner>/<repo>`（有一個會通過的 GitHub Actions），clone 到本機 `<clone>`（team 的 canonical checkout）。第三個分頁：

   ```bash
   export AGEND_HOME=<home>
   ~/.cargo/bin/cargo run -q -p agend-daemon --example adapters_probe -- github-setup <clone>
   agend task create --team g12gh --role dev --workflow gh-demo "hello"
   ```

   `github-setup` 建 team `g12gh`、兩個第 10 施工關的 `fake-worker`（dev、reviewer）、workflow `gh-demo`（work → submit(github) → `gh pr checks {pr} --watch` → review → approve(human) → merge）。第二行印出 `<t-N>`。等 watch 印 `attention_required approval:<t-N>/approve/1`：

   ```bash
   gh pr list --repo <owner>/<repo> --head agend/<t-N>/hello --state all --json number,url
   ~/.cargo/bin/cargo run -q -p agend-client --example client_probe -- resolve approval:<t-N>/approve/1 approve
   gh pr view --repo <owner>/<repo> <pr> --json state,mergeCommit,headRefName
   git -C <clone> ls-remote origin "refs/heads/agend/*"
   ```

   `<pr>` 是第一行的 `number`。應該看到：`"state":"MERGED"`；merge commit 的訊息有 `Agend-Task: <t-N>`；`ls-remote` 什麼都不印。

   - [ ] 通過

7. 故意弄壞：核准之前，有人從外面往 PR 推了一個 commit（C 段）。

   **這步在驗什麼**：遠端多了本機沒有的 commit 時，daemon 在送 merge 前的同步就發現、不 merge、也不覆蓋，而是退回 work（P17）。錯了的話，被推進來的東西會跟著 merge，或被 daemon 默默蓋掉。

   照步驟 6 再開一個 task（`"second"`），watch 印 `approval:<t-M>/approve/1` 時先不要按。從**另一份 clone** 推一個 commit（`<clone>` 那個 branch 已經被 daemon 的 worktree checkout 著）：

   ```bash
   git clone -q "$(git -C <clone> remote get-url origin)" /tmp/g12-other
   git -C /tmp/g12-other switch -q agend/<t-M>/second
   echo extra > /tmp/g12-other/extra.txt && git -C /tmp/g12-other add extra.txt && git -C /tmp/g12-other commit -qm extra && git -C /tmp/g12-other push -q origin HEAD
   ~/.cargo/bin/cargo run -q -p agend-client --example client_probe -- resolve approval:<t-M>/approve/1 approve
   gh pr list --repo <owner>/<repo> --head agend/<t-M>/second --json number,state
   ```

   應該看到：watch 印 `<t-M>: origin has commits not in the worktree (…); back to work`（沒有呼叫 merge，所以沒有 409）；最後一行 `"state":"OPEN"`。收尾：

   ```bash
   agend task cancel <t-M>; rm -rf /tmp/g12-other
   ```

   watch 印 `<t-M> cancelled`；`gh pr list --repo <owner>/<repo> --head agend/<t-M>/second --state all --json state` 是 `CLOSED`。

   - [ ] 通過

8. Telegram：手機收到「需要你」，在手機上核准（D 段）。

   **這步在驗什麼**：「需要你」推到手機、手機上按的核准跟 TUI 走同一條路（P28、P29）。錯了的話你離開電腦就不知道有事卡著，或手機按了沒用。

   先準備：一個 bot（token 存成 `<home>/secrets/telegram.token`，`chmod 600`）、一個開了 topic 的 supergroup（bot 是管理員）、你的 user id。照 P24 寫 `<home>/config.toml`，重開 daemon（第一個分頁 Ctrl-C 後 `agend daemon`）。第三個分頁：

   ```bash
   export AGEND_HOME=<home>
   ~/.cargo/bin/cargo run -q -p agend-daemon --example adapters_probe -- telegram-setup
   agend task create --team g12tg --role dev --workflow demo "tg"
   ```

   （`telegram-setup` 建暫存 repo、team `g12tg`、兩個 `fake-worker`（`g12-fdev`、`g12-frev`）、第 10 施工關的 `demo` workflow（forge local，停在人工核准）。）

   應該看到：手機的「需要你」topic 出現 `approval <t-K>/approve/1`、四行摘要與兩個按鈕；按 `approve` → watch 印 `attention_resolved approval:<t-K>/approve/1 by telegram:<你的 id>`；手機那則變成「已解決」、按鈕消失；team topic 出現 `<t-K> merged`。

   - [ ] 通過

9. Telegram：在手機上回答請示（D 段）。

   **這步在驗什麼**：請示是對話（D35）：手機上回覆的文字送到 agent，那一項變成已解決（P29）。錯了的話你回了，agent 還在等。

   ```bash
   export AGEND_HOME=<home>
   ~/.cargo/bin/cargo run -q -p agend-daemon --example adapters_probe -- ask g12-fdev "Pick a color?"
   ```

   （讓步驟 8 建的假 agent `g12-fdev` 發一則 `agend ask`。）在手機上**回覆**那則訊息寫 `blue`。應該看到：watch 印 `ask <id> answered by telegram:<你的 id>: blue`；那一項從「需要你」消失。

   - [ ] 通過

10. 故意弄壞：Telegram 的 allowlist 是空的（D 段）。

    **這步在驗什麼**：設定錯了不會靜靜不動：`doctor` 失敗並給修正方式（v1 #2207、P24）。錯了的話你以為手機會收到，其實全部被丟掉。

    把 `<home>/config.toml` 的 `allowed_user_ids` 改成 `[]`：

    ```bash
    export AGEND_HOME=<home>
    agend doctor; echo "exit=$?"
    ```

    應該看到：Telegram 那一列 `FAIL`，附修正方式（在 `<home>/config.toml` 的 `allowed_user_ids` 加回你的 user id）；`exit=1`。重開 daemon 時 log 有 `telegram: allowed_user_ids is empty; not receiving`。改回來後那一列 `ok`。

    - [ ] 通過

11. 收尾（每段最後都做）。

    **這步在驗什麼**：什麼都不留（第 6 施工關的孤兒巡查照舊）。

    各分頁 Ctrl-C，然後：

    ```bash
    export AGEND_HOME=<home>
    ~/.cargo/bin/cargo run -q -p agend-daemon --example adapters_probe -- teardown
    pgrep -fl "agend holder g12-"; pgrep -fl "opencode (serve|attach)"; ls "$AGEND_HOME"
    ```

    應該看到：兩個 `pgrep` 都不印；`ls` 說找不到（home 已刪）；做過 C 段的話，`gh pr list --repo <owner>/<repo>` 空的。

    - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-28 第 2 輪 review REFUTED（1 HIGH、5 MEDIUM、5 LOW）後改寫成 P1–P34、每題一個決定：拿掉 claude「60 秒改走 channel」的退路（違反 D16 的 spike C1 證據），改用工具 hook 當心跳、`Esc` 經 daemon 轉送時判斷閒置（P8）；新增 `RemoteDiverged` 事件（P17、P27）與「卡在未知提示」最小版（P7）；checks 裡的 gh 另成一題（P23）；opencode 權限改成只放行 worktree，`OPENCODE_CONFIG_CONTENT`／`OPENCODE_PERMISSION` 改回官方有列；步驟 7、8、9、11 改成照抄能跑。
- 2026-09-28 第 1 輪 review REFUTED（5 HIGH、10 MEDIUM、4 LOW）後改寫成 P1–P26（`0ac05ad`）。
- 2026-09-28 開工前提案 P1–P21 寫定（draft PR #138，branch `docs/gate-12-proposal`）；狀態改為提案中。
- 2026-09-25 使用者決定：真 CLI 一致性檢查（錄製器 + `tests/conformance.rs`）列為必要完成條件（`feat/backend-recorder`）。

## 下一步

```bash
cat docs/gates/gate-12-adapters.md
~/.cargo/bin/cargo xtask accept adapters
```
