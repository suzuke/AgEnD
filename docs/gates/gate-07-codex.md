# 第 7 施工關：daemon：codex driver + 送達（`codex`）

> **TL;DR**
> - codex driver、送達模型、三級忙碌策略；codex 第一次有 thread id 可以 resume（補上第 6 施工關 H2 的缺口）。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：逐題決定下面的開工前提案 P1–P9（每題最後一行打勾）；另外請你在自己的終端跑「未查證」表裡的安全指令（不花 token），結果貼回來。P4 是安全決定（shim 可能被繞過），請特別看。第 6 施工關已 merge（#125），確認後即可開工。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**提案中**（2026-09-26）：開工前提案 P1–P9 待你逐題確認（P4 要你明確選 A／B／C／D）；第 6 施工關已 merge（#125，`3e28e06`）。

## 範圍

- 怎麼跟 codex 講話：holder 持有 `codex app-server`（附屬程序）與 codex TUI（PTY），daemon 的 driver 走 app-server 的 JSON-RPC（P1）
- 附屬程序：holder 協定新增 `SpawnSidecar`，就緒判斷放 daemon 的 driver（第 4 施工關 P8 移來，使用者 2026-09-25 決定；做法見 P2）
- codex 的 thread id：daemon 先建 thread、存進 DB，agent 一律用 `resume <id>` 起；補上第 6 施工關 H2「codex 死一次就 `failed`」的缺口（P3）
- codex 的啟動設定：`CODEX_HOME`、trust、sandbox／approval、更新提示（P4）
- 送達模型：訊息 id、`queued → sent → confirmed | failed` 各代表什麼、單一冪等、崩潰後對帳（P5）
- 三級忙碌策略：queue、steer、interrupt 對到 codex 的方法、閒置與競態怎麼處理；D30 去抖動校準（P6）
- driver 事件與 cursor：daemon 不在時的事件怎麼補回（P7）
- 測試：什麼是假的、什麼是真的；DRV 契約四次開機；真 codex 只在你核准時跑（P8）
- `check-deps` 新規則與移到後面施工關的事（P9）

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子。文件已定、這裡不重問的：訊息內容走 backend 的結構化 API、PTY 只送單一控制鍵、只有一套冪等（[delivery](../architecture/delivery.md#送達模型)、V1-LESSONS #1）；送達狀態只有 `queued → sent → confirmed | failed` 四個、轉換規則已寫在 core（`DeliveryState::can_transition_to`）；三級忙碌策略與 codex 對應的方法、「不支援 → 中斷」（[delivery](../architecture/delivery.md#忙碌策略三級)、`policy::busy::effective_level`）；推送帶完整內容（V1-LESSONS #2）；去抖動「轉 busy 立即、轉 idle 穩定 5 秒」（D30，本關只校準）；訊息保留 30 天（D31）；附屬程序由 holder 持有、協定是 `SpawnSidecar`（第 4 施工關 P8）；holder 死掉的 5 秒／3 次／`failed`、絕不自動全新啟動（第 6 施工關 P6）；agent 環境白名單（第 6 施工關 P3、H3）；holder 協定同 major 只加欄位（D26）；重啟類契約用四次開機、跨真的 process（[CONTRACTS.md](../../crates/agend-testkit/CONTRACTS.md)、第 2 施工關 A25）；真 CLI 一致性檢查是必要完成條件（使用者 2026-09-25）。

codex 的事實來源：[backends/codex.md](../backends/codex.md)、[spike-codex](../research/spike-codex.md)（`codex-cli 0.156.1`）、`crates/agend-testkit/transcripts/codex/` 的 5 個錄製檔、v1 `src/transport/codex_app_server.rs`（唯讀）。標 **未查證** 的是這些來源都沒證實的，查法集中在「已知風險」下面的表。

### P1：daemon 怎麼跟 codex 講話

- 問題：daemon 對 codex 送訊息、看狀態，走 app-server 的 JSON-RPC，還是讀 holder 的 PTY 畫面？PTY 裡跑什麼？
- 建議：
  - 一個 codex instance ＝ 一個 holder 裡的**兩個程序**：`codex app-server --listen unix://…`（附屬程序，P2）＋ PTY 裡的 codex TUI `codex --remote unix://<真正的 socket> resume <thread id>`（P3）。TUI 只給人 attach 看、給螢幕分類器認 hard gate。
  - daemon 的 `driver::codex` 只經 app-server 講話：WebSocket（`tungstenite`，阻塞 I/O）。每個 instance 一條長連線、**一條自己的 std thread**（比照第 6 施工關 H7 與 `runtime/link.rs`：活得跟連線一樣久的阻塞工作不佔 `spawn_blocking` 的名額）；一次性的呼叫（`thread/start` 等）才用 `spawn_blocking`。連上先 `initialize`，再 `thread/resume {threadId, excludeTurns: true}`（不 resume 就只收到粗粒度狀態，spike S2）。
  - 連線前一律 `realpath` socket 路徑（已有 `socket_connect_path`，陷阱 1）。
  - **不做**：PTY 打字、讀畫面判斷 busy／idle、v1 的「猜 TUI 開了哪個 thread」（`discover_loaded_tui_thread`，多 thread 時會拒絕送）。
- 理由：結構化 API 有訊息 id、turn id、`turn/completed`，是「確認送達」唯一可靠的來源；v1 改走 app-server 後「貼上沒送出」幾乎消失。TUI 還是要有，因為第 11 施工關的 attach 畫面要看得到 agent。
- 替代方案：只跑 app-server、PTY 空著（最簡單，但人 attach 看不到 agent）；讀畫面判斷（v1 的坑，V1-LESSONS #3、#4）；`tokio-tungstenite`（多一個 async 依賴，一條連線用不到）。
- 例子：`ps` 看到 holder `g7-1` 底下兩個程序：`codex app-server --listen unix://…/g7-1.codex.sock` 與 `codex --remote unix:///private/tmp/codex-daemon-501/… resume 01a0d1fc-…`；daemon log `g7-1: app-server connected, thread 01a0d1fc resumed (idle)`。
- [ ] 待你確認

### P2：附屬程序 `SpawnSidecar`：誰起、給什麼環境、怎麼知道好了、死了怎麼辦

- 問題：holder 協定要加什麼？app-server 拿到什麼環境？socket 放哪？app-server 死了、TUI 死了，各怎麼處理？`Shutdown` 怎麼停它？
- 建議：
  - holder 協定 V1 **加一個請求**：`SpawnSidecar {program, args, env, working_directory}` → 回 `SidecarSpawned {process_id}`。舊 holder 回 `Error{code: "unsupported_request"}`（`agend-holder/src/server.rs` 對不認得的請求就是這樣回），daemon 記 `holder too old for codex` 並當成起不來（D26：同 major 只加）。
  - `env` ＝ **跟 agent 一樣的白名單**（第 6 施工關 P3、H3：`AGEND_HOME`、`AGEND_INSTANCE`、以 `$AGEND_HOME/bin` 開頭的 `PATH`、`HOME`、`USER`…；不給其他 `AGEND_*`、不給 secret）。holder 同樣先全部清空再套用（第 4 施工關 P8）。app-server 才是真正跑指令的程序，所以它拿到的環境就是 agent 指令的環境（見 P4 的 PATH 風險）。
  - 跟 `Spawn` 一樣一個 holder 一生只一次：第二次回 `Error{code: "already_spawned"}`、什麼都不變（daemon 在兩步之間重啟時會重送）。
  - 順序：`SpawnSidecar` → driver 等就緒 → 建／接 thread（P3）→ `Spawn` TUI。
  - 就緒 ＝ driver 連得上（`realpath` 後）而且 `initialize` 成功；每 100 ms 試一次，**20 秒**放棄（v1 codex `ready_timeout_secs: 20`）。放棄算一次「死掉」，走第 6 施工關 P6。
  - socket：`$AGEND_HOME/run/holders/<id>.codex.sock`。路徑太長時 codex 會綁到 `/private/tmp/codex-daemon-<uid>/<sha256>`、這裡只剩 symlink（spike 的長路徑情況；2026-09-25 錄製時短路徑也這樣，所以一律 `realpath`）。
  - 附屬程序不接 PTY、自己一個 process group（group id ＝ 它的 pid，holder 自己 spawn、一定大於 1）；stdout／stderr 寫進 holder 的 log（`run/holders/<id>.log`）。
  - **一起結束、holder 留著**：app-server 或 TUI 任一個結束，holder 就停另一個（跟 `Shutdown` 停 agent 同一套：TUI 關 PTY master → 5 秒 → SIGKILL 它的 process group；app-server 送 SIGTERM → 5 秒 → SIGKILL 它的 process group）。之後 holder **照第 4 施工關 P7 留著**最後的畫面與結束狀態，回報 `Exited`（多一個選填欄位 `which: "sidecar" | "agent"`），直到 `Shutdown` 或 24 小時安全網。daemon 照第 6 施工關 H9 先 `Shutdown` 舊 holder、再起新的帶 resume；`failed` 時照 H8 留著給人看。
  - `Shutdown` 時 app-server 還在：跟上面一樣 SIGTERM → 5 秒 → SIGKILL 它的 process group，然後才刪 socket、放 lock。
- 理由：app-server 是 thread 狀態的主人，TUI 沒有它就沒用；分開各自重起要處理「TUI 連著一個剛換掉的 app-server」這類組合，整組重起只有一條路。holder 留著不改第 4 施工關 P7 與第 6 施工關 H8、H9 已追認的行為。holder 不講 codex 協定（`sidecar.rs` 的 Must NOT），就緒判斷只能放 driver。
- 替代方案：app-server 死了只重起它、TUI 留著（多一套狀態）；兩個都結束後 holder 自己退出（推翻第 4 施工關 P7，結束碼會遺失）；就緒看 socket 檔出現（檔案在、程序還沒 accept 時會連不上）；每個 codex instance 共用一個 app-server（一個死全部死、thread 混在一起）；app-server 繼承 holder 環境（漏 secret）。
- 例子：在沙箱裡只對 app-server 的 pid `kill -9` → holder log `sidecar exited (signal: SIGKILL); stopping agent`；`holder_probe snapshot g7-1` 仍拿得到最後畫面與 `exited which=sidecar`；daemon log `holder g7-1 died`、`Shutdown` 舊 holder、5 秒後 `restart 1/3 resume 01a0d1fc-…`。
- [ ] 待你確認

### P3：codex 的 thread id 與 resume（補第 6 施工關 H2）

- 問題：第 6 施工關 H2：codex 沒有 session id，第一次起來後一死就 `failed`。thread id 從哪來、存哪、怎麼 resume？
- 建議：
  - **daemon 自己先建 thread**：第一次起 instance 時，app-server 就緒後 driver 呼叫 `thread/start {cwd}`，拿到 `threadId`，**先寫進 `instances.session_id`**，再 `Spawn` TUI。
  - 所以 codex 的 TUI **每一次**都是 `resume <thread id>` 起，包括第一次；第 6 施工關的 `session_args` 對 codex 改成 `--remote unix://<真正的 socket> resume <id>`，沒有「新／舊」兩種。`new` 狀態對 codex 只代表「thread 還沒建」。**這改寫了第 6 施工關 H1 對 codex 的 `new` 意義**（H1 原本是「session 還沒建立、下次用 `--session-id`」），請一併確認。
  - 重起：新 holder、新 app-server → driver `thread/resume {threadId}` → TUI `resume <id>`（spike S4：同一個 `CODEX_HOME`，砍光再起仍保有完整上下文）。
  - `thread/resume` 說找不到這個 thread：
    - daemon 從沒對這個 instance 送出過訊息（`messages` 表沒有 `sent` 以上的列，P5）→ 視為「空 thread 沒落地」，建新 thread、覆寫 `session_id`、log 一行 `thread <old> not found and never used; new thread <new>`。**未查證**：codex 是不是要等第一個 turn 才把 thread 寫到磁碟（U1）。
    - **這是第 6 施工關 P6「絕不自動全新啟動」的例外，請你明確決定**：理由是空 thread 沒有上下文可丟。不接受的話，這種情況一律 `failed`（替代方案第 3 個）。
    - 其他情況 → `failed`，交給人（第 6 施工關 P6：絕不丟掉對話重來）。
- 理由：自己建 thread，id 在 agent 起來**之前**就在 DB，daemon 死在任何一步都不會出現「有 agent、不知道 thread」；不用像 v1 那樣從已載入的 thread 裡猜哪個是 TUI 的。第 6 施工關 H2 對 claude 也是「自己給 id」，同一個想法。
- 替代方案：讓 TUI 自己建 thread，daemon 再 `thread/loaded/list` 找（v1 的做法，多 thread 時要拒絕送）；讀 `~/.codex/sessions/` 的 rollout 檔名找 id（綁 codex 的檔案格式）；找不到一律 `failed`（holder 在第一則訊息前死掉也要人處理）。
- 例子：`daemon_probe add g7-1 --codex` 之後開 daemon：`g7-1: thread 01a0d1fc-… created`、`g7-1: holder pid=5230 started (resume 01a0d1fc-…)`；在沙箱裡砍掉 holder：`restart 1/3 resume 01a0d1fc-…`，thread id 不變。
- [ ] 待你確認

### P4：codex 的啟動設定與 **shim 會不會被繞過**（安全決定，請明確選）

- 問題：用哪個 `CODEX_HOME`？trust 提示、sandbox／approval、更新提示怎麼處理？最重要的：codex 跑的指令還會不會先找到 shim？
- **風險先講清楚**：codex 用 `/bin/zsh -lc "<指令>"` 跑每個指令（spike-codex.md 的 S7 payload、`approval.jsonl`）。`-l` 是 login shell，macOS 的 `/etc/zprofile` 會跑 `path_helper`，把 `/usr/bin`、`/opt/homebrew/bin` 等排到我們放在最前面的 `$AGEND_HOME/bin` **前面**。reviewer 實測：`env -i PATH=/tmp/fakeshimdir:/usr/bin:/bin /bin/zsh -lc 'echo $PATH'` 印出來 `fakeshimdir` 排在 `/opt/homebrew/bin`、`/usr/bin` 後面。結果：agent 的 `git`、`kill` 可能直接跑到真的 binary，**shim 的防護（第 3 施工關）對 codex 沒有作用**。再加上 `approval_policy="never"` ＋ `danger-full-access`，codex 自己也不擋。
- 建議（設定部分）：
  - `CODEX_HOME` 用你原本的（`~/.codex`，不設這個變數）：登入資料、rollout 都共用。**daemon 自己絕不寫 `~/.codex/` 裡的任何檔案**（codex 照常寫它的 sessions、log）。
  - 所有設定用**每次啟動的 `-c` 參數**（只影響那個程序）：trust `-c 'projects={"<realpath 後的工作目錄>"={trust_level="trusted"}}'`（v1 #3402）、`-c check_for_update_on_startup=false`（v1 #1626）、app-server 帶 `-c approval_policy="never" -c sandbox_mode="danger-full-access"`（v1 的做法）。
  - `-c` 放在子命令**前面**（`codex -c … app-server --listen …`，v1 #3402 的位置；v1 說 `-c` 在 0.148 是全域選項）。0.156.1 放後面行不行**未查證**（U12）。TUI 在 `--remote … resume` 時接不接受權限覆寫：v1 0.148–0.150 拒絕，0.156.1 **未查證**（U4）。
  - 另一條路（不必靠 `-c`）：`thread/start` 本身收 `approvalPolicy`、`sandbox`（錄製器 `recorder/codex.rs` 就這樣帶）。建議 approval／sandbox 同時用 `-c`（給 app-server 預設）和 `thread/start` 參數（給這個 thread），兩個都有就不怕其中一個被忽略。
  - 你的 `~/.codex/config.toml` 裡的 `notify`、hooks、`mcp_servers` 也會在每個 agent 生效（backends/codex.md 陷阱）。本關**不**覆寫它們（覆寫要逐個列名字、plugin 的關不掉），只記在風險；要不要關由你決定。
  - app-server 仍送來 approval 請求（`item/*/requestApproval`）時：driver 回 `decline`、log 一行 `approval declined (gate 7 has no handler)`。轉給人回答是第 10、11 施工關的事（P9）。
- 建議（shim 部分，請從下面選一個；**我建議 A**）：
  - **A. 自己的 `ZDOTDIR`**：agent 與 app-server 環境加 `ZDOTDIR=$AGEND_HOME/zsh`，裡面的 `.zprofile` 在 `/etc/zprofile`（`path_helper`）**之後**執行，只做一件事：`export PATH="$AGEND_HOME/bin:$PATH"`（再視需要 `source` 你的 `~/.zprofile`，順序放在前面）。**未查證**：codex 的 `shell_environment_policy` 會不會把 `ZDOTDIR` 濾掉（U13）；你的預設 shell 若是 bash，這招不適用（bash 的 login 檔在 `$HOME`）。
  - B. 讓 codex 用非 login shell：codex 有沒有這種設定**未查證**（U13 的同一份 schema／`--help` 查）。
  - C. `-c shell_environment_policy.set.PATH=…`：只設環境變數，login shell 之後還是會被 `path_helper` 重排，**單獨用沒用**，只列出來說明為什麼不選。
  - D. 本關先接受風險：只靠 worktree 的 git hook（第 10 施工關才裝，而且只擋 git 的 ref 更新，不擋 `kill`），並把 U8 的結果記下來再決定。
  - 不管選哪個：`codex_live`（P8）加一個 turn 跑 `command -v git kill`，印出的路徑必須在 `$AGEND_HOME/bin`（U8）；假 app-server 的測試照選定的方案檢查組出的環境。
- 理由：v1 試過獨立 `CODEX_HOME`，登入資料會分岔（`provider_detect.rs` 的註解）；回答 trust 提示會讓 codex 在你的 config 裡每個 workspace 永久加一筆（v1 #3317），`-c` 什麼都不寫。shim 是 v2 對 git／kill 唯一的防護，codex 又關掉了自己的 sandbox，所以 PATH 順序必須是明確的決定，不能默默假設（比照第 6 施工關 H14，與你已確認的安全前提字面不同的地方要你明確選）。
- 替代方案：每個 instance 自己的 `CODEX_HOME`＋複製 `auth.json`（登入過期要逐個處理）；`workspace-write` sandbox＋逐一核准（第 7 施工關沒有核准的人，agent 會卡住；也不解決 PATH）；預寫 `config.toml`（改到你的檔案）。
- 例子：選 A 時，daemon 組出的 app-server 指令是 `codex -c 'projects={"/Users/you/ws/g7-1"={trust_level="trusted"}}' -c check_for_update_on_startup=false -c approval_policy="never" -c sandbox_mode="danger-full-access" app-server --listen unix://…/g7-1.codex.sock`，環境多一個 `ZDOTDIR=$AGEND_HOME/zsh`；`codex_live` 印 `command -v git → $AGEND_HOME/bin/git`；跑完「你親自驗收」步驟 8 前後，`ls -l ~/.codex/config.toml` 的修改時間相同。
- 另外（不在本關改，列給第 3、6 施工關）：**任何** agent 的工具只要用 login zsh 跑指令，macOS 上都會有同樣的 PATH 重排，claude 的 Bash 工具也要在第 12 施工關實測。
- [ ] 待你確認（請寫明選 A／B／C／D）

### P5：送達模型：狀態代表什麼、冪等放哪、當掉怎麼辦

- 問題：`sent` 與 `confirmed` 各在什麼時候成立？同一個 id 送兩次、daemon 在送出的瞬間當掉，怎麼保證不重複也不遺失？
- 建議：
  - 新 migration `0003_messages`：`messages` 表（`id` 主鍵、`to_instance`、`from`、`body`、`level`、`state`、`turn_id`、時間），保留 30 天（D31，列進第 5 施工關 P8 的規則表）。**這張表就是唯一的一套冪等。**
  - 四個狀態在 codex 的意思：
    - `queued`：寫進 DB 了，還沒拿到 codex 的 RPC 回覆。app-server 連不上（holder 重起中）也停在這裡，連上後照送；**不算失敗**（V1-LESSONS #1）。
    - `sent`：codex 回了 RPC 成功（`turn/start` 的 `turn.id`、`turn/steer` 的 `turnId`、`thread/queue/add` 的 `queuedSubmission`）。
    - `confirmed`：一定先經過 `sent`（core 的 `can_transition_to` 不允許 `queued → confirmed`；對帳時一次補兩步）。thread 裡真的出現這則 user message（`item/completed`，`type: userMessage`）＝ agent 的上下文裡已經有它。對法：有 `clientId` 就比 `clientId == 訊息 id`；沒有就比 `turn_id` ＋ 內容完全相同。
    - `failed`：codex 拒絕（RPC 錯誤且不是 P6 的競態）、instance 被刪或已 `failed`。
    - `sent` 一直等不到確認就停在 `sent`（誠實標未確認），不重送、不改 `failed`。
  - 送出一律帶 `clientUserMessageId = 訊息 id`（v1 在 `turn/start` 就這樣帶）。**未查證**：`turn/start`、`turn/steer` 收不收這個欄位、會不會回在 `clientId`（錄製檔裡只有 `thread/queue/add` 回 `clientId`，U2）。
  - 冪等：`deliver` 先查表；id 已存在就回目前的狀態、不呼叫 codex（DRV-9：再送不是錯誤）。
  - 當掉的窗口（RPC 送出了、`sent` 還沒寫）：開機後每個 `queued` 的列先查兩個地方：**`thread/queue/list`**（排了隊、還沒輪到的訊息不在任何 turn 裡，比 `clientUserMessageId`）與 thread 歷史（P7 的 `thread/turns/list`，比 user message）。在佇列裡 → 補成 `sent`；在歷史裡 → 補成 `sent` 再 `confirmed`；兩邊都沒有才送。只查歷史會把「排隊中」的訊息再送一次（重複一個 turn，違反 DRV-9）。
  - 內容：只送完整 body，前面加兩行標頭 `From: <from>`、`Task: <task id>`（沒有 task 就省略）＋空行。不截斷。
- 理由：一張表同時是冪等、狀態、TUI 顯示的來源；「確認」以 codex 自己的 thread 為準，不是「RPC 回 200」。重送前查歷史，把「崩潰時送一半」這個 v1 標成 `Ambiguous` 的狀態收回四狀態裡。
- 替代方案：冪等放 driver 記憶體（daemon 重啟就忘，契約的 `ObjDedup`）；`sent` 就當 `confirmed`（v1 的假成功）；崩潰窗口直接重送（可能重複一個 turn）。
- 例子：`deliver m-7` 兩次 → 第二次 log `m-7 already confirmed; not sent again`，thread 裡只有一則 `m-7`；daemon 在 `turn/start` 送出後、寫 `sent` 前被硬殺 → 開機 log `m-7 found in thread history (turn 3f2a…); marked sent, confirmed`；`thread/queue/add` 之後被硬殺 → `m-8 found in thread queue; marked sent`。
- [ ] 待你確認

### P6：三級忙碌策略怎麼落到 codex

- 問題：誰決定用哪一級？daemon 以為 agent 在忙、其實剛好閒下來（或反過來）時怎麼辦？D30 的去抖動要怎麼校準？
- 建議：
  - 等級由**呼叫者**傳入（`deliver(…, mode)` 已經是這樣）；本關不做「依緊急程度自動選」，預設 `Queue`。誰在什麼情況用哪一級，由第 8–10 施工關的呼叫點決定（P9）。
  - 「忙不忙」看 driver 自己收到的 `thread/status/changed`（`active`／`idle`），**不經去抖動**：去抖動只給 TUI 顯示用。
  - 閒置：不管哪一級，一律 `turn/start`。
  - 忙碌：
    - `Queue` → `thread/queue/add {clientUserMessageId}`。若回覆時 thread 已經是 `idle`（剛好結束），再呼叫一次 `thread/queue/start`：回 `-32600 … active or pending turn` 表示 codex 已經自己開始了，當成功。**未查證**：閒置 thread 上 `queue/add` 會不會自己開始（U3）。
    - `Steer` → `turn/steer {expectedTurnId}`。回 `-32600`（turn 剛好結束）→ 改送一次 `turn/start`。**未查證**：真 codex 對剛結束的 turn 回的是不是 `-32600`（只有假 app-server 這樣回，U9）。
    - `Interrupt` → `turn/interrupt`，等到 `turn/completed status: interrupted`（最多 5 秒）→ `turn/start`；5 秒沒等到也照送 `turn/start`（spike S3：忙碌時的 `turn/start` 會併進進行中的 turn，最壞變成 steer，不會壞狀態）。
    - 永遠不在 `queue/add` 之後主動 `queue/start`（除了上面「已經 idle」那一種）。
    - `Interrupt` 時 thread 已有排隊的訊息：codex 應該會先開始排隊的那一個，我們的 `turn/start` 併進去（等同 steer；**未查證**，U11，錄製情境 `queue_idle` 一起錄）。本關接受、記一行 log，不另做處理。
  - D30 校準：用錄製檔的真實資料。`busy.jsonl` 裡排隊的下一個 turn 自動開始時，`idle` 只持續 **12 ms**（`1790316814864` → `…876`）；codex 的忙碌狀態來自結構化事件、不會像 v1 讀畫面那樣亂跳。所以 **codex 維持 5 秒**，不改 core；把這筆數字寫進 D30 的來源。
- 理由：等級規則是政策，現在還沒有呼叫點，先做成參數最小；busy 判斷不準也不會壞狀態（spike S3），所以只處理兩個已知競態、各一次改送，不做重試迴圈。
- 替代方案：core 加 `urgency → level` 對照表（沒有使用者，第 10 施工關再看）；送之前先 `thread/read` 問狀態（多一次 RPC，仍有競態）；steer 一律用忙碌時的 `turn/start`（依賴沒寫在文件上的行為）；去抖動改 1 秒（沒有資料支持）。
- 例子：demo 的 `== busy` 段分三次，每次先讓 agent 跑一個長 turn（turn A），再送一則：`m-q queue → thread/queue/add → sent → confirmed (turn A 之後的新 turn)`；`m-s steer → turn/steer → sent → confirmed (turn A 裡)`；`m-i interrupt → turn/interrupt (A interrupted), turn/start → sent → confirmed (新 turn)`。
- [ ] 待你確認

### P7：driver 事件與 cursor：daemon 不在時的事件怎麼補

- 問題：`Driver::events(after_cursor)` 要回「cursor 之後的全部事件，含 daemon 不在時的」（DRV-6）。codex 斷線時的通知不會重播，事件從哪來？
- 建議：
  - **codex 的 thread 歷史就是事件日誌**，daemon 不另存。`events(after)` 呼叫 `thread/turns/list`（分頁讀完），每個 turn 依序展開成：`BusyChanged{true}` → 每則 user message 一個 `MessageConfirmed`（對到 `messages` 表的 id）→ turn 結束時 `TurnCompleted` → `BusyChanged{false}`；還在跑的 turn 只展開到目前為止。
  - cursor ＝ `<turn id>:<在該 turn 裡的序號>`。假設歷史只會往後長（**未查證**：codex 的 rollback、context 壓縮會不會刪改舊 turn，U10；若會，cursor 指到的 turn 不見時改從頭重讀、用 `messages` 表去重），所以同一個 cursor 重讀只會變長（DRV-7），任何舊 cursor 都能接（DRV-6）。
  - 即時通知只拿來更新「現在忙不忙」（P6）與觸發一次 `events` 讀取，不另外算 cursor。
  - **未查證**：`thread/turns/list` 的分頁參數、回來的 user message 有沒有 `clientId`（U5）。假 app-server 目前不支援這個方法，要補（P8）。
- 理由：不必多一張事件表、不必處理「DB 寫了一半」；daemon 重啟後的補回與平常的讀取是同一段程式。
- 替代方案：daemon 把收到的通知寫進 DB 事件表、重連後用 `turns/list` 補缺口（兩個來源要對齊）；讀 `~/.codex/sessions/` 的 rollout 檔（v1 `shadow/rollout.rs` 的做法，綁 codex 的檔案格式）。
- 例子：daemon 停著時，排隊的 `m-q` 自己跑完一個 turn；daemon 再起來、拿停機前最後的 cursor `7c1e…:3` 讀 `events` → 拿到 `BusyChanged{true}`、`MessageConfirmed{m-q}`、`TurnCompleted`、`BusyChanged{false}`，`m-q` 變 `confirmed`。
- [ ] 待你確認

### P8：測試：什麼是假的、什麼是真的

- 問題：driver 對誰測？重啟類契約怎麼跑？真的 codex 要不要跑、誰核准、花多少 token？
- 建議：
  - 預設全部對 `fake-codex-app-server`（真的程序、真的 unix socket、真的 WebSocket）。要補的：`thread/turns/list`；接受 `clientUserMessageId`（照 U2 的結果決定回不回 `clientId`）；閒置 thread 上 `queue/add` 的行為（照 U3）；`thread/resume` 找不到時的錯誤（照 U1）。
  - PTY 裡的 TUI 用新的小假程式 `fake-codex-tui`：印出收到的參數（`agent args: --remote … resume <id>`）然後等著；它**不**連 app-server（daemon 不依賴 TUI 做任何事）。
  - 契約 DRV-1..9 對 `CodexDriver` ＋假 app-server ＋真 DB 跑。DRV-6、DRV-9 用四次開機、跨真的 process（比照第 6 施工關 P4 第 2、3 層）：開機 1 送 `m-1`、`m-2`；開機 2 閒置；兩次開機之間假 app-server 自己跑完排隊的 turn；開機 3 從舊 cursor 補回、再送 `m-1`（不可多一個 turn）；開機 4 檢查。反向檢查「每次開機用新的 `AGEND_HOME`」必須失敗。
  - 一致性檢查：新增錄製情境 `turns_list`、`queue_idle`、`resume_empty`，讓上面三個補丁有真 CLI 的依據。**錄製要跑真的 codex、花少量 token（約 5 個很短的 turn），是「你親自驗收」步驟 7 的選做步驟，由你打勾核准後你自己跑**（`cargo xtask record codex --sandbox …`）；沒錄之前，三個補丁在假 app-server 裡標「未查證」。
  - 真 codex 端到端：做成**選做**的 example `codex_live`（`agend-daemon`），要設 `AGEND_REAL_CODEX=1` 才跑、在 `record-sandbox.sh` 裡跑、CI 永遠不跑；約 3 個很短的 turn。它驗假的驗不到的：`-c` 覆寫與位置、`--remote … resume`、trust 提示不出現、指令找到的是 shim（U4、U6、U7、U8、U12、U13）。
  - 本 agent 與 verifier **都不跑真 codex**。
- 理由：假 app-server 已經照錄製檔對過形狀，driver 的邏輯可以全部在 CI 驗；只有「真 CLI 接不接受這些參數」必須真跑，花費小、由你決定時機。
- 替代方案：CI 跑真 codex（要登入、花錢、不穩）；完全不跑真 codex（U4、U6、U7 只能等第 9 施工關有人真的用才發現）；TUI 也用真 codex（要登入，測試跑不動）。
- 例子：在 `record-sandbox.sh` 裡 `AGEND_REAL_CODEX=1 target/debug/examples/codex_live` → `thread 01a0… created`、`m-1 idle → turn/start → confirmed`、`kill app-server → restart 1/3 resume 01a0…`、`m-2 → confirmed; reply mentions m-1`。
- [ ] 待你確認

### P9：依賴規則、這關不做的事

- 問題：新依賴怎麼擋？哪些看起來相關的東西不在本關？
- 建議：
  - `check-deps` 加規則：`agend-holder` 不能依賴 `tungstenite`（holder 不講 backend 協定）；`agend-daemon` 可以用 `tungstenite`（阻塞版，長連線一條 std thread，第 6 施工關 H7），不加 `tokio-tungstenite`。
  - **不做**（移到後面）：
    - codex approval 轉給人回答（needs-you）→ 第 10、11 施工關；本關一律 `decline`（P4）。
    - client 協定的 `Send` 加 `level`、`agend send` 命令 → 第 8、9 施工關；本關只有 `deliver` API 與 demo。
    - 依任務情況自動選忙碌等級 → 第 10 施工關（P6）。
    - usage limit／rate limit（`account/rateLimits/updated`、螢幕 hard gate）→ 第 10 施工關的 supervisor。
    - codex 升版後的 canary instance（delivery「另外」那段）→ 第 13 施工關。
    - claude、opencode 的 driver 與它們的 session id → 第 12 施工關。
- 理由：規則讓「holder 偷偷講 codex 協定」編譯不過；每一項移出去的都還沒有呼叫點，現在做只能用假資料驗。
- 替代方案：本關先做 approval 轉發（沒有人能回答）；`Send` 現在就加 `level`（第 8 施工關才有 server）。
- 例子：有人在 `agend-holder` 的 `Cargo.toml` 加 `tungstenite`，`cargo xtask check-deps` 失敗：`agend-holder must not depend on tungstenite (the holder never speaks a backend protocol, gate 7 P9)`。
- [ ] 待你確認

### 已知風險（開工時處理）

- 第 6 施工關已 merge（#125，`3e28e06`）：本關要改它的 `supervisor::session_args`、`instances` 的 `new` 意義（P3）與 holder 協定（P2），第 6 施工關的驗收要重跑。
- **shim 在 macOS login zsh 下排到後面（P4）**：不只 codex，任何用 login shell 跑指令的 agent 都一樣；第 3 施工關（shim）與第 6 施工關（PATH 白名單）的頁面沒提到，建議在共用文件補一條。
- `sent` 之後永遠等不到確認（例如 codex 改了 user message 的形狀）只會看到一堆停在 `sent`；demo 與 TUI 要把「`sent` 超過 10 分鐘」顯示出來（開工時細化）。
- 用你的 `~/.codex` 時，你自己的 MCP server、plugin、`notify`、hooks 也會在每個 agent 生效（backends/codex.md 陷阱）；本關不處理（P4），記給第 12 施工關。
- 冪等依賴「同一則訊息的內容不變」：P5 崩潰對帳在沒有 `clientId` 時用內容比對，兩則內容完全相同、不同 id 的訊息在同一個 turn 裡會被當成一則（機率低；U2 若證實有 `clientId` 就沒有這個問題）。

**未查證的 codex 事實**（每條都附你可以自己跑的安全查法；`--help` 與 `generate-json-schema` 不呼叫模型、不花 token。本 agent 沒有執行任何 codex 指令）：

| # | 事實 | 影響 | 怎麼查 |
|---|---|---|---|
| U1 | `thread/start` 之後還沒有任何 turn、app-server 重起，`thread/resume` 找不找得到這個 thread | P3 的「空 thread」分支是否需要 | 真跑才知道：錄製情境 `resume_empty`（P8，花極少 token） |
| U2 | `turn/start`、`turn/steer` 收不收 `clientUserMessageId`，user message 會不會帶 `clientId` | P5 的確認與對帳 | `codex app-server generate-json-schema --experimental -o /tmp/cx && grep -l clientUserMessageId /tmp/cx/*.json` |
| U3 | 閒置 thread 上 `thread/queue/add` 會不會自己開始 turn | P6 的 queue 競態處理 | 真跑：錄製情境 `queue_idle` |
| U4 | 0.156.1 的 `codex --remote … resume <id>` 接不接受 `-c` 覆寫（v1 說 0.148–0.150 拒絕權限覆寫） | P4 的設定放 app-server 還是 TUI | `codex resume --help` 看有沒有 `--remote`、`-c`；實際接受與否要 P8 的 `codex_live` |
| U5 | `thread/turns/list` 的分頁參數與回傳形狀 | P7 | 同 U2 的 schema：`grep -A40 '"ThreadTurnsListParams"' /tmp/cx/*.json` |
| U6 | 帶 `--remote` 的 TUI 會不會另外起或接上 codex 共用的背景 app-server（`--no-daemon`） | P1、P2（程序數） | `codex --help \| grep -i -e remote -e daemon` 與 `codex resume --help \| grep -i daemon` |
| U7 | 0.156.1 的 `-c projects={…}` 仍能讓 trust 提示不出現（v1 在 0.149 驗過） | P4 | P8 的 `codex_live`（看第一個畫面） |
| U8 | codex 的指令（`/bin/zsh -lc`）找到的 `git`、`kill` 是不是 `$AGEND_HOME/bin` 的 shim | P4（安全） | 不花 token 的旁證：`env -i HOME=$HOME PATH=/tmp/x:/usr/bin:/bin /bin/zsh -lc 'echo $PATH'`；真正的答案：`codex_live` 裡跑 `command -v git kill` |
| U9 | `turn/steer` 碰到剛結束的 turn 回 `-32600`（目前只有假 app-server 這樣回） | P6 | 真跑才知道：可加進錄製情境 `queue_idle` |
| U10 | thread 歷史只會往後長（rollback、context 壓縮不刪改舊 turn） | P7 的 cursor | schema 裡找 rollback 類方法：`ls /tmp/cx \| grep -i -e rollback -e compact`；行為要真跑 |
| U11 | `turn/interrupt` 時已有排隊訊息，codex 先開始排隊的那一個 | P6 | 真跑：錄製情境 `queue_idle` |
| U12 | 0.156.1 的 `-c` 放在子命令後面也有效（v1 #3402 放前面） | P4 | `codex app-server --help \| grep -e '-c'`；實際效果要 `codex_live` |
| U13 | codex 的 `shell_environment_policy` 會不會濾掉 `ZDOTDIR`；codex 能不能設成非 login shell | P4 選 A／B | 同 U2 的 schema：`grep -rl -i -e shell_environment_policy -e login /tmp/cx`；實際效果要 `codex_live` |

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-daemon`、`~/.cargo/bin/cargo test -p agend-holder`、`~/.cargo/bin/cargo test -p agend-testkit` 單獨通過，包括：`SpawnSidecar` 只一次、環境是白名單、任一個結束就兩個都停但 holder 留著回報 `Exited`、`Shutdown` 停掉 app-server 的 process group（P2）；thread 先建、先寫 DB 才 `Spawn`、每次都 `resume <id>`（P3）；`-c` 參數組出來的樣子、approval 請求回 `decline`、選定的 shim 方案組出的環境（P4）；四個狀態的轉換、再送同一個 id 不呼叫 codex、崩潰窗口對帳（歷史與佇列兩邊，P5）；三級各一條、兩個競態各一條（P6）；cursor 展開與重讀（P7）
- [ ] 契約 DRV-1..9 對 `CodexDriver` ＋假 app-server ＋真 DB 通過；DRV-6、DRV-9 四次開機跨真的 process 通過，反向檢查「每次開機用新的 `AGEND_HOME`」必須失敗（P8）
- [ ] 第 6 施工關的 `daemon-holder` 驗收仍通過（本關改了 `session_args` 與 holder 協定）
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過），並有新規則：`agend-holder` 不能依賴 `tungstenite`（P9；故意加依賴會失敗）
- [ ] `~/.cargo/bin/cargo xtask accept codex` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 真 CLI 一致性檢查（必要；使用者已決定 2026-09-25）：`codex --version` 和 `crates/agend-testkit/transcripts/codex/` 錄製檔 header 的 `version` 相同，不同就先用錄製器重錄（[RECORDER.md](../../crates/agend-testkit/RECORDER.md#重錄cli-升版時)）；`~/.cargo/bin/cargo test -p agend-testkit --test conformance` 通過（含 P8 新增的三個情境）
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] 測試不留殘留：結束時沒有 `g7-` 或測試 id 的 holder、假 app-server；kill 只對自己起的、大於 1 的 pid
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」（verifier 不跑真 codex）

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

**每個新開的終端機分頁都要先跑這段**：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.x.y`。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

1. 跑 demo（全部對假 codex）。

   **這步在驗什麼**：真 daemon、真 holder、假 app-server 與假 TUI 從頭跑完下面每一段。錯了代表最基本的「起 codex instance、送得到」不成立。

   ```bash
   ~/.cargo/bin/cargo xtask accept codex
   ```

   應該看到：最後一行 `gate 7 (codex): checks passed`（開工時細化）。

   - [ ] 通過

2. 三級忙碌策略。

   **這步在驗什麼**：每一級對到正確的 codex 方法，而且都走到 `confirmed`（P5、P6）。錯了的話緊急訊息會排到最後，或插入變成打斷。

   操作：同一次輸出，找 `== busy`。應該看到：閒置時的一則走 `turn/start`；忙碌時三則分別是 `thread/queue/add`、`turn/steer`、`turn/interrupt, turn/start`；四則都印 `queued → sent → confirmed`，確認的 turn 跟 P6 例子一樣（queue 在長 turn 之後的新 turn、steer 在長 turn 裡、interrupt 讓長 turn 變 `interrupted`）。

   - [ ] 通過

3. 故意弄壞：同一個訊息 id 送兩次，其中一次在 daemon 重啟之後。

   **這步在驗什麼**：只有一套冪等、跨重啟也有效（P5、DRV-9）。錯了的話 agent 會做兩次同一件事（v1 三套去重並存的問題）。

   操作：找 `== idempotent`。應該看到：第二次印 `already confirmed; not sent again`；假 app-server 的 turn 數沒有增加。

   - [ ] 通過

4. 四次開機：daemon 不在時 agent 照跑，回來後補回。

   **這步在驗什麼**：daemon 停著時排隊的訊息自己跑完，daemon 回來從舊 cursor 補回、不漏不重（P7、DRV-6）。錯了的話 daemon 重啟就會漏掉「已確認」或把訊息再送一次。

   操作：找 `== restart`。應該看到：4 行 `boot N`，daemon pid 每行不同、app-server pid 每行相同；開機 3 印 `backfilled 4 events from <cursor>`、`m-q confirmed while daemon was down`；開機 4 `ok`。

   - [ ] 通過

5. 故意弄壞：硬殺 app-server，看同一個 thread 接回。

   **這步在驗什麼**：codex 死掉之後會用**同一個 thread id** resume，不再像第 6 施工關那樣直接 `failed`（P2、P3）。錯了的話 codex agent 一死就失去全部上下文，或要人手動處理。

   操作：找 `== resume`（開工時細化：也可以照第 6 施工關步驟 8 的方式自己動手，在 `probe-sandbox.sh` 裡只砍自己記下的 app-server pid）。應該看到：`sidecar exited; stopping agent`、`restart 1/3 resume <T>`；假 TUI 印 `agent args: … resume <T>`，`<T>` 與開頭 `thread <T> created` 相同；之後送的訊息照樣 `confirmed`。

   - [ ] 通過

6. 真 CLI 一致性檢查（必做；使用者已決定 2026-09-25）。

   **這步在驗什麼**：driver 測試用的假 app-server 和你機器上真的 codex 講同樣形狀的協定（含 P8 新增的三個情境）。壞了的話，driver 對假的全綠、接上真的才出錯（v1 #1483）。

   ```bash
   codex --version
   head -1 crates/agend-testkit/transcripts/codex/one_turn.jsonl
   ~/.cargo/bin/cargo test -p agend-testkit --test conformance
   ```

   應該看到：第一行的版本和錄製檔 header 的 `"version"` 相同；最後 `test result: ok. N passed`（N 開工時細化）。版本不同：先重錄再跑一次（`~/.cargo/bin/cargo xtask record codex --sandbox ~/Documents/Hack/AgEnD-ops/record-sandbox.sh`，會跑真的 codex、花少量 token，細節見 [RECORDER.md](../../crates/agend-testkit/RECORDER.md)）；檢查不過就改假 codex，不改錄製檔。

   - [ ] 通過

7. 選做（要你核准，會跑真的 codex、花約 5 個很短的 turn）：錄三個新情境。

   **這步在驗什麼**：P8 對假 app-server 補的三個行為（`thread/turns/list`、閒置時 `queue/add`、resume 空 thread）和真的 codex 一樣（U1、U3、U5、U9、U11）。不錄的話，這三個補丁只是照猜的寫，步驟 6 對它們沒有依據。

   ```bash
   ~/.cargo/bin/cargo xtask record codex --sandbox ~/Documents/Hack/AgEnD-ops/record-sandbox.sh
   ~/.cargo/bin/cargo test -p agend-testkit --test conformance
   ```

   應該看到：`transcripts/codex/` 多出 `turns_list.jsonl`、`queue_idle.jsonl`、`resume_empty.jsonl`（開工時細化：只錄這三個的參數）；一致性檢查通過。不通過就改假 app-server 與 P3／P5／P6 對應的程式，不改錄製檔。

   - [ ] 我核准花這些 token，已錄製並通過
   - [ ] 這次不做（寫進驗收紀錄；三個補丁維持「未查證」）

8. 選做（要你核准，花約 3 個很短的 turn）：真 codex 端到端。

   **這步在驗什麼**：假的驗不到的事：真 codex 接受 P4 的 `-c` 參數、`--remote … resume <id>` 接得上、trust 提示沒出現、**指令找到的 `git`／`kill` 是 shim**（U4、U6、U7、U8、U12、U13）。錯了的話第 9 施工關第一次有人真的用時才會發現，或 agent 可以繞過 shim。

   ```bash
   ls -l ~/.codex/config.toml
   ~/.cargo/bin/cargo build -q -p agend-daemon --example codex_live
   AGEND_REAL_CODEX=1 ~/Documents/Hack/AgEnD-ops/record-sandbox.sh target/debug/examples/codex_live
   ls -l ~/.codex/config.toml
   ```

   應該看到：`thread <T> created`、`m-1 … confirmed`、`restart 1/3 resume <T>`、`m-2 … confirmed; reply mentions m-1`、`command -v git → <AGEND_HOME>/bin/git`；前後兩次 `ls -l` 的修改時間相同（P4：不寫你的 config；沙箱本來就擋 `~/.codex/config.toml` 與 repo 的寫入，所以先在沙箱外 build）。開工時細化 home 放 `/tmp` 的參數。

   - [ ] 通過
   - [ ] 這次不做（寫進驗收紀錄）

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-26 fresh-context review REFUTED 後修正：P4 改成安全決定（macOS login zsh 的 `path_helper` 會把 shim 排到後面，選項 A–D，新增 U8、U12、U13）；P2 定義 sidecar 環境＝白名單、`Shutdown` 停它的 process group、兩個都結束後 holder 照第 4 施工關 P7 留著；P5 崩潰對帳加查 `thread/queue/list`、`queued` 一定經過 `sent`；P1 長連線改一條 std thread（第 6 施工關 H7）；P3 標明是第 6 施工關 P6 的例外與 H1 的改寫；新增 U9–U11；錄新情境成為你親自驗收的選做步驟 7（共 8 步）；合入第 6 施工關（#125）。
- 2026-09-26 開工前提案 P1–P9 寫定（待你逐題確認）；未查證的 codex 事實 U1–U7 列出查法；你親自驗收改為 7 步（第 7 步選做）；狀態改為提案中（#126）。
- 2026-09-25 使用者決定：真 CLI 一致性檢查（錄製器 + `tests/conformance.rs`）列為必要完成條件，取代選做的 smoke test（`feat/backend-recorder`）。

## 下一步

```bash
cat docs/gates/gate-07-codex.md
~/.cargo/bin/cargo xtask accept codex
```
