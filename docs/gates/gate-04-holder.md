# 第 4 施工關：agend-holder（`holder`）

> **TL;DR**
> - 每個 instance 一個 holder：在 PTY 裡跑 agent、記住畫面、回報結束碼，並且**活過 daemon 重啟**（D3）。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：開工前提案 P1–P9 已由使用者確認（2026-09-25）；等第 3 施工關完成（D22）後開工。

## 狀態

**提案中**（2026-09-25）：P1–P9 使用者已確認，等第 3 施工關完成後開工。

## 範圍

- `agend holder <instance-id>` 子命令：以 PTY 啟動 agent、注入環境（P1、P8）
- 脫離終端、只由協定 `Shutdown` 停止；孤兒安全網（P2）
- `$AGEND_HOME/run/holders/` 的 socket、鎖檔、log；不重複、判斷存活（P3）
- holder 協定 server：版本協商、單一連線、快照接串流、落後斷線（P4）
- 畫面維護與純文字快照（P5）
- 可寫進 PTY 的三種位元組（P6）
- exit code 回報與 `Shutdown`（P7）
- 附屬程序（例如 codex app-server）**不在本關**，移到第 7 施工關（P8）

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子。文件已定、這裡不重問的：JSON Lines + base64、`hello` 版本協商、新 daemon 要能跟舊一個 major 的 holder 溝通（D26）；重連給現成畫面、不重播位元組（D3）；訊息內容不在 PTY 打字（delivery）；每個 instance 一個 holder、與 daemon 同一個 binary（D11）。

### P1：holder 是什麼樣的程序

- 問題：holder 用什麼指令跑？要不要 tokio？
- 建議：`agend holder <instance-id>` 子命令，在 argv[0] 分派之後、CLI 解析最前面分出去，不讀設定、不開 DB。內部用 std thread（讀 PTY、寫 PTY、接 socket、等 agent 結束），不用 tokio。`check-deps` 加一條：`agend-holder` 不能依賴 async runtime、SQLite 或 `agend-daemon`。探測 client 做成 `agend-holder` 的 example（`holder_probe`），不做 production 命令。
- 理由：holder 一跑好幾天、很少更新，零件越少越穩；一個 holder 只有一個 PTY、一條連線，thread 就夠。v1 的 `write_actor.rs` 1,304 行，是因為一個 daemon 管所有 PTY。
- 替代方案：用 tokio（多一個大依賴）；獨立 binary（違反 D11）。
- 例子：`ps` 看到 `agend holder dev-1`；shim 的 T9 認得它是 agend 程序。
- [x] 使用者確認（2026-09-25）

### P2：holder 怎麼活過 daemon（含第 3 施工關 T18）、怎麼不變孤兒

- 問題：daemon 結束、關分頁、Ctrl-C 時 holder 怎麼不跟著死？agent 用 shell 內建 `kill` 砍 holder 怎麼辦？holder 越難殺，怎麼避免孤兒？
- 建議：
  - holder 一啟動就 `setsid()`：自己的 session 與 process group，沒有控制終端。stdin 接 `/dev/null`，stdout／stderr 寫 `run/holders/<id>.log`。
  - 忽略 SIGHUP、SIGINT、SIGQUIT、SIGTERM；只由協定 `Shutdown` 停。SIGKILL 擋不了（D3 已寫明的上限）。
  - `kill -9` 之後的補救（偵測 holder 死了 → 用 `--resume <id>` 把 agent 接回）交給第 6 施工關。
  - 防孤兒（使用者追加）：
    - 第 4 施工關：holder 安全網：`AGEND_HOME` 被刪，或 agent 已結束且**連續 24 小時**沒有人連上 → 自行退出。
    - 第 6 施工關：daemon 開機巡查 `run/holders/` 鎖檔，DB 裡沒有的 instance → 送 `Shutdown`。
    - 第 9 施工關：`agend doctor` 列出所有 holder，標出孤兒。
    - 測試不留殘留（P9）。
  - 殭屍：holder 收 agent；daemon 收 holder；daemon 不在時由 launchd／init 收。
- 理由：daemon 會因升級、改設定、當掉而重啟，agent 手上的工作（跑到一半的測試、還沒 commit 的改動、讀過的脈絡）不該因此中斷（v1 問題 #7）。`kill -9 -1` 同 uid 下沒有任何做法擋得住，只能事後補救。
- 替代方案：收到 SIGTERM 就停（agent 一個 `kill $PPID` 就停掉自己的 holder，Ctrl-C 殺掉所有 agent）；另一個 uid 跑 agent（要 sudo、太重）。
- 例子：前景 daemon 按 Ctrl-C，daemon 停了，`agend holder dev-1` 和裡面的 claude 還在；agent 打 `kill $PPID`，holder 也還在。
- [x] 使用者確認（2026-09-25，含防孤兒四點與 24 小時）

### P3：run 目錄、不重複、判斷存活

- 問題：socket 放哪？怎麼避免同一個 instance 起兩個 holder？怎麼從外面判斷還活著？
- 建議：
  - `$AGEND_HOME/run/holders/`（0700），每個 holder 三個檔：`<id>.sock`、`<id>.lock`、`<id>.log`。holder 活著期間持有 `<id>.lock` 的 `flock`，內容是 holder 的 pid。
  - 拿不到 lock → 印 `holder for <id> already running (pid N)`，exit 1；拿到了 → 刪掉舊 socket、重新 bind。
  - 「在跑」＝ lock 被持有，而且 socket 連得上。
  - socket 路徑超過 100 bytes 就拒絕啟動，印出路徑與長度（macOS 上限 104）。
  - `HolderHandle.process_id` 填 holder 的 pid。
- 理由：程序死掉時 kernel 自動放掉 flock，不會有「pid 檔還在、程序已死」或 pid 重用誤判；RTM-3／RTM-4 要求 recover 回來的 pid、socket 與啟動時相同。鎖檔也是 P2 開機巡查的依據。
- 替代方案：只看 socket 檔（程序死了檔案還在）；pid 檔（pid 重用會誤判）。
- 例子：連續兩次 `agend holder dev-1`：第二個拿不到 lock，印出第一個的 pid 後結束。
- [x] 使用者確認（2026-09-25）

### P4：連線規則

- 問題：daemon 重連時送什麼、依什麼順序？舊連線半開怎麼辦？daemon 讀太慢怎麼辦？
- 建議：
  - 同時只服務一條連線；新連線進來就關掉舊的。**踢掉的是舊連線，不是 holder**（holder 只有一個，見 P3）。
  - 每條連線：`hello` → 回選定版本 → `ScreenSnapshot` → 持續 `PtyBytes`；agent 已結束則快照後接 `Exited`。
  - 快照與位元組同一條佇列、同一把鎖產生：不漏、不重。
  - 佇列超過 1 MiB（client 太慢）就斷線，client 重連拿新快照。
  - 版本不合：回 `Error` 並斷線，holder 照常跑。`Snapshot` 請求隨時可送。
- 理由：舊 daemon 當掉時連線可能半開，「新的接手」保證新 daemon 一定連得上；讀太慢就斷線，agent 不會因 daemon 慢而卡住。
- 替代方案：拒絕第二條連線（卡住的舊 daemon 擋住新 daemon）；多條連線（用不到）。
- 風險（依第 3 施工關威脅模型接受）：同 uid 的 agent 可以連 socket 搶走連線。
- 例子：探測 client 看到 `counter=12` 時被砍；重連後快照是 `counter=15`，接著串流送 16、17……
- [x] 使用者確認（2026-09-25）

### P5：畫面與快照

- 問題：快照放什麼？畫面多大？scrollback 留多少？
- 建議：alacritty_terminal 維護畫面，預設 50 列 × 200 欄（spawn 後可 `Resize`）。`ScreenSnapshotData.screen` 放可見畫面純文字（每列去行尾空白、`\n` 連接，約 10 KB）。scrollback 1,000 列、只在記憶體、這關不送出。給 TUI 的 ANSI 版本到第 11 施工關以新增欄位加上（D26）。
- 理由：第 6 施工關與螢幕分類器只需要文字，ANSI 色碼夾在字中間會讓比對失敗；200 欄讓長提示不換行；1,000 列約 5 MB／holder（v1 的 10,000 列約 48 MB）。
- 替代方案：現在就送 ANSI 版（沒人用）；scrollback 0；80×24。
- 例子：bash 計數器的快照：`counter=15`，下一行 `$`。
- [x] 使用者確認（2026-09-25）

### P6：可寫進 PTY 的位元組

- 問題：skeleton 寫「PTY 只收單一控制鍵」，但協定有操作者輸入，TUI 程式也會送終端查詢。到底哪些可以寫？
- 建議：只有三種，都經過同一條寫入 thread（佇列上限 64，滿了回 `Error{code: "pty_busy"}`，不卡住）：
  1. `SendControlKey` 的按鍵位元組（第 1 施工關追認的 `control_key_bytes`）；`Unknown` 回 `Error{code: "unknown_control_key"}`，一個 byte 都不寫。
  2. `OperatorTerminalInput`：人 attach 時打的字，原樣轉送。
  3. alacritty 產生的終端查詢回覆（例如游標位置 `ESC[6n`），沿用 v1 `PtyWriteListener`。

  skeleton 的 `Must NOT` 改成列出這三種。**開工前**在沙箱用真的 codex／claude 確認「不回覆終端查詢會不會卡住」。
- 理由：TUI 程式啟動時會查游標位置，沒人回可能卡住；寫入放獨立 thread，agent 不讀 stdin 時只卡那條 thread（v1 實測卡過 9.7 秒）。
- 替代方案：不回終端查詢（真 backend 可能卡在啟動）。
- 例子：daemon 送 `esc`，PTY 收到 `0x1b`；送不認得的鍵，holder 回錯誤，PTY 一個 byte 都沒收到。
- [x] 使用者確認（2026-09-25）

### P7：agent 結束之後、怎麼停掉 holder

- 問題：agent 結束時 holder 要不要跟著走？`Shutdown` 做哪些事？
- 建議：
  - agent 結束後 holder 留著最後的畫面與結束狀態，每次有人連上就在快照後送 `Exited`，直到 `Shutdown`（加上 P2 的 24 小時安全網）。
  - `Exited` 新增欄位 `signal`（被訊號殺掉時填訊號名，`code` 為 `None`）。這是 core 協定的新增欄位，要重跑第 1 施工關測試。
  - `Shutdown`：agent 還在 → 關 PTY master（agent 收 SIGHUP）→ 等 5 秒 → 對 agent 的 process group 送 SIGKILL（group id＝agent pid，holder 自己 spawn、一定大於 1）→ 刪 socket、放 lock → exit 0。
  - holder 絕不自己重啟 agent。
- 理由：agent 在 daemon 重啟空檔結束時，新的 daemon 仍拿得到結束碼。
- 替代方案：agent 一結束 holder 就走（結束碼會遺失）。
- 例子：bash `exit 7`；你隔一分鐘才連上，仍看到 `exited code=7`。
- [x] 使用者確認（2026-09-25）

### P8：Spawn 的環境與附屬程序

- 問題：agent 拿到哪些環境變數？附屬程序這關做不做？
- 建議：agent 的環境**只有** `Spawn` 帶來的 `env`（先全部清空），沒給時補 `TERM=xterm-256color`。附屬程序整個移到第 7 施工關（新增 `SpawnSidecar` 請求，就緒判斷放 daemon 的 driver）。
- 理由：daemon 環境可能有 secret（例如 Telegram token），繼承就漏給 agent；附屬程序第 7 施工關才有使用者，才驗得到真正的坑（socket realpath、路徑長度）。
- 替代方案：繼承再覆蓋（漏 secret）；照 ROADMAP 在本關做（沒人用、驗不到）。
- 例子：daemon 環境有 `TELEGRAM_BOT_TOKEN`；agent 裡 `env | grep TELEGRAM` 什麼都不印。
- [x] 使用者確認（2026-09-25）

### P9：測試：真的程序、真的重啟、測試不傷人

- 問題：D3 要的是「跨程序還活著」，同一個程序裡的測試分不出來（第 2 施工關 A25）。第 4 施工關怎麼測？
- 建議：
  - 跨程序測試放 `crates/agend/tests/holder_process.rs`，用 `CARGO_BIN_EXE_agend` 跑真的 binary；`xtask accept holder` 的 crates 加上 `agend`。
  - 四次開機，每次是一個獨立的探測程序：啟動器起 holder 後自己結束；開機 1 連上、拿快照、送鍵；開機 2 只連上；開機 3 檢查計數器變大再送鍵；開機 4 確認 pid、socket 與第一次相同。任兩次開機之間沒有探測程序活著。
  - RTM-1..9 對真的 Runtime 跑是第 6 施工關的事；本關提供 `is_running` 判斷（P3）與 recover 回相同 pid／socket。
  - 安全：每個測試自己的 `AGEND_HOME`（instance id 要短）；停止一律 `Shutdown`；保底清理只對「這個測試從 lock 檔讀到、且大於 1」的 pid 送 SIGKILL；絕不用 `-1`、`0`、負數或 `pkill`；結束時檢查沒有殘留 holder；故意砍 holder 的探測只砍自己起的 pid、在 `probe-sandbox.sh` 裡跑。
- 理由：holder 的核心承諾就是跨程序；等到第 6 施工關才驗，D3 最關鍵的性質會晚兩關。
- 替代方案：本關只做同程序測試。
- 例子：開機 1 看到 pid 4242、`counter=3`；開機 4 看到 pid 4242、`counter=11`。
- [x] 使用者確認（2026-09-25）

### 已知風險（開工時處理）

- Linux systemd 預設 `KillMode=control-group` 會在重啟 daemon 時殺掉整個 cgroup（含 holder），`setsid` 也逃不掉 → 第 13 施工關的 unit 必須 `KillMode=process`，並實測 launchd（使用者 2026-09-25 決定現在就寫進第 13 施工關）。
- 終端查詢回覆（P6）與 200 欄排版還沒用真 CLI 驗過。
- macOS socket 路徑 104 bytes：測試暫存路徑 + `run/holders/` + `<id>.sock` 約 97，instance id 要短。
- 螢幕分類器的 fixture 不是 holder 產生的（違反 #1493「用真的 producer」）：本關加一個測試，把錄下的 PTY 位元組餵給 holder 畫面再跑 `classify`。
- `check-deps` 的 async 禁用清單含 `mio`：確認 alacritty_terminal、portable-pty 的依賴樹沒有 mio／tokio。

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-holder` 單獨通過
- [ ] `~/.cargo/bin/cargo test -p agend --test holder_process` 通過：四次開機、啟動器結束後 holder 還在、重複啟動被拒絕、`Shutdown` 後沒有殘留程序（P9）
- [ ] core 協定新增 `signal`（P7）：`~/.cargo/bin/cargo test -p agend-core` 與 xtask protocol compatibility tests 重跑通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過），並包含新的 `agend-holder` 規則（開工時細化：故意加 tokio 依賴會失敗）
- [ ] `~/.cargo/bin/cargo xtask accept holder` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新；skeleton 的 `Must NOT` 依 P6 改好；名詞表加上 `run/holders`、holder lock
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」（verifier 的 kill 探測只對自己起的 pid、在沙箱裡跑）

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 跑 demo。

   **這步在驗什麼**：holder 包著一個每秒加一的 bash 計數器，下面各段都跑完。錯了代表 holder 最基本的「持有 agent、給畫面」不成立。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo xtask accept holder
   ```

   應該看到：最後一行 `gate 4 (holder): checks passed`。

   - [ ] 通過

2. 啟動它的程序結束後，holder 還在。

   **這步在驗什麼**：D3 的核心：daemon（這裡用啟動器代替）結束後 holder 與 bash 繼續跑。錯了的話，每次重啟 daemon 所有 agent 都會死（v1 問題 #7）。

   操作：同一次輸出，找 `== detach`。應該看到：`launcher exited`、`holder alive: pid <N>`，`counter` 還在增加。

   - [ ] 通過

3. 故意弄壞：探測 client 中途被砍，再重連。

   **這步在驗什麼**：重連拿到的是當下畫面，不是從頭來或重播。錯了的話 daemon 重啟後看到舊畫面，或 agent 被重啟。

   操作：找 `== reconnect`。應該看到：斷線前 `counter=A`、重連後 `counter=B`，B > A；前後 `bash pid` 相同。

   - [ ] 通過

4. 送鍵，以及不認得的鍵。

   **這步在驗什麼**：PTY 只收列舉過的按鍵（P6）。錯了的話 daemon 可能把不該送的東西打進 agent。

   操作：找 `== keys`。應該看到：`sent y`、畫面出現 `got y`；`sent unknown -> error unknown_control_key`、`pty bytes written: 0`。

   - [ ] 通過

5. 故意弄壞：同一個 instance 再起一個 holder。

   **這步在驗什麼**：同一個 instance 不會有兩個 holder 搶同一個 agent（P3）。錯了的話重啟 daemon 時會多出 holder。

   操作：找 `== duplicate`。應該看到：`holder for demo-1 already running (pid <N>)`、`exit=1`，第一個 holder 還活著。

   - [ ] 通過

6. 故意弄壞：agent 用 shell 內建 `kill` 砍自己的 holder（T18）。

   **這步在驗什麼**：agent 打 `kill $PPID`（預設 TERM）砍不掉 holder（P2）。錯了的話犯錯的 agent 能把自己連同 holder 弄死。

   操作：找 `== agent-kills-parent`。應該看到：`agent ran: kill -TERM <holder pid>`，接著 `holder alive: pid <N>`。

   - [ ] 通過

7. 四次開機。

   **這步在驗什麼**：不只撐過一次重啟，四次都撐過，其中一次開機什麼都沒做（P9）。錯了的話只撐得過一次重啟的實作會混過去。

   操作：找 `== lifecycle`。應該看到：4 行 `boot N: pid <同一個> counter=<越來越大>`。

   - [ ] 通過

8. agent 結束，以及停止。

   **這步在驗什麼**：結束碼照實回報、重連也拿得到；`Shutdown` 後什麼都不留（P7）。錯了的話結束碼遺失，或留下沒人管的 holder。

   操作：找 `== exit`、`== shutdown`。應該看到：`exited code=7`；重連後仍 `exited code=7`；`shutdown`、`socket gone`、`lock free`、`holder gone`。

   - [ ] 通過

9. 你自己動手：關掉啟動 holder 的終端機分頁（開工時細化）。

   **這步在驗什麼**：真的終端機裡，關分頁（SIGHUP）或 Ctrl-C 都殺不掉 holder；第 6 施工關「Ctrl-C 停 daemon」靠的就是這點。

   操作（示意）：`AGEND_HOLDER_DEMO_KEEP=1` 跑 demo，印出 `export AGEND_HOME=…` 與一行 `holder_probe start demo-2 -- …`；新分頁貼上執行後關掉那個分頁；再開新分頁 `holder_probe snapshot demo-2`。應該看到：`counter` 還在增加。最後 `holder_probe shutdown demo-2`。

   - [ ] 通過

10. 故意弄壞：硬殺 holder，看已知上限（沙箱裡跑，只砍自己起的 pid）。

    **這步在驗什麼**：D3 寫明的上限：holder 被 `kill -9`，裡面的 agent 一起死（補救是第 6 施工關的事）。錯了的話（bash 還活著）代表 agent 沒被 holder 持有，停止時會留孤兒。

    操作（開工時細化）：只對步驟 9 印出、且大於 1 的 holder pid，在 `probe-sandbox.sh` 裡 `kill -9`。應該看到：`holder_probe snapshot demo-2` 回 `connect failed`，demo 自己的標記程序找不到。

    - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-25 開工前提案 P1–P9 寫定，使用者逐題確認（P2 追加防孤兒四點、24 小時安全網）；附屬程序移到第 7 施工關；systemd `KillMode=process` 記入第 13 施工關；狀態改為提案中。

## 下一步

```bash
cat docs/gates/gate-04-holder.md
~/.cargo/bin/cargo xtask accept holder
```
