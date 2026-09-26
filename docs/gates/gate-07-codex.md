# 第 7 施工關：daemon：codex driver + 送達（`codex`）

> **TL;DR**
> - codex driver、送達模型、三級忙碌策略；codex 第一次有 thread id 可以 resume（補上第 6 施工關 H2 的缺口）。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：逐題決定下面的開工前提案 P1–P9（每題最後一行打勾）；另外請你在自己的終端跑「未查證」表裡的安全指令（不花 token），結果貼回來；U1、U3、U9、U11 沒有安全指令，要真跑 codex（步驟 7，選做、花 token）。P2 推翻第 4 施工關 P8 的 `SpawnSidecar`、P4 是安全決定（shim 可能被繞過），請特別看。第 6 施工關已 merge（#125），確認後即可開工。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**提案中**（2026-09-26）：開工前提案 P1–P9 待你逐題確認（P4 要你明確選 A／B／C／D）；第 6 施工關已 merge（#125，`3e28e06`）。

## 範圍

- 怎麼跟 codex 講話：holder 持有 `codex app-server`（附屬程序）與 codex TUI（PTY），daemon 的 driver 走 app-server 的 JSON-RPC（P1）
- 附屬程序（第 4 施工關 P8 移來）：**不加 `SpawnSidecar`**，app-server 由 PTY 裡的 `sh` 包裝在背景起、跟 TUI 同一個 process group；就緒判斷放 daemon 的 driver（P2）
- codex 的 thread id：daemon 先建 thread、存進 DB，agent 一律用 `resume <id>` 起；補上第 6 施工關 H2「codex 死一次就 `failed`」的缺口（P3）
- codex 的啟動設定：`CODEX_HOME`、trust、sandbox／approval、更新提示（P4）
- 送達模型：訊息 id、`queued → sent → confirmed | failed` 各代表什麼、單一冪等、崩潰後對帳（P5）
- 三級忙碌策略：queue、steer、interrupt 對到 codex 的方法、閒置與競態怎麼處理；D30 去抖動校準（P6）
- driver 事件與 cursor：daemon 不在時的事件怎麼補回（P7）
- 測試：什麼是假的、什麼是真的；DRV 契約四次開機；真 codex 只在你核准時跑（P8）
- `check-deps` 新規則與移到後面施工關的事（P9）

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子。文件已定、這裡不重問的：訊息內容走 backend 的結構化 API、PTY 只送單一控制鍵、只有一套冪等（[delivery](../architecture/delivery.md#送達模型)、V1-LESSONS #1）；送達狀態只有 `queued → sent → confirmed | failed` 四個、轉換規則已寫在 core（`DeliveryState::can_transition_to`）；三級忙碌策略與 codex 對應的方法、「不支援 → 中斷」（[delivery](../architecture/delivery.md#忙碌策略三級)、`policy::busy::effective_level`）；推送帶完整內容（V1-LESSONS #2）；去抖動「轉 busy 立即、轉 idle 穩定 5 秒」（D30，本關只校準）；訊息保留 30 天（D31）；附屬程序由 holder 持有、就緒判斷放 daemon 的 driver（第 4 施工關 P8；P8 的另一半「協定是 `SpawnSidecar`」這裡建議推翻，見 P2）；holder 死掉的 5 秒／3 次／`failed`、絕不自動全新啟動（第 6 施工關 P6）；agent 環境白名單（第 6 施工關 P3、H3）；holder 協定同 major 只加欄位（D26）；重啟類契約用四次開機、跨真的 process（[CONTRACTS.md](../../crates/agend-testkit/CONTRACTS.md)、第 2 施工關 A25）；真 CLI 一致性檢查是必要完成條件（使用者 2026-09-25）。

codex 的事實來源：[backends/codex.md](../backends/codex.md)、[spike-codex](../research/spike-codex.md)（`codex-cli 0.156.1`）、`crates/agend-testkit/transcripts/codex/` 的 5 個錄製檔、v1 `src/transport/codex_app_server.rs`（唯讀）。標 **未查證** 的是這些來源都沒證實的，查法集中在「已知風險」下面的表。

### P1：daemon 怎麼跟 codex 講話

- 問題：daemon 對 codex 送訊息、看狀態，走 app-server 的 JSON-RPC，還是讀 holder 的 PTY 畫面？PTY 裡跑什麼？
- 建議：
  - 一個 codex instance ＝ 一個 holder 裡、同一個 process group 的**兩個程序**：`codex app-server --listen unix://…`（附屬程序，由 P2 的包裝在背景起）＋ PTY 裡的 codex TUI `codex resume <thread id> --remote unix://<真正的 socket>`（P3；子命令在前是 spike S4 驗過的順序）。TUI 只給人 attach 看、給螢幕分類器認 hard gate。
  - daemon 的 `driver::codex` 只經 app-server 講話：WebSocket（`tungstenite`，阻塞 I/O）。每個 instance 一條長連線、**一條自己的 std thread**（比照第 6 施工關 H7 與 `runtime/link.rs`：活得跟連線一樣久的阻塞工作不佔 `spawn_blocking` 的名額）；一次性的呼叫（`thread/start` 等）才用 `spawn_blocking`。連上先 `initialize`，再 `thread/resume {threadId, excludeTurns: true}`（不 resume 就只收到粗粒度狀態，spike S2）。
  - 連線前一律 `realpath` socket 路徑（已有 `socket_connect_path`，陷阱 1）。
  - **不做**：PTY 打字、讀畫面判斷 busy／idle、v1 的「猜 TUI 開了哪個 thread」（`discover_loaded_tui_thread`，多 thread 時會拒絕送）。
- 理由：結構化 API 有訊息 id、turn id、`turn/completed`，是「確認送達」唯一可靠的來源；v1 改走 app-server 後「貼上沒送出」幾乎消失。TUI 還是要有，因為第 11 施工關的 attach 畫面要看得到 agent。
- 替代方案：只跑 app-server、PTY 空著（最簡單，但人 attach 看不到 agent）；讀畫面判斷（v1 的坑，V1-LESSONS #3、#4）；`tokio-tungstenite`（多一個 async 依賴，一條連線用不到）。
- 例子：`ps -o pid,pgid,command` 看到 holder `g7-1` 底下兩個程序、pgid 相同：`codex app-server --listen unix://…/g7-1.codex.sock` 與 `codex resume 01a0d1fc-… --remote unix:///private/tmp/codex-daemon-501/…`；daemon log `g7-1: app-server connected, thread 01a0d1fc resumed (idle)`。
- [ ] 待你確認

### P2：app-server 怎麼跟 TUI 一起放進 holder（**不做 `SpawnSidecar`**）

- 問題：app-server 由 holder 持有（ARCHITECTURE 程序模型），要不要照第 4 施工關 P8 的原計畫給 holder 協定加 `SpawnSidecar`？app-server 拿到什麼環境、怎麼知道好了、死了怎麼辦？
- **方向改變，請明確決定**：第 4 施工關 P8（使用者 2026-09-25 確認）原本說「新增 `SpawnSidecar` 請求」。這份提案建議**不做**，改用下面的包裝程序；這是推翻 P8 的那半句，要你確認。前三輪 review 每一輪都在「獨立的第二個程序」上找到新的一類程序生命週期問題（見替代方案），根本原因是「holder 能像管 PTY 子程序一樣安全地管另一個獨立 process group 的程序」這個假設不成立。
- 建議：
  - **holder 協定不改**。holder 仍只有一個 PTY 子程序，但它是一個很小的 `sh` 包裝：

    ```sh
    codex -c … app-server --listen "unix://$SOCK" >>"$LOG" 2>&1 &
    i=0; while [ ! -s "$GO" ]; do i=$((i+1)); [ $i -gt 300 ] && exit 1; sleep 0.1; done
    exec codex -c … resume "$(sed -n 1p "$GO")" --remote "unix://$(sed -n 2p "$GO")"
    ```

    （確切寫法開工時細化；`$SOCK` ＝ `$AGEND_HOME/run/holders/<id>.codex.sock`、`$GO` ＝ `…/<id>.codex-go`、`$LOG` ＝ holder 的 log。）
  - 非互動的 `sh` 沒有 job control，所以背景的 app-server 跟 TUI 在**同一個 process group、同一個 session**（下面實驗 E1）。第 4 施工關對 agent 已保證的事全部自動涵蓋 app-server：portable-pty 在 `pre_exec` 把 SIGCHLD／HUP／INT／QUIT／TERM／ALRM 設回預設（所以 holder 忽略的訊號不會被繼承）；`Shutdown` 對 group 送 SIGHUP（G2）；結束後保留 zombie、group id 不會被重用（G11）；holder 被 `kill -9` → PTY 掛斷 → 整個 group 收到 SIGHUP（實驗 E2）；24 小時安全網。POSIX 規定非互動 shell 的背景程序忽略 SIGINT／SIGQUIT（本機沒量），我們不用這兩個訊號，無妨。
  - 環境：包裝就是 agent，拿到的就是第 6 施工關 P3、H3 的白名單（沒有 secret）；app-server 與 codex 跑的每個指令都繼承它（P4 的 PATH 問題照樣存在）。
  - 順序與就緒：daemon 在 `Spawn` **之前**刪掉舊的 `$SOCK`（symlink；若它指到 `/private/tmp/codex-daemon-<uid>/` 下的檔案，連那個檔案一起刪，**真 codex 會不會自己清舊 socket 未查證**，U14）與舊的 `$GO` → `Spawn` 包裝 → driver 每 100 ms 試連、`initialize` 成功才算就緒，**20 秒**放棄（v1 `ready_timeout_secs: 20`；放棄＝一次「死掉」，走第 6 施工關 P6）→ 建／接 thread（P3）→ 寫 `$GO`（第 1 行 thread id、第 2 行 `realpath` 後的 socket）→ 包裝 `exec` TUI。包裝等 `$GO` 最多 30 秒，等不到就 `exit 1`（holder 回報 `Exited`，照第 6 施工關 P6）。
  - 一個先結束、另一個還在：
    - TUI 先結束：它是 session leader，kernel 對 group 送 SIGHUP，app-server 跟著結束（實驗 E3，在 zombie 還沒回收時也一樣）；holder 回報 `Exited`，daemon 照第 6 施工關 H9 `Shutdown`（G11 對 group 送 SIGKILL 清掉剩下的）再帶 resume 重起。
    - app-server 先結束：TUI 還在，holder 不知道。driver 的長連線斷了、20 秒連不回來 → 當成死掉：daemon 對 holder 送 `Shutdown`（SIGHUP 整個 group）→ 照第 6 施工關 P6 重起。真 TUI 斷線後自己會不會結束**未查證**（U15），不影響這條規則。
- 理由：零協定變更、零新的程序管理程式碼；app-server 活得跟 TUI 一樣久，是它本來的樣子。v1 的 app-server 是 daemon 的子程序（V1-LESSONS #8），這裡是 holder 的孫程序，daemon 重啟照樣不斷線（D3）。
- 替代方案：
  - `SpawnSidecar`（原計畫，前三輪的設計）：落選。三類問題：① app-server 繼承 holder 忽略的 HUP／INT／QUIT／TERM，要自己在 `pre_exec` 重設；② 結束後要自己保留 zombie，否則對 group 的 SIGKILL 可能打到重用的 pgid；③ **holder 被 `kill -9` 時 app-server 變孤兒**（自己的 group、沒有 PTY、沒人掛斷），新 holder 的 `SpawnSidecar` 甚至會經舊的 socket symlink 連到那個孤兒。每一類都要另寫程式與測試，第 4 施工關對 PTY 子程序已經全部做過。
  - 包裝不 `exec`、自己 `wait` 兩個程序再殺 group：多一段 shell 邏輯，kernel 的掛斷已經做到。
  - 每個 codex instance 共用一個 app-server：一個死全部死、thread 混在一起。
- 例子（對應下面實驗的真實輸出，用 `sleep` 代替 codex）：`tui pid 18121 pgid 18121 sid 18121`、`bg pid 18122 pgid 18121 sid 18121`；holder 程序不清理直接結束 → `tui alive False, app-server stand-in alive False`。真 codex 時：在沙箱裡 `kill -9` holder → `pgrep -f "codex.*g7-1"` 什麼都不印；daemon log `holder g7-1 died`、5 秒後 `restart 1/3 resume 01a0d1fc-…`。
- 實驗（2026-09-26，本機 macOS，`/private/tmp/g7exp/`，Python `pty.fork` 起 `sh -c 'sleep 1001 >>log 2>&1 & exec sleep 1002'`，只用自己起的程序、沒有送任何 kill）：
  - E1：兩個程序 pgid、sid 相同（都等於 TUI 的 pid）。
  - E2：持有 master 的「holder」程序用 `os._exit` 直接結束（像 `kill -9`：kernel 關掉 fd）→ 0.5 秒後兩個都不在。
  - E3：TUI 那一邊先 `exit 3`、master 仍開著、TUI 不回收（像 G11）→ 背景的 stand-in 也結束了（session leader 結束時 kernel 對前景 group 送 SIGHUP）。結束後 `pgrep` 沒有殘留。
- [ ] 待你確認（含「推翻第 4 施工關 P8 的 `SpawnSidecar`」）

### P3：codex 的 thread id 與 resume（補第 6 施工關 H2）

- 問題：第 6 施工關 H2：codex 沒有 session id，第一次起來後一死就 `failed`。thread id 從哪來、存哪、怎麼 resume？
- 建議：
  - **daemon 自己先建 thread**：`instances.session_id` 是 NULL 時，app-server 就緒後 driver 呼叫 `thread/start {cwd}`，拿到 `threadId`，**先寫進 `instances.session_id`**，再寫 `$GO` 讓包裝 `exec` TUI（P2）。
  - 所以 codex 的 TUI **每一次**都是 `resume <thread id> --remote unix://<真正的 socket>` 起，包括第一次（spike S4 的順序；反過來行不行未查證，U4），沒有「新／舊」兩種。codex 的分支看 **`session_id` 是不是 NULL**，不看 `new`／`running`；`new`／`running` 維持第 6 施工關 H1 的意思（第一次 `Spawn` 被確認後才寫 `running`），H1 不必改。第 6 施工關的 `session_args` 對 codex 回傳包裝的參數（P2），thread id 經 `$GO` 給 TUI。
  - 重起：新 holder、新 app-server → driver `thread/resume {threadId}` → TUI `resume <id>`（spike S4：同一個 `CODEX_HOME`，砍光再起仍保有完整上下文）。
  - `thread/resume` 說找不到這個 thread：
    - daemon 從沒對這個 instance 送出過訊息（`messages` 表沒有 `sent` 以上的列，P5）→ 視為「空 thread 沒落地」，建新 thread、覆寫 `session_id`、log 一行 `thread <old> not found and never used; new thread <new>`。**未查證**：codex 是不是要等第一個 turn 才把 thread 寫到磁碟（U1）。
    - **這是第 6 施工關 P6「絕不自動全新啟動」的例外，請你明確決定**：理由是空 thread 沒有上下文可丟。不接受的話，這種情況一律 `failed`（替代方案第 3 個）。
    - 其他情況 → `failed`，交給人（第 6 施工關 P6：絕不丟掉對話重來）。
- 理由：自己建 thread，id 在 agent 起來**之前**就在 DB，daemon 死在任何一步都不會出現「有 agent、不知道 thread」；不用像 v1 那樣從已載入的 thread 裡猜哪個是 TUI 的。第 6 施工關 H2 對 claude 也是「自己給 id」，同一個想法。
- 替代方案：讓 TUI 自己建 thread，daemon 再 `thread/loaded/list` 找（v1 的做法，多 thread 時要拒絕送）；讀 `~/.codex/sessions/` 的 rollout 檔名找 id（綁 codex 的檔案格式）；找不到一律 `failed`（holder 在第一則訊息前死掉也要人處理）。
- 例子：`daemon_probe add g7-1 --codex` 之後開 daemon：`g7-1: holder pid=5230 started`、`g7-1: app-server ready`、`g7-1: thread 01a0d1fc-… created`、`g7-1: go (resume 01a0d1fc-…)`；在沙箱裡砍掉 holder：`restart 1/3`、`thread 01a0d1fc-… resumed`，thread id 不變。
- [ ] 待你確認

### P4：codex 的啟動設定與 **shim 會不會被繞過**（安全決定，請明確選）

- 問題：用哪個 `CODEX_HOME`？trust 提示、sandbox／approval、更新提示怎麼處理？最重要的：codex 跑的指令還會不會先找到 shim？
- **風險先講清楚**：codex 用 `/bin/zsh -lc "<指令>"` 跑每個指令（spike-codex.md 的 S7 payload、`approval.jsonl`）。`-l` 是 login shell，macOS 的 `/etc/zprofile` 會跑 `path_helper`，把 `/etc/paths`、`/etc/paths.d` 列的目錄（`/usr/local/bin`、`/usr/bin`、`/bin`…）排到我們放在最前面的 `$AGEND_HOME/bin` **前面**；`/opt/homebrew/bin` 這類則是你自己的 dotfile（例如 `brew shellenv`）加的，也可能排到前面。reviewer 實測：`env -i PATH=/tmp/fakeshimdir:/usr/bin:/bin /bin/zsh -lc 'echo $PATH'` 印出來 `fakeshimdir` 排在 `/opt/homebrew/bin`、`/usr/bin` 後面。另外 codex 會做 shell 快照（`~/.codex/shell_snapshots`），可能用快照裡的 PATH 蓋掉我們給的（**未查證**，併入 U8、U13）。結果：agent 的 `git`、`pkill`、`killall` 可能直接跑到真的 binary，**shim 的防護（第 3 施工關）對 codex 沒有作用**。（`kill` 本來就是 zsh 內建指令、shim 攔不到，這是已確認的第 3 施工關 T18，由 holder 忽略 TERM 與第 6 施工關的 resume 補救處理，跟 PATH 無關。）PATH 順序只在「放了 `git`／`pkill`／`killall` 的目錄」之間有差：`/usr/bin`、`/opt/homebrew/bin` 這類排到 shim 前面才有問題。再加上 `approval_policy="never"` ＋ `danger-full-access`，codex 自己也不擋。
- 建議（設定部分）：
  - `CODEX_HOME` 用你原本的（`~/.codex`，不設這個變數）：登入資料、rollout 都共用。**daemon 自己絕不寫 `~/.codex/` 裡的任何檔案**（codex 照常寫它的 sessions、log）。
  - 所有設定用**每次啟動的 `-c` 參數**（只影響那個程序）：trust `-c 'projects={"<realpath 後的工作目錄>"={trust_level="trusted"}}'`（v1 #3402）、`-c check_for_update_on_startup=false`（v1 #1626）、app-server 帶 `-c approval_policy="never" -c sandbox_mode="danger-full-access"`（v1 的做法）。
  - `-c` 放在子命令**前面**（`codex -c … app-server --listen …`，v1 #3402 的位置；v1 說 `-c` 在 0.148 是全域選項）。0.156.1 放後面行不行**未查證**（U12）。TUI 在 `resume … --remote` 時接不接受權限覆寫：v1 0.148–0.150 拒絕，0.156.1 **未查證**（U4）。
  - 另一條路（不必靠 `-c`）：`thread/start` 本身收 `approvalPolicy`、`sandbox`（錄製器 `recorder/codex.rs` 就這樣帶）。建議 approval／sandbox 同時用 `-c`（給 app-server 預設）和 `thread/start` 參數（給這個 thread），兩個都有就不怕其中一個被忽略。
  - 你的 `~/.codex/config.toml` 裡的 `notify`、hooks、`mcp_servers` 也會在每個 agent 生效（backends/codex.md 陷阱）。本關**不**覆寫它們（覆寫要逐個列名字、plugin 的關不掉），只記在風險；要不要關由你決定。
  - app-server 仍送來 approval 請求（`item/*/requestApproval`）時：driver 回 `decline`、log 一行 `approval declined (gate 7 has no handler)`。轉給人回答是第 10、11 施工關的事（P9）。
- 建議（shim 部分，請從下面選一個；**我建議 A**）：
  - **A. 自己的 `ZDOTDIR`**：agent 與 app-server 環境加 `ZDOTDIR=$AGEND_HOME/zsh`，裡面的 `.zprofile` 在 `/etc/zprofile`（`path_helper`）**之後**執行，只做一件事：`export PATH="$AGEND_HOME/bin:$PATH"`（再視需要 `source` 你的 `~/.zprofile`，順序放在前面）。**未查證**：codex 的 `shell_environment_policy` 會不會把 `ZDOTDIR` 濾掉（U13）；你的預設 shell 若是 bash，這招不適用（bash 的 login 檔在 `$HOME`）。副作用三個：
    - 設了 `ZDOTDIR`，zsh 就**不讀你的 `~/.zshenv`**（不只 `~/.zprofile`、`~/.zshrc`）；要保留就在我們的檔案裡 `source`，而且 `source` 完再把 shim 放回最前面。
    - 你自己的 dotfile 也會改 PATH：reviewer 的機器上 `~/.zshenv` 把 `~/.cargo/bin` 放到 shim 前面，**連非 login 的 `zsh -c` 也一樣**。所以 B 單獨用也不夠，重排的「最後一步」必須是我們的。
    - 多給 agent 一個 `ZDOTDIR` 是**修改第 6 施工關 H3 的環境白名單**，請一併確認。
  - B. 讓 codex 用非 login shell：codex 有沒有這種設定**未查證**（U13 的同一份 schema／`--help` 查）；而且擋不到你的 `~/.zshenv` 改 PATH（見 A 的副作用），單獨用不夠。
  - C. `-c shell_environment_policy.set.PATH=…`：只設環境變數，login shell 之後還是會被 `path_helper` 與你的 dotfile 重排，**單獨用沒用**，只列出來說明為什麼不選。
  - D. 本關先接受風險：只靠 worktree 的 git hook（第 10 施工關才裝，而且只擋 git 的 ref 更新，不擋 `pkill`／`killall`），並把 U8 的結果記下來再決定。
  - 不管選哪個：`codex_live`（P8）加一個 turn 跑 `command -v git pkill killall`，三個路徑都必須在 `$AGEND_HOME/bin`（U8；`kill` 會印 `kill`＝內建，照 T18 不檢查）；假 app-server 的測試照選定的方案檢查組出的環境。
- 理由：v1 試過獨立 `CODEX_HOME`，登入資料會分岔（`provider_detect.rs` 的註解）；回答 trust 提示會讓 codex 在你的 config 裡每個 workspace 永久加一筆（v1 #3317），`-c` 什麼都不寫。shim 是 v2 對 `git`、`pkill`、`killall` 唯一的防護，codex 又關掉了自己的 sandbox，所以 PATH 順序必須是明確的決定，不能默默假設（比照第 6 施工關 H14，與你已確認的安全前提字面不同的地方要你明確選）。
- 替代方案：每個 instance 自己的 `CODEX_HOME`＋複製 `auth.json`（登入過期要逐個處理）；`workspace-write` sandbox＋逐一核准（第 7 施工關沒有核准的人，agent 會卡住；也不解決 PATH）；預寫 `config.toml`（改到你的檔案）。
- 例子：選 A 時，包裝裡的 app-server 指令是 `codex -c 'projects={"/Users/you/ws/g7-1"={trust_level="trusted"}}' -c check_for_update_on_startup=false -c approval_policy="never" -c sandbox_mode="danger-full-access" app-server --listen unix://…/g7-1.codex.sock`，包裝的環境（也就是 app-server 與 TUI 的環境）多一個 `ZDOTDIR=$AGEND_HOME/zsh`；`codex_live` 印 `git`、`pkill`、`killall` 都在 `$AGEND_HOME/bin/`；跑完「你親自驗收」步驟 8 前後，`ls -l ~/.codex/config.toml` 的修改時間相同。
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
    - `Queue` → `thread/queue/add {clientUserMessageId}`。若回覆時 thread 已經是 `idle`（剛好結束），再呼叫一次 `thread/queue/start`：回 `-32600 … active or pending turn` 表示 codex 已經自己開始了，當成功。**這跟現有的規則衝突，要你核准**：`crates/agend-daemon/src/driver/codex.rs` 開頭的 Must NOT 與 [backends/codex.md](../backends/codex.md) 陷阱都寫「`queue/add` 之後不要呼叫 `queue/start`」；spike 看到的是忙碌時兩者競爭，閒置時的行為沒人測過（U3）。不核准的話：閒置時的 Queue 直接改送 `turn/start`（閒置本來就一律 `turn/start`），只剩「送出瞬間剛好變閒置」的窗口，那則訊息可能一直排在佇列裡，要靠 `sent` 超過 10 分鐘的顯示讓人發現。**未查證**：閒置 thread 上 `queue/add` 會不會自己開始（U3）。
    - `Steer` → `turn/steer {expectedTurnId}`。回 `-32600`（turn 剛好結束）→ 改送一次 `turn/start`。**未查證**：真 codex 對剛結束的 turn 回的是不是 `-32600`（只有假 app-server 這樣回，U9）。
    - `Interrupt` → `turn/interrupt`，等到 `turn/completed status: interrupted`（最多 5 秒）→ `turn/start`；5 秒沒等到也照送 `turn/start`（spike S3：忙碌時的 `turn/start` 會併進進行中的 turn，最壞變成 steer，不會壞狀態）。
    - 永遠不在 `queue/add` 之後主動 `queue/start`（除了上面「已經 idle」那一種，而且要你核准；核准的話 `codex.rs` 的 Must NOT 與 backends/codex.md 陷阱要跟著改寫）。
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
  - 預設全部對 `fake-codex-app-server`（真的程序、真的 unix socket、真的 WebSocket）。要補的：`thread/turns/list`；`thread/queue/list`（P5 的對帳）；接受 `clientUserMessageId`（照 U2 的結果決定回不回 `clientId`）；閒置 thread 上 `queue/add` 的行為（照 U3）；`thread/resume` 找不到時的錯誤（照 U1）；`--listen` 的路徑已有舊 symlink 時先刪掉再綁（目前會 `EEXIST`，`fake_agent/codex.rs` 的 `symlink`；真 codex 的行為見 U14）。沒錄製之前，這些補丁在假 app-server 的程式註解裡標「未查證」，一致性檢查只比現有 5 個情境。
  - PTY 裡跑的是真的 P2 `sh` 包裝，只是 `codex` 換成測試的假程式：app-server 那行起 `fake-codex-app-server`，TUI 那行起新的小假程式 `fake-codex-tui`（印出收到的參數 `agent args: resume <id> --remote …` 然後等著；**不**連 app-server，daemon 不依賴 TUI）。包裝用參數拿到程式路徑（開工時細化）。
  - 契約 DRV-1..9 對 `CodexDriver` ＋假 app-server ＋真 DB 跑。DRV-6、DRV-9 用四次開機、跨真的 process（比照第 6 施工關 P4 第 2、3 層）：開機 1 送 `m-1`、`m-2`；開機 2 閒置；兩次開機之間假 app-server 自己跑完排隊的 turn；開機 3 從舊 cursor 補回、再送 `m-1`（不可多一個 turn）；開機 4 檢查。反向檢查「每次開機用新的 `AGEND_HOME`」必須失敗。
  - 一致性檢查：新增錄製情境 `turns_list`（含 `thread/queue/list`）、`queue_idle`、`resume_empty`，讓上面的補丁有真 CLI 的依據。**錄製要跑真的 codex、花少量 token（約 5 個很短的 turn），是「你親自驗收」步驟 7 的選做步驟，由你打勾核准後你自己跑**（`cargo xtask record codex --sandbox …`）；沒錄之前，三個補丁在假 app-server 裡標「未查證」。
  - 真 codex 端到端：做成**選做**的 example `codex_live`（`agend-daemon`），要設 `AGEND_REAL_CODEX=1` 才跑、在 `record-sandbox.sh` 裡跑、CI 永遠不跑；約 3 個很短的 turn。它驗假的驗不到的：`-c` 覆寫與位置、`resume … --remote`、trust 提示不出現、指令找到的是 shim、包裝在真 codex 下的行為（U4、U6、U7、U8、U12、U13、U14、U15）。
  - 本 agent 與 verifier **都不跑真 codex**。
- 理由：假 app-server 已經照錄製檔對過形狀，driver 的邏輯可以全部在 CI 驗；只有「真 CLI 接不接受這些參數」必須真跑，花費小、由你決定時機。
- 替代方案：CI 跑真 codex（要登入、花錢、不穩）；完全不跑真 codex（U4、U6、U7、U8、U14、U15 只能等第 9 施工關有人真的用才發現）；TUI 也用真 codex（要登入，測試跑不動）。
- 例子：在 `record-sandbox.sh` 裡 `AGEND_REAL_CODEX=1 target/debug/examples/codex_live` → `thread 01a0… created`、`m-1 idle → turn/start → confirmed`、`kill -9 holder → no codex left → restart 1/3, thread 01a0… resumed`、`m-2 → confirmed; reply mentions m-1`。
- [ ] 待你確認

### P9：依賴規則、這關不做的事

- 問題：新依賴怎麼擋？哪些看起來相關的東西不在本關？
- 建議：
  - `check-deps` **不加新規則**：holder 完全不碰 codex（P2 不改 holder），擋 `agend-holder` 依賴 `tungstenite` 已經沒有意義。`agend-daemon` 用 `tungstenite`（阻塞版，長連線一條 std thread，第 6 施工關 H7），不加 `tokio-tungstenite`。
  - `agend-holder/src/sidecar.rs`（只有說明、沒有程式）改寫成指向 P2 的一句話，或刪掉（開工時決定）。
  - **不做**（移到後面）：
    - codex approval 轉給人回答（needs-you）→ 第 10、11 施工關；本關一律 `decline`（P4）。
    - client 協定的 `Send` 加 `level`、`agend send` 命令 → 第 8、9 施工關；本關只有 `deliver` API 與 demo。
    - 依任務情況自動選忙碌等級 → 第 10 施工關（P6）。
    - usage limit／rate limit（`account/rateLimits/updated`、螢幕 hard gate）→ 第 10 施工關的 supervisor。
    - codex 升版後的 canary instance（delivery「另外」那段）→ 第 13 施工關。
    - claude、opencode 的 driver 與它們的 session id → 第 12 施工關。
- 理由：每一項移出去的都還沒有呼叫點，現在做只能用假資料驗。
- 替代方案：本關先做 approval 轉發（沒有人能回答）；`Send` 現在就加 `level`（第 8 施工關才有 server）；保留「holder 不能依賴 `tungstenite`」規則（無害，但擋的是一個已經不存在的設計）。
- 例子：demo 裡假 app-server 送來 `item/commandExecution/requestApproval` → daemon log `approval declined (gate 7 has no handler)`，那個 turn 照常結束；`agend send` 在本關還不存在。
- [ ] 待你確認

### 已知風險（開工時處理）

- 第 6 施工關已 merge（#125，`3e28e06`）：本關要改它的 `supervisor::session_args`（P2、P3），holder 協定不改；第 6 施工關的驗收要重跑。
- **shim 在 macOS login zsh 下排到後面（P4）**：不只 codex，任何用 login shell 跑指令的 agent 都一樣；第 3 施工關（shim）與第 6 施工關（PATH 白名單）的頁面沒提到，建議在共用文件補一條。
- `sent` 之後永遠等不到確認（例如 codex 改了 user message 的形狀）只會看到一堆停在 `sent`；demo 與 TUI 要把「`sent` 超過 10 分鐘」顯示出來（開工時細化）。
- 用你的 `~/.codex` 時，你自己的 MCP server、plugin、`notify`、hooks 也會在每個 agent 生效（backends/codex.md 陷阱）；本關不處理（P4），記給第 12 施工關。
- 冪等依賴「同一則訊息的內容不變」：P5 崩潰對帳在沒有 `clientId` 時用內容比對，兩則內容完全相同、不同 id 的訊息在同一個 turn 裡會被當成一則（機率低；U2 若證實有 `clientId` 就沒有這個問題）。

**未查證的 codex 事實**（每條都附你可以自己跑的安全查法；`--help` 與 `generate-json-schema` 不呼叫模型、不花 token。本 agent 沒有執行任何 codex 指令）：

| # | 事實 | 影響 | 怎麼查 |
|---|---|---|---|
| U1 | `thread/start` 之後還沒有任何 turn、app-server 重起，`thread/resume` 找不找得到這個 thread | P3 的「空 thread」分支是否需要 | 真跑才知道：錄製情境 `resume_empty`（P8，花極少 token） |
| U2 | `turn/start`、`turn/steer` 收不收 `clientUserMessageId`，user message 會不會帶 `clientId`；`thread/queue/list` 回不回 `clientUserMessageId` | P5 的確認與對帳 | `codex app-server generate-json-schema --experimental -o /tmp/cx && grep -l clientUserMessageId /tmp/cx/*.json` |
| U3 | 閒置 thread 上 `thread/queue/add` 會不會自己開始 turn | P6 的 queue 競態處理 | 真跑：錄製情境 `queue_idle` |
| U4 | 0.156.1 的 `codex resume <id> --remote …` 接不接受 `-c` 覆寫（v1 說 0.148–0.150 拒絕權限覆寫）；`--remote` 放在 `resume` 前面（v1 的順序）是否也行（spike S4 只驗過放後面） | P4 的設定放 app-server 還是 TUI | `codex resume --help` 看有沒有 `--remote`、`-c`；實際接受與否要 P8 的 `codex_live` |
| U5 | `thread/turns/list`、`thread/queue/list` 的參數與回傳形狀 | P5、P7 | 同 U2 的 schema：`grep -A40 -e '"ThreadTurnsListParams"' -e '"ThreadQueueListParams"' /tmp/cx/*.json` |
| U6 | 帶 `--remote` 的 TUI 會不會另外起或接上 codex 共用的背景 app-server（`--no-daemon`） | P1、P2（程序數、group） | `codex --help \| grep -i -e remote -e daemon` 與 `codex resume --help \| grep -i daemon` |
| U7 | 0.156.1 的 `-c projects={…}` 仍能讓 trust 提示不出現（v1 在 0.149 驗過） | P4 | P8 的 `codex_live`（看第一個畫面） |
| U8 | codex 的指令（`/bin/zsh -lc`）找到的 `git`、`pkill`、`killall` 是不是 `$AGEND_HOME/bin` 的 shim（含 shell 快照會不會蓋掉 PATH） | P4（安全） | 不花 token 的旁證：`env -i HOME=$HOME PATH=/tmp/x:/usr/bin:/bin /bin/zsh -lc 'echo $PATH'`；真正的答案：`codex_live` 裡跑 `command -v git pkill killall` |
| U9 | `turn/steer` 碰到剛結束的 turn 回 `-32600`（目前只有假 app-server 這樣回） | P6 | 真跑才知道：可加進錄製情境 `queue_idle` |
| U10 | thread 歷史只會往後長（rollback、context 壓縮不刪改舊 turn） | P7 的 cursor | schema 裡找 rollback 類方法：`ls /tmp/cx \| grep -i -e rollback -e compact`；行為要真跑 |
| U11 | `turn/interrupt` 時已有排隊訊息，codex 先開始排隊的那一個 | P6 | 真跑：錄製情境 `queue_idle` |
| U12 | 0.156.1 的 `-c` 放在子命令後面也有效（v1 #3402 放前面） | P4 | `codex app-server --help \| grep -e '-c'`；實際效果要 `codex_live` |
| U13 | codex 的 `shell_environment_policy` 會不會濾掉 `ZDOTDIR`；codex 能不能設成非 login shell；shell 快照（`~/.codex/shell_snapshots`）怎麼用 PATH | P4 選 A／B | 同 U2 的 schema：`grep -rl -i -e shell_environment_policy -e login -e snapshot /tmp/cx`；實際效果要 `codex_live` |
| U14 | app-server 綁 socket 時，舊的 symlink 或 `/private/tmp/codex-daemon-<uid>/` 下的舊 socket 檔還在，會不會失敗（假 app-server 目前 `EEXIST`） | P2 的「先刪舊 socket」 | 真跑才知道：`codex_live`（第二次啟動前不刪，看錯誤）；P2 先照最保守的「一律先刪」做 |
| U15 | app-server 先結束時，真 TUI 會不會自己結束；app-server 會不會攔 SIGHUP（攔了就不跟 TUI 一起結束） | P2 的「一個先結束」規則 | 真跑：`codex_live`；規則本身不依賴答案（daemon 20 秒後 `Shutdown` 整個 group，SIGKILL 兜底） |

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-daemon`、`~/.cargo/bin/cargo test -p agend-holder`、`~/.cargo/bin/cargo test -p agend-testkit` 單獨通過，包括：包裝起的 app-server 與 TUI 同一個 process group；holder 被測試自己 `kill -9` 後沒有殘留的假 app-server；TUI 先結束時 app-server 跟著結束；app-server 先結束時 daemon 20 秒後 `Shutdown`；舊 socket 與 `$GO` 在 `Spawn` 前被刪（P2）。訊號重設、zombie 保留、`Shutdown` 的 group SIGHUP／SIGKILL 由第 4 施工關既有的 holder 行為與測試涵蓋（portable-pty `pre_exec`、G2、G11），本關不另寫；thread 先建、先寫 DB 才 `Spawn`、每次都 `resume <id>`（P3）；`-c` 參數組出來的樣子、approval 請求回 `decline`、選定的 shim 方案組出的環境（P4）；四個狀態的轉換、再送同一個 id 不呼叫 codex、崩潰窗口對帳（歷史與佇列兩邊，P5）；三級各一條、兩個競態各一條（P6）；cursor 展開與重讀（P7）
- [ ] 契約 DRV-1..9 對 `CodexDriver` ＋假 app-server ＋真 DB 通過；DRV-6、DRV-9 四次開機跨真的 process 通過，反向檢查「每次開機用新的 `AGEND_HOME`」必須失敗（P8）
- [ ] 第 6 施工關的 `daemon-holder` 驗收仍通過（本關改了 `session_args`）
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過；本關不加新規則，P9）
- [ ] `~/.cargo/bin/cargo xtask accept codex` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 真 CLI 一致性檢查（必要；使用者已決定 2026-09-25）：`codex --version` 和 `crates/agend-testkit/transcripts/codex/` 錄製檔 header 的 `version` 相同，不同就先用錄製器重錄（[RECORDER.md](../../crates/agend-testkit/RECORDER.md#重錄cli-升版時)）；`~/.cargo/bin/cargo test -p agend-testkit --test conformance` 通過（現有 5 個情境；P8 的新情境只有在步驟 7 錄了之後才加進來，不是完成條件）
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

   **這步在驗什麼**：真 daemon、真 holder、真的 `sh` 包裝，裡面是假 app-server 與假 TUI，從頭跑完下面每一段。錯了代表最基本的「起 codex instance、送得到」不成立。

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

5. 故意弄壞：硬殺 holder，看 app-server 沒變孤兒、同一個 thread 接回。

   **這步在驗什麼**：holder 死掉時 app-server 跟著結束、不留孤兒（P2），新的一組用**同一個 thread id** resume，不再像第 6 施工關那樣直接 `failed`（P3）。錯了的話會有沒人管的 app-server 佔著 socket，或 codex agent 一死就失去全部上下文。

   操作：找 `== resume`（開工時細化：也可以照第 6 施工關步驟 8 的方式自己動手，在 `probe-sandbox.sh` 裡只砍自己記下的 holder pid）。應該看到：`holder g7-… killed -9 by test`、`no fake-codex-app-server left`、`restart 1/3`、`thread <T> resumed`；假 TUI 印 `agent args: … resume <T>`，`<T>` 與開頭 `thread <T> created` 相同；之後送的訊息照樣 `confirmed`。

   - [ ] 通過

6. 真 CLI 一致性檢查（必做；使用者已決定 2026-09-25）。

   **這步在驗什麼**：driver 測試用的假 app-server 和你機器上真的 codex 講同樣形狀的協定（現有 5 個情境）。壞了的話，driver 對假的全綠、接上真的才出錯（v1 #1483）。

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

   **這步在驗什麼**：假的驗不到的事：真 codex 接受 P4 的 `-c` 參數、`resume <id> --remote …` 接得上、trust 提示沒出現、**指令找到的 `git`／`pkill`／`killall` 是 shim**（`kill` 是內建，T18）（U4、U6、U7、U8、U12、U13）。錯了的話第 9 施工關第一次有人真的用時才會發現，或 agent 可以繞過 shim。

   ```bash
   ls -l ~/.codex/config.toml
   ~/.cargo/bin/cargo build -q -p agend-daemon --example codex_live
   AGEND_REAL_CODEX=1 ~/Documents/Hack/AgEnD-ops/record-sandbox.sh target/debug/examples/codex_live
   ls -l ~/.codex/config.toml
   ```

   應該看到：`thread <T> created`、`m-1 … confirmed`、`restart 1/3 resume <T>`、`m-2 … confirmed; reply mentions m-1`、`command -v git pkill killall` 三行都在 `<AGEND_HOME>/bin/`；前後兩次 `ls -l` 的修改時間相同（P4：不寫你的 config；沙箱本來就擋 `~/.codex/config.toml` 與 repo 的寫入，所以先在沙箱外 build）。開工時細化 home 放 `/tmp` 的參數。

   - [ ] 通過
   - [ ] 這次不做（寫進驗收紀錄）

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-26 第三輪 review REFUTED（holder `kill -9` 時獨立的 app-server 變孤兒、新 holder 會經舊 socket 連到它）後**改方向**：P2 不做 `SpawnSidecar`，改用 PTY 裡的 `sh` 包裝在背景起 app-server（同一個 process group，第 4 施工關的保證全部涵蓋；本機實驗 E1–E3）；推翻第 4 施工關 P8 的那半句要你確認。另修：必要的一致性檢查不再依賴選做的步驟 7；P5 的 `thread/queue/list` 進假 app-server 補丁與 U2、U5；codex 分支改看 `session_id` 是否 NULL、H1 不改；`path_helper` 與 dotfile 各加哪些目錄；shell 快照；新增 U14、U15；P9 不加 check-deps 規則。
- 2026-09-26 第二輪 review REFUTED 後修正：`kill` 是 zsh 內建（第 3 施工關 T18），U8 改查 `git`／`pkill`／`killall`；sidecar 在 `pre_exec` 重設 holder 忽略的訊號並附 SIGTERM 測試、結束後保留 zombie（G11）；TUI 停止改照 G2 送 SIGHUP；選項 A 的三個副作用（不讀 `~/.zshenv`、使用者 dotfile 也會重排、修改 H3 白名單）；閒置時 `queue/start` 標明與 `codex.rs`、backends/codex.md 衝突、要核准；TUI 參數改 spike S4 順序 `resume <id> --remote …`。
- 2026-09-26 fresh-context review REFUTED 後修正：P4 改成安全決定（macOS login zsh 的 `path_helper` 會把 shim 排到後面，選項 A–D，新增 U8、U12、U13）；P2 定義 sidecar 環境＝白名單、`Shutdown` 停它的 process group、兩個都結束後 holder 照第 4 施工關 P7 留著；P5 崩潰對帳加查 `thread/queue/list`、`queued` 一定經過 `sent`；P1 長連線改一條 std thread（第 6 施工關 H7）；P3 標明是第 6 施工關 P6 的例外與 H1 的改寫；新增 U9–U11；錄新情境成為你親自驗收的選做步驟 7（共 8 步）；合入第 6 施工關（#125）。
- 2026-09-26 開工前提案 P1–P9 寫定（待你逐題確認）；未查證的 codex 事實 U1–U7 列出查法；你親自驗收改為 7 步（第 7 步選做）；狀態改為提案中（#126）。
- 2026-09-25 使用者決定：真 CLI 一致性檢查（錄製器 + `tests/conformance.rs`）列為必要完成條件，取代選做的 smoke test（`feat/backend-recorder`）。

## 下一步

```bash
cat docs/gates/gate-07-codex.md
~/.cargo/bin/cargo xtask accept codex
```
