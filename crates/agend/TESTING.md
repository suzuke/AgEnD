# agend 測試

> **TL;DR**
> - 整合測試直接執行建好的 binary；shim 與 git hook 的真 repo 測試（`tests/shim_*.rs`）也在這裡，因為 git 以獨立程序執行 hook，需要真的 `agend` binary。
> - 記住：argv[0] 分派用真的 symlink 驗證，不是呼叫函式。
> - CLI（第 9 施工關）：`tests/cli.rs` 跑真的 `agend` binary，比對 stdout、stderr、exit code；下面的 `CLI-n` 表每列對假 daemon（`FakeDaemon::start_at($AGEND_HOME/run/daemon.sock)`）與真 `agend daemon` 各跑一次（第 10 施工關才有 handler 的只對假）；各段與 `cli_demo` 共用 `tests/common/`。
> - holder 的跨程序測試（`tests/holder_process.rs`）與 daemon ↔ holder 的測試（`tests/holder_runtime.rs`、`tests/daemon_process.rs`，第 6 施工關）也在這裡，因為要用真的 binary。

## 第 10 施工關驗證

`cargo build -p agend -p agend-testkit --bins` 後跑 `cargo test -p agend --test pipeline`：happy、checks 返工、review 返工、WIP、main 前進、兩個 failpoint 的四次開機、沙箱、hook／cancel、問答／提醒、排隊與 no-role、sandbox retry、merge-blocked；完整 demo 用 `cargo xtask accept pipeline`。`pipeline_context` 以暫停 reviewer 驗 headless review 的等待／重啟／回報，避免自動 reviewer 搶先完成；`pipeline_archive*` 的九個 target 共 31 個回歸，涵蓋大 binary、merge-only 解法、staged-only bytes、隱藏 index 旗標與 stat cache、顯示設定、ignored 檔案，以及保存失敗不刪原 WIP／index。行尾／內容轉換、未解衝突與 nested Git metadata（含不完整狀態）須保留原資料；一般子目錄、空檔與 symlink 用真 git apply 還原。各組內容見 [daemon TESTING](../agend-daemon/TESTING.md)。 content-filter producer 先建立有效 stat cache，再用真 git add --renormalize 強制套用 attributes；不依賴檔案 timestamp 的競態碰巧觸發 clean filter。

## 第 11 施工關 C 段（已驗收並合併 #145）

`cargo test -p agend --test terminal_runtime` 使用真 binary／holder／PTY 驗 runtime frame／control 配對、實際 resize、舊 owner 拒絕、取消 native blocked input 後憑證失效與 holder 重連、不重送、取消已到但未接收的 grant、唯讀查詢取消不打斷控制、整份超限拒絕及新 holder generation。取消案例先以自有 holder lock 核 PID，SIGSTOP 後以 `ps` 核已停止，保證第一次 poll 為 Pending；RAII 在正常與 panic 路徑 SIGCONT，不與 runtime monitor 搶 child exit receipt。真 native 查詢／grant 才會進取消路徑，不依賴真 holder 回覆速度。這些尚不代表 daemon client 1.4／TUI／Codex U17 已完成。

terminal_capability 與 terminal_hub 驗能力／權限、控制／尺寸／歷史、EOF 與資源清理；六項 C 契約同跑 fake／native，完整 App 與 U17 已通過。版本許可與 resume 歸屬的 12 個 native cases 只用 fake backend，真模型不在 CI 執行。 [版本政策](../../docs/gates/gate-11c-codex-input.md)。

`tui_native_app` 的兩個情境經完整 App／真 daemon／holder 到 raw PTY 程序，逐 byte 核鍵鼠／paste 與超限拒絕，agent 內 stty size 核 resize，多視窗交接、>1,000 列歷史／clamp、alt 與 daemon 重啟不自動控制。20 次開關每次 thread／fd 回基準；[證據與重跑](../../docs/gates/gate-11c-native-app-validation.md)。

`tui_outer_pty` 在真外層 PTY 執行真正 `agend app`，經 crossterm capture 驗鍵鼠／paste、kernel resize、多視窗與歷史；正常／panic unwind 後核原 termios、alt／mouse／paste／focus／cursor／SGR 還原。20 次 App 程序退出回同一 fd 基準，holder pid 保留；共用 `tests/common/native_app.rs` 的 raw agent，不使用真 LLM。另有 12 次 burst 的端到端可見 deadline，每次 ≤300 ms、不加 holder round-trip 額度；起點在 producer trigger 前，終點由外層 parser 首次解析出目標文字時記錄，另列 producer acknowledgment 與觀察迴圈返回時間，避免把輸出後 stty／檔案回報成本誤當顯示延遲；800 ms 取樣 mutant 被同一斷言拒絕。[外層證據](../../docs/gates/gate-11c-outer-validation.md)。

`codex_u17` 的兩個 foundation cases 用明確 opt-in raw fake frontend，驗同 thread 人工 turn、busy／queue／獨立 receipt、相同人工文字不能確認未嘗試送出的 row；component restart 保留 holder／thread，舊 attach 拒絕。本批另加入完整 daemon 子程序／client／App 的重啟、草稿、scope／caller 與 durable turn id，以及 attempted crash-window 人工同文拒絕。12 個 tests 含一個 re-exec 入口，沒有真 Codex LLM；[範圍與反例](../../docs/gates/gate-11c-u17-validation.md)。

新增 `resume_notification_requires_persistent_attribution_before_the_rpc_reply`：真 holder／wrapper／raw fake frontend 先產生並保存 clientId=null 的人工同文 turn，再以預設關閉的 fake-only `agendFake/replayUserOnNextResume` 在下一次 resume response 前重播原 item/completed。測試核 producer item／turn 身分、實際 replay count=1 與一次性關閉；重接到拒絕版本仍不得取人工 receipt。沒有合成人工 item／frame，不呼叫真模型。

## 怎麼跑

```bash
cargo test -p agend
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `tests/argv0_dispatch.rs::version_prints_the_package_version` | `--version` 印出 `agend <版本>` |
| `tests/argv0_dispatch.rs::unknown_command_fails_with_usage_hint` | 未知命令 exit 2 並提示 `agend --help` |
| `tests/argv0_dispatch.rs::invoked_as_git_reaches_the_shim` | 名為 `git` 的 symlink 進入 shim，不會走 CLI：`git --version` 交給真的 git；沒有 binding 時 `git worktree add` 被拒絕（exit 1、附下一步） |
| `tests/shim_gh.rs` | 六個 native cases：GraphQL CR／block strings、重複 approve 值與 atomic 完成訊號回歸；拒絕不執行／不記 payload、原 argv／cwd／exit、操作者與 bypass、缺 gh／PATH loop、三種 backend holder 的 PATH 與清理；無 GitHub 網路／token／模型。 |
| `tests/shim_*.rs` | agend-shim 的真 repo 測試（shim 導向與拒絕、git hook、快照、kill 形式）；每個檔案證明什麼見 [agend-shim/TESTING.md](../agend-shim/TESTING.md#測試分類真-repocratesagendtests) |
| `tests/holder_runtime.rs` | 第 6 施工關契約第 1、2 層：RTM-1..9 對真的 `HolderRuntime` + 真 holder；測試 binary 重新執行自己的四次開機（四個 pid、holder pid 不變、計數器變大）；反向：每次開機換新的 `AGEND_HOME` 必須在開機 2 失敗 |
| `tests/daemon_process.rs` | 第 6 施工關契約第 3 層與 P1、P3、P6：真 `agend daemon` 四次開機（開機 3 被測試 `kill -9`）、反向檢查、agent 一直死 → 3 次 `--resume` 後 `failed`、daemon 死在第一次 `Spawn` 前 → 下次仍 `--session-id`（不 resume 沒建立的 session）、agent 環境白名單與 shim 在 PATH 最前、第二個 daemon 10 秒後被拒、孤兒 holder 下次開機被 `Shutdown`、沒有 `AGEND_HOME` exit 1。各段內容與 `daemon_probe demo` 共用（見 [agend-daemon TESTING](../agend-daemon/TESTING.md)） |
| `tests/client_protocol.rs` | 第 8 施工關：CLP-1..12 除 CLP-8 對真 `agend daemon`（每條一個新 home、1 個在跑 + 6 個 `failed` 的 instance）；CLP-8（2000 個事件的慢 client）對同一份 daemon server 程式在測試程序裡跑；socket 0700／0600、開機計畫做完才 bind（一個 holder 鎖被持有但不回應的 instance 讓計畫等 5 秒：socket 出現的第一刻連上，看到的全貌裡下一個 instance 已經起來）、`kill -9` 後舊 socket 被換掉、Ctrl-C 刪檔、101 bytes 的路徑拒絕且不建 `agend.db`（P1）；`retry` 依 backend 與 `session_started` 帶 `--resume`／`--session-id`／不帶，先 `Shutdown` 留著的 holder（每個 instance 只起一次 holder、沒有 start failed、沒有 restart），opencode 跑過的沒有 `retry`（codex 在第 7 施工關後可以 resume），`failed` 的終端只回最後畫面（P5、P6）；在跑的終端先畫面後 `terminal_bytes`、不存在的 instance `no_terminal`、`command` 回 `not_supported`；`agend debug ping --count 12` 跨 daemon 重啟全部 ok、有一行 `retried`，`agend debug watch` 重連後重拿全貌；對只講 1.0 的假 daemon 立刻失敗；沒有 daemon 時 10–13 秒後的訊息；`debug` 的參數與 `AGEND_HOME`。各段與 `client_demo` 共用（`agend-daemon/tests/common/client_process.rs`） |
| `tests/codex_process.rs` | 第 7 施工關：真 `agend daemon`、真 holder、`sh` 包裝，codex 是本 crate 的 example `fake_codex`。第一次啟動先建 thread 再交接、app-server 與 TUI 同一個 process group、TUI 印 `resume <thread>`；daemon 跑著時 `kill -9` holder → 清掃、`restart 1/3`、同一個 thread；codex 忽略 SIGHUP 時 holder 死掉留下的 app-server 被清掃 SIGKILL（daemon 跑著、以及 daemon 停著時下次開機）；TUI 一直死 → 3 次都 resume 同一個 thread、`failed`、沒有 codex 留下（TUI 結束時 app-server 跟著結束）；app-server 死掉 20 秒後算死亡、舊 holder 收到 `Shutdown`、新的接回同一個 thread；第一次啟動在 `thread/start` 前 daemon 被 `kill -9` → 下次開機建 thread、等著的包裝接手、不是 `failed`；schema v3 的第 6 施工關 codex 列（holder 活著）→ `failed`＋`legacy_no_thread`、holder 不動、沒有連 app-server，`new` 的照常起；`failed`、holder 還活著、`agent_pid` 有值且 argv 有 thread 標記的 instance，開機不清掃（開機改成一律清掃時這條失敗）。各段與 `codex_demo` 共用（`agend-daemon/tests/common/codex_process.rs`） |
| `tests/cli.rs::every_cli_row_holds_against_the_fake_and_the_real_daemon` | 下方 `CLI-n` 表每一列；成功只寫 stdout、失敗只寫 stderr，`--json` 只印一個 JSON 值、stderr 空 |
| `tests/cli.rs::a_lost_send_is_resent_with_its_id_and_nothing_else_is` | P5：代理丟掉 `send` 的回應、假 daemon 重啟 → CLI 用同一個 `message_id` 重送、印 `(retried …)`、exit 0，收件者只有一則；`instance add` 同樣情況 → `agend: daemon restarted during the request; check with agend instance list`、exit 1，daemon 只收到一次 |
| `tests/cli.rs::restart_keeps_the_pid_and_the_holders` | P7：`agend daemon restart` → 預檢三行、`the daemon is back: pid <同一個>`；daemon log `recovered=1`；`boot_id` 變了、holder pid 不變；它的預檢 home（daemon log `preflight home /tmp/agend-pf-…`）已刪掉 |
| `tests/cli.rs::a_failed_preflight_changes_nothing` | P7：`--binary` 是 `/usr/bin/false`（`exited with status 1`）、會在 stderr 說錯的 script（`exited with status 3: db copy: broken on purpose`）、`/usr/bin/true`（`without reporting its steps`）、不存在的路徑（`cannot run`）→ `preflight_failed`、exit 1；`agend.db` 位元組完全相同、同一個 pid 與 `boot_id`、log 沒有 `exec` |
| `tests/cli.rs::one_restart_at_a_time` | P7：預檢 3 秒的 binary 在跑時第二個 restart → `invalid_request: a restart is already in progress`；第一個失敗之後再 restart 成功 |
| `tests/cli.rs::restart_waits_for_eof_and_a_new_boot_id` | P7（假 daemon ＋代理）：舊連線一直不關 → 30 秒後 `did not stop within 30 s`、exit 1；新 daemon 的 `boot_id` 跟舊的一樣 → 10 秒後 `did not come back within 10 s`、exit 1；正常 → `the daemon is back` |
| `tests/cli.rs::inherited_holders_are_reaped_and_nothing_else_is` | P7：`exec` 前起的 holder 被移除後 log `reaped inherited holder pid <H>`、`ps` 看不到殭屍；`exec` 後自己起的 holder 的 wait thread 記到 `exited: exit status …`（不是 `wait:` 錯誤）；之後 `--binary /usr/bin/false` 仍是 `exited with status 1`（預檢子程序的 exit status 沒被搶） |
| `tests/cli.rs::init_and_doctor` | P3、P8、P9（`PATH` 只有 stub：`git`、`claude` 印版本，codex／opencode 不在；真的 backend 不會被執行）：沒設 `AGEND_HOME` 的 `init` exit 2、什麼都不建；`init` 建 home 0700、doctor 每一列、`next:`；再跑 `already exists`；git 2.30.0 → `fail … older than 2.38`、`fix:`、exit 1；`--json` 八項、非 ok 都有 `fix`；home 0755 → `warn`、`chmod 700`；`fleet.yaml` → `init` 與 doctor 都拒絕；假 daemon 有 codex instance、PATH 沒有 codex → `fail  codex … used by g9-c`；一個假 daemon 不認識的 holder → `warn  holders … 1 orphan: g9-orphan` |
| `tests/cli.rs::milestone_two_codex_agents_across_a_restart` | 里程碑（P1、P5、P7）：真 daemon、兩個用 `fake_codex` 的 codex instance（`agend instance add … --program fake_codex -- --turn-ms 200`）互送 10 則；第 5 輪的 `send g9-b a5` 經代理、回應被丟掉，接著 `agend daemon restart`，CLI 重送 → `(retried …)`；兩邊 `inbox` 剛好 10 則、各一次；daemon log 20 則都 `confirmed`；重啟失敗時先停止自有 daemon 並附上 stderr，保留原本 10 秒連線要求；停掉 daemon 後 DB 每列 `confirmed`、每則在收件者的 codex thread 裡剛好一個 user message（`clientId`） |
| `tests/cli.rs::ctrl_c_during_a_preflight_leaves_nothing` | verifier F1：預檢子程序在跑時對 daemon 送 SIGINT → 它的暫存 home（DB 複本）被刪、子程序被 daemon kill 並收屍（log `preflight pid <P>: stopped`） |
| `tests/cli.rs::the_preflight_deadline_holds` | verifier F2：`sleep 8 & exit 1` 的預檢在背景程序結束（8 秒）前就回 `exited with status 1`（不等背景程序關 stdout）；永遠不結束的預檢 60 秒後 `did not finish within 60 s`、子程序已結束、暫存 home 都刪掉，下一次 restart 照常跑 |
| `tests/cli.rs::oversized_messages_and_lines_are_refused` | verifier F3（L17）：1 MiB + 1 的 body → `invalid_request`（訊息寫上限）、剛好 1 MiB 可以；超過 8 MiB 的一行 → `invalid_request` 並關連線；之後的小訊息照常送達 |
| `tests/cli.rs::large_messages_to_codex_are_delivered` | 第 2 輪 verifier #1：兩個 `fake_codex` agent，先一則小的（thread 的第一則），再連續送 3 則 512 KiB（比 macOS、Linux 的 unix socket 緩衝區都大）→ 都 `accepted`、都 `confirmed`，之後 `instance remove` 照常 |
| `tests/cli.rs::a_stuck_codex_app_server_never_wedges_the_daemon` | 同上，但 app-server 是 `fake_codex --disable duplex-io`（寫的時候不讀）：`instance remove` 仍在幾秒內完成、Ctrl-C 仍停得掉 daemon（沒有修之前 remove 沒有回應、Ctrl-C 沒反應） |
| `tests/cli.rs::a_binary_swapped_during_its_preflight_is_refused` | 第 2 輪 verifier #3：預檢跑的時候把 binary 原地改一個 byte（大小不變）、把修改時間設回去 → `changed while its preflight ran`、daemon 沒被換掉 |
| `tests/cli.rs::ctrl_c_during_a_restart_stops_the_daemon` | 預檢通過（`preflight passed`）之後馬上 Ctrl-C：daemon 結束、沒有 `exec`（修之前 Ctrl-C 被舊的 image 吃掉、新的照樣跑） |
| `tests/cli.rs::unreachable_daemon_after_ten_seconds`、`an_older_daemon_is_refused_at_once`、`version_starts_fast` | 沒有 daemon：至少等了 10 秒、訊息寫 `after 10 s`（`cannot reach … Start it with: agend daemon`）、exit 1（不設上限：機器忙時量到的是機器，不是 agend），`--json` 的 code 是 `daemon_unreachable`；說 1.1 的假 daemon：只說了一次 hello（沒有重試，在 daemon 那邊數，不量時間）、`this agend needs 1.3 — stop the daemon (Ctrl-C) and start this binary: agend daemon`、`version_mismatch`；`agend --version` 50 次的中位數 < 10 ms |
| `tests/client_protocol.rs`（第 9 施工關部分） | CLP-13..17 對真 daemon（見 testkit CONTRACTS）；`debug` 沒設 `AGEND_HOME` 改成跟其他命令同一句、exit 2；終端那段裡操作者送 agent 命令 `status` 現在是 `forbidden` |
| `tests/client_protocol.rs`（第 11 施工關 B 段部分） | CLP-18..20 對真 daemon：再訂一次終端拿到有新輸出的畫面、不存在的 instance `no_terminal` 且舊串流停止、`terminal_input` 先查身分、`no_terminal`、操作者的位元組到 PTY（畫面回顯）；CLP-10 改成對沒有終端的 instance 送 `terminal_input` → `no_terminal` |
| `tests/terminal_line_limits.rs` | 第 11 施工關 B 段 × 第 9 施工關 L17：終端路徑上最長的行（holder 1000×1000 全是 4 bytes 字元的畫面 4 MB、預設 50×200 約 40 KB、8 KiB 的 PTY 塊、最長的按鍵 `terminal_input`）都小於 `MAX_LINE_BYTES`；client 送的只有很短的行，大的行是 daemon → client，不受 8 MiB 限制 |
| `tests/tui_daemon.rs` | 第 11 施工關 B 段：`App` 經 `ClientSource` 接真 `agend daemon`（home `/tmp/g11.t-<pid>-<n>`，一個計數的 agent、一個一起來就死的）：首頁 `沒有進行中的目標`、約 15 秒後不按鍵自己出現 `需要你 · 1`（帶 `新`）；展開有「不處理的話」與 `[1] 重試`，看過不消失、`1` → `已送出：重試 …`、`需要你 · 0`、daemon log `retry requested by the operator`；終端 `即時`、不按鍵計數器增加、`L` 換英文；`i` 輸入、`hello` 回顯、`Ctrl-]` 回到即時；`AGEND_INSTANCE` 的 app 打字與 `retry` 都 `forbidden`；daemon 重啟時斷線畫面、重連回到終端、計數器接著跑、首頁項目由全貌帶回；`agend app` 沒設 home 與非終端都 exit 2。codex instance（`fake_codex`）的 `terminal_input` → `not_supported`（fake 預設 0.158.0 未獲開放）。各段與 `examples/tui_real.rs` 共用（`tests/common/tui_process.rs`） |
| `tests/holder_process.rs` | `agend holder`：啟動器結束後 holder 還在、四次獨立開機看到同一個 holder、重複啟動 exit 1、agent 的 TERM／HUP／INT／QUIT 無效、安全網、路徑太長拒絕（細節見 [agend-holder TESTING](../agend-holder/TESTING.md)） |

## CLI-n 表（第 9 施工關 P10）

「兩者」＝假 daemon 與真 daemon 各跑一次、預期相同；「只對假」＝沿用第 9 施工關的假 daemon 案例；第 10 施工關真 pipeline 由 `tests/pipeline.rs` 驗證；「—」＝不需要 daemon。`A`＝`AGEND_INSTANCE=g9-a`、`B`＝`g9-b`；兩邊都有 `g9-a`、`g9-b` 兩個 claude instance。完整的比對字串在 `tests/common/cli_table.rs`。

| 列 | 誰 | 命令 | 預期 | 跑在 |
|---|---|---|---|---|
| CLI-1 | — | `--version` | `agend 0.0.0`、exit 0 | — |
| CLI-2 | — | `status`（`AGEND_HOME` 與 `HOME` 都未設） | `agend: AGEND_HOME is not set and HOME is not an absolute path; … export AGEND_HOME=<absolute path>`、exit 2 | — |
| CLI-3 | — | `status`（`AGEND_HOME=g9-relative`） | `AGEND_HOME must be an absolute path`、exit 2 | — |
| CLI-4 | — | `status`（home 有 `fleet.yaml`） | `looks like an AgEnD v1 home (fleet.yaml); set AGEND_HOME to another directory`、exit 1 | — |
| CLI-5 | 操作者 | `send g9-b` | clap 的訊息＋`example: agend send dev-2 …`、exit 2 | — |
| CLI-6 | 操作者 | `send g9-b --json` | `{"error":{"code":"usage",…}}`、exit 2 | — |
| CLI-7 | — | `--help` | 第一段是 `Examples:` | — |
| CLI-8 | 操作者 | `status` | `daemon: pid …, client protocol 1.3`、`instances: 2 (…)`、`needs you: 0` | 兩者 |
| CLI-9 | A | `status` | `g9-a (claude): no task`、`next: agend inbox \| …` | 兩者 |
| CLI-10 | A | `instance add x claude` | `agend: forbidden: only the operator can add instances; ask the operator`、exit 1 | 兩者 |
| CLI-11 | 操作者 | `done t-1/work/1` | `agend: forbidden: agend done is an agent command; it runs inside an agent, where AGEND_INSTANCE is set`、exit 1 | 兩者 |
| CLI-12 | 操作者 | `done t-1/work/1 --json` | 一行 `{"error":{"code":"forbidden",…}}`、stderr 空、exit 1 | 兩者 |
| CLI-13、14 | A | `send g9-b "hi b"`、`send g9-b urgent --level steer` | `accepted: message <uuid> to g9-b (queue)`／`(steer)` | 兩者 |
| CLI-15 | B | `inbox` | `<id> from g9-a: hi b`、`<id> from g9-a: urgent` | 兩者 |
| CLI-16 | B | `inbox --after <不存在的 uuid>` | `agend: unknown_message: … run agend inbox without --after`、exit 1 | 兩者 |
| CLI-17 | A | `send nobody hi` | `agend: unknown_instance: no instance nobody`、exit 1 | 兩者 |
| CLI-18 | 操作者 | `instance add g9-new claude --program /bin/sh -- -c "sleep 600"` | `added g9-new (claude, session …, …/workspace/g9-new); starting` | 兩者 |
| CLI-19 | 操作者 | `instance list` | 表頭 `NAME    BACKEND  STATE …  DIR`、`g9-new  claude   …` | 兩者 |
| CLI-20 | 操作者 | `instance add g9-new claude` | `agend: instance_exists: name g9-new is already used`、exit 1 | 兩者 |
| CLI-21 | 操作者 | `instance add Bad_Name claude` | `agend: invalid_request: invalid name "Bad_Name": …`、exit 1 | 兩者 |
| CLI-22 | 操作者 | `instance remove g9-new`（stdin 不是終端） | `… needs --yes when not on a terminal`、exit 2 | — |
| CLI-23、24 | 操作者 | `instance remove g9-new --yes` 兩次 | `removed g9-new; workspace kept at …`；第二次 `unknown_instance`、exit 1 | 兩者 |
| CLI-25 | 操作者 | `task cancel t-1` | `agend: invalid_request: unknown task t-1`、exit 1 | 兩者 |
| CLI-26、27 | 操作者 | `task create --role dev "login page"`（沒有／有 `--team web`） | `needs --team <team>`、exit 2；`invalid_request: unknown team web`、exit 1 | — |
| CLI-28、29 | A | `done t-42`、`remind 1d` | `invalid ticket …`、`invalid delay …`、exit 2 | — |
| CLI-30 | 操作者 | `daemon restart --binary /usr/bin/false` | `agend: preflight_failed: /usr/bin/false daemon preflight exited with status 1; the daemon keeps running agend 0.0.0…`、exit 1 | 兩者 |
| CLI-31 | 操作者 | `daemon restart` | `preflight agend 0.0.0 (…):`、`  db copy: …`、`restarting the daemon (pid …) ...`、`the daemon is back: pid …, client protocol 1.3, instances=2 (… s)` | 兩者 |
| CLI-32 | B | `inbox`（重啟之後） | 還是那兩則 | 兩者 |
| CLI-33..35 | A | `done t-1/work/1`、`block …`、`ask …` | `done`／`block` 對未知 task 回 `invalid_request`；`ask` 建立持久化對話、exit 1 | 只對真 |
| CLI-36 | A | `status`（假 daemon `assign t-42 review 2`） | `t-42 · review (attempt 2) · ticket t-42/review/2` | 只對假（第 10 施工關） |
| CLI-37、38 | A | `review approve t-42/review/2` 兩次 | `accepted`；第二次 `agend: stale_result: …`、exit 1 | 只對假（第 10 施工關） |
| CLI-39..46 | A | `done`、`result`、`review changes`、`ask --option …`、`block`、`unblock`、`remind 30m`、`task create` | `accepted`／`asked A-…`／`created T-…` | 只對假（第 10 施工關） |

## 用到的假實作

- `agend_testkit::tempdir::TempDir`
- `tests/shim_common` 的 `Fixture`（見 agend-shim/TESTING.md）
- `agend_testkit::fake_daemon::FakeDaemon`（`start_at` 綁在 `$AGEND_HOME/run/daemon.sock`，CLI 照正常路徑找到它）、`contract::client::proxy::Proxy::start_at`（丟掉一個回應、不關舊連線、改 `boot_id`）、`fake_agent::codex::Probe`（讀 codex thread 的歷史）

## 還沒測的

- [x] 所有 CLI 命令、doctor、init（第 9 施工關：`tests/cli.rs`）
- [ ] TTY 上 `instance remove` 的確認問題（測試的 stdin 不是終端；只驗了非 TTY 沒 `--yes` 回 exit 2）
- [ ] doctor 的 `disk` fail／home 超過 20 GB、`home` 不能寫：規則是 `agend_core::setup` 的常數，沒有做出小磁碟或 20 GB 的 home（D24 的「每個 doctor 檢查都有故意弄壞的測試」在第 13 施工關補齊）

`pipeline_attention_events` 經真 daemon／SQLite／Git 與 Rust CLI watch 驗 timeout 不重建核准項目、Approve／RequestChanges 真 action 各一次、新返工 attempt 才重新要求核准、stage 順序及 single merge。SQLite trigger 拒絕舊的 secondary note 寫入時，Approve／RequestChanges 仍各一次發布真 action、持久化決定並完成 single merge；不靠再重啟恢復。

C 段 CLP 拒絕案例同跑真 parser-backed fake 與真 daemon：agent caller 的 Acquire／Resize／Input／Release 全部 forbidden；之後核尺寸不變、原 owner 輸入仍可實收、拒絕 bytes 沒有進 consumer。

U17 live 工具以 itemsView: full 分頁取完整 items。0.159.3 首次四回合已由獨立 verifier 核實，第四回合只核 receipt；使用者同意只開放 0.159.3。這次開放不增授權模型或 merge。 [版本政策](../../docs/gates/gate-11c-codex-input.md)。

## 下一步

```bash
cargo test -p agend
```

## 第 12A Claude helpers

`cargo test -p agend --test claude_bridge` 是本批可自動重驗入口。原 21 個 bridge 回歸另接完整 DRV 十個案例、新 HOME 反向、Git task／review 與遺失 Esc completion；`boot_child` 只作重新執行的 child 入口，不是獨立通過證據。包含 MCP parse／schema 錯誤後下一請求仍可處理；ACK 磁碟保存失敗回工具錯誤；通知收到後以 MCP ping 做 Written 完成 barrier，不能把 stdout 到達當成 SQLite commit。Fixture 停自己 daemon／holders 並刪 home；scope 與指令見 [bridge 基礎](../../docs/gates/gate-12a-bridge.md)。

DRV fixture 的 actor readiness hint 不代替 daemon idle：送達前讀實際 Driver BusyChanged 紀錄。新增 100 ms 提早 hint，沿用 DRV-4 的 Sent／Confirmed 斷言；忽略 idle 紀錄的反向以 queued 失敗。bridge 全檔目前 27 個入口，其中 `boot_child` 不算獨立回歸。既有 pipeline fixture 只在同一 native FleetView 同時包含 approve 階段與 approval attention 時核准，沿用 60 秒等待期限，不把階段可見當成 attention 已發布。

## 真模型 smoke（預設不執行）

使用者要求完整真模型通訊 smoke 為必要驗收；固定版本、七則訊息計畫、執行 opt-in 與清理見[真模型 smoke](../../docs/gates/gate-12a-live-smoke.md)。`claude_live_cleanup` example 只停止 nonce-owned holders 與精確身分的孤兒群組；不啟動 backend。CI 仍不執行真模型。

`python3 -B scripts/verify_smoke_contract.py --agend <固定 binary>` 分別以 Bash／zsh 執行生成的 INITIAL 與真 gh shim，零 Claude／daemon／訊息；核 `/usr/bin/which` 的五個外部 PATH、唯讀 `pr merge --help` 仍被 gh_merge 拒絕、peer prompt quoting、七段語法與每個 shell 的二十三個 audit／PATH／缺少觀察與 prefix 反例，另核空 prefix 正例與缺 baseline 負例；四個 startup token 拒絕由真 shim 產生，prefix bytes 必須在 INITIAL 前保存並逐 byte 核對，base64 保留換行差異及無效 UTF-8 的原始失敗資料，工作 suffix 仍只允許一個 gh_merge。失敗的路徑／拒絕／exit／原生 audit 先保存再判定；驗證只刪自有 fixture，真 smoke 保留使用者要求的 account trust entries，見[指令修正](../../docs/gates/gate-12a-smoke-contract.md)、[v7 原始失敗](../../docs/gates/gate-12a-observed-smoke-v7.md)與[v8 原始失敗](../../docs/gates/gate-12a-observed-smoke-v8.md)。

2026-10-06 啟動診斷取得四份相同真 frame，結果 CAPTURED、零工作訊息。
新增完整 Ready 提示 fixture 的 native P6 回歸只跑 shell producer／真 daemon／holder，
核 SessionStart 先到、三鍵完成後穩定五秒、Ready 不加鍵及 halted=1；不啟動真 Claude。
[第二次完整 smoke](../../docs/gates/gate-12a-observed-smoke-v2.md)亦在 initial idle 逾時：A idle、B unknown，24 份 frame、零工作訊息。
新增已錄製的 how-does 完整 Ready literal 與同一 native P6 回歸；未知提示仍拒絕。
新真 CLI 計畫仍需另行授權；native 通過不等於模型通訊通過。

2026-10-06 [D41](../../docs/decisions/d41.md) 允許 Ready 建議內容可變，其他完整畫面／版本／路徑／尺寸不變。
`claude_startup::startup_variable_ready_suggestions_replay_actual_v5_and_both_widths` 經真 daemon／holder／PTY 重播 v5 兩份捕獲及 140 欄變體，核五秒初始 idle 與 Ready 不加鍵；
`startup_variable_ready_rejects_unknown_footer_and_split_hint_without_idle_or_more_keys` 拒絕未知 footer／分行建議。
既有無 SessionStart、人工控制、結果不明與四次開機回歸維持；這些測試不啟動真 Claude、不送模型訊息。

## 第 12B 原生 OpenCode bridge

先 `cargo build -p agend -p agend-testkit --bins`，再 `cargo test -p agend --test opencode_bridge`。兩個案例使用真 daemon／holder／shell wrapper 和原生 fake OpenCode CLI：保存一筆權限工作，daemon 重啟保留 holder／ask，拒絕後重複回覆失敗，再分別正常停止與 SIGKILL 自有 holder；新 holder 恢復原 session、歷史只有一筆 user message，最後確認 server port 關閉。Fixture 僅清自己的 lab；panic 路徑停止自有 holders，依 marker sweep 舊 group。零模型呼叫；不替代真 1.18.34 驗收。

12D `cargo test -p agend --bin agend doctor::telegram_tests` 驗 Telegram 未設定、空 allowlist 回報 fail、允許 user、公開 token 檔拒絕及 malformed 設定不洩漏 secret；doctor 只讀本機，不呼叫 Telegram。既有 `--test cli init_and_doctor` 仍驗正式 binary 輸出。

12D 手機探針 `telegram_mobile_probe` 與 `examples/support/telegram_mobile.py`：`--local` 使用自有 daemon／SQLite／Client／TUI App 狀態模型，驗已讀不關閉、確認後關閉、重啟與清理，不呼叫 Telegram。真測使用 `--credentials` 專用私人 env 檔，在 300 秒內由人先點 Mark read 再 acknowledge；要求持久 update outcome 為 read、accepted，兩個 daemon boot 正常退出並刪除自有通知／home。此探針不渲染 TUI 終端畫面，亦不代替所有手機動作或真 forum 驗收。見 [12D 手機紀錄](../../docs/gates/gate-12d-telegram.md#手機-callback-驗收2026-10-07)。

第 13A 的 `tests/install_home.rs` 以隔離 HOME 執行正式 CLI：預設 socket 對真協定 producer、明確覆寫、agent 不可退回預設、init 權限／重跑保存編輯、v1 與 symlink 拒絕。所有路徑由 TempDir 擁有並清理；既有 missing-home 案例同時移除 HOME，避免碰測試執行者的真實 home。

第 13B `cargo test -p agend --test install_service` 驗真 CLI 的唯讀 service plan、特殊字元與環境拒絕；macOS 使用 plutil、Linux 使用 systemd-analyze verify 解析實際產物。測試僅寫隔離 fixture，不註冊服務，不能代替 holder 存活的真服務驗收。

13B 施工中的 `cargo test -p agend --bin agend service::tests` 檢查安裝 receipt 對帳、遺失註冊回覆、中斷解除安裝、外來與修改過的檔案保存。holder 路徑反例涵蓋 `run`／`run/holders` symlink 與 live lock 配上外部 socket symlink；後者用真 UnixListener 確認沒有連線，並驗證尚未呼叫 service stop。此組使用 manager 狀態模型，不能證明 launchd／systemd 真服務生命週期。`install_service` 另以真 CLI 驗 `--no-start` 的發布權限、重跑與修改後拒絕。

`service::manager::systemd` 消費隔離 systemd 255 的實際 GetAll JSON：baseline 通過，drop-in 和各項有效設定變更拒絕。Linux-only `live_process_identity_uses_the_actual_executable_arguments_and_home` 以自有原生程序驗 `/proc` identity，結束會 kill/wait 精確 Child；macOS 不執行這一項；2026-10-08 已在隔離 Ubuntu 24.04／systemd 255 執行通過。該批另以正式 CLI 驗證全新安裝、daemon 重啟保留 holder、兩個 systemd 反例與預設保留資料的解除安裝；原始腳本及分段結果保存在 `AgEnD-ops/g13-install-20261008/linux-service-*.py`／`.log`。

13B 資料刪除：`service::tests` 使用真 SQLite DB 與真 Git repository，驗明確刪除、鎖 inode 保留、外部 symlink 目標不變、live store／Git workspace 拒絕；`install_service` 用正式 CLI 驗缺少／錯誤路徑確認及缺 receipt 時零刪除。Linux `same_device_bind_mount_cannot_delete_external_data` 預設 ignored，必須在自有可掛載的隔離環境另以 `--ignored` 執行；一般 tests 全綠不代表該項已驗。

macOS `service::manager::launchd::tests::native_process_identity_checks_executable_argv_home_and_exit` 以自有 C 程式消費真 proc API，另用當機 SDK 核 Region／RegionPath 的 ABI size／offset。驗 executable 映射 inode、精確 argv／home、UID／世代與退出；原程序仍存活時替換同一路徑 executable 必須拒絕。Child guard 精確 kill/wait、TempDir 清理，零 launchd 註冊／模型。此測試不代替 loaded definition 或真 Rust daemon 的服務驗收。

13C `cargo test -p agend --test backend_import` 使用真 CLI／原生 executable 檔案驗隔離副本、重複與竄改拒絕、外部 symlink 保留、版本 identity、wrapper／agent 拒絕及維護期間零發布。不執行 backend，不代替完整 canary 驗收。

13C `daemon_refuses_unverified_or_changed_managed_program_before_starting_a_holder` 以真 CLI 匯入 manifest，再啟動七個隔離 daemon case，核未驗版本、修改後 executable、外部 symlink alias、裸 PATH 命令及相對 cwd 路徑 都在 holder 建立前拒絕，停止後 DB 保留 Failed。自有 Lab 負責程序與目錄清理，沒有執行真 backend。

受管路徑測試另驗 PATH 前項為 0610（owner 無執行權）時不能遮蔽後面的受管 binary，以及 ambiguous `foo/bar` 拒絕。此項需 non-root 使用者執行，以真正測到 access(X_OK) 權限語義。

`cargo test -p agend --test message_delivery` 用真 Store producer 與兩次 daemon 開機驗四種 delivery state、精確 ID／對象／attempt 時間、missing 回 None、agent forbidden、無 body 及查詢不改狀態。無 backend／模型程序，Lab 清除 daemon 與 home。

13C canary 施工：`cargo test -p agend --bin agend backend::canary::process::tests` 使用原生 C probe，驗證 fork／posix_spawn 拒絕而 pthread 仍可用、版本逾時回收、探測程序自行 setsid 的回收與外部 run symlink 拒絕。`backend_canary` 以既有原生 fake Codex 驗三則 confirmed、idle、版本不符拒絕、完成後沒有 canary home 與 fleet activation。`message_delivery` 另驗操作員 sender、重送相同 ID 不改內容、相同 ID 不同內容拒絕、agent 無權使用操作員 RPC 與未知 instance 不存訊息。版本探測子程序限制已通過 macOS 與 Linux ARM64 原生驗證；Linux x86_64 與真 backend 尚待驗，測試不使用真模型。

- Canary 保留報告：使用原生 producer 的報告，變更 artifact／binary／平台／版本、清理結果、收據身份與狀態，以及時間順序／越界／overflow；正式 verifier 必須拒絕，inspect 顯示 invalid 且不啟用版本。

Canary 必須取得三筆與 delivery 綁定的 completed outcomes；修改 failed／foreign turn／缺 outcome 必須拒絕。

執行 backend_canary 前建置 `cargo build -p agend --example fake_codex` 與 `cargo build -p agend-testkit --bin fake-opencode-cli --bin fake-claude-cli`。共用測試對 Codex／OpenCode／Claude 各核三筆 delivery／outcome、錯版拒絕、報告異常與清理，不執行真模型。Claude fixture 以保存的 Ready 畫面走真 holder parser，經正式 channel helper 的 MCP ACK 回覆後才發 PostToolUse 與 Stop；ACK 到 Stop 間留延遲，但單憑延遲不宣稱一定讀到中間 Unknown 狀態。

`cargo test -p agend --test pinned_launcher` 在自有短路徑 Lab 啟動正式 daemon，原地以 false 替換原始 AgEnD 路徑，再以正式 InstanceAdd 啟動 bash fixture；核對 holder 與 shim 使用固定副本、daemon 停止後 holder／副本保留，以及停止自有 holder 後整個 Lab 可清除。無模型或主機服務註冊。

`backend_canary` 的三 backend 原生整合案例使用正式預設 180 秒總期限；期限仍涵蓋版本探測、三回合、最終驗證與清理。Ubuntu debug binary 的複製／雜湊在並行測試下曾使 60 秒測試預算於最終驗證逾時（三回合已完成），因此測試與正式預設對齊；這不放寬產品期限，也不代表真模型已驗證。

`holder_runtime` 新增 SQLite 預存 UUID → native holder 啟動 → 新 runtime 重連同一 agent PID、錯 UUID／legacy holder 拒絕且保留程序、未驗證 holder 的歷史 Exited 不送入 supervisor 三項。既有 RTM-1..9 與四 boot 跨程序案例持續驗證。此批是 holder 連線契約，尚未開放 CLI 受管版本准入。

`backend_canary` 的三 backend 原生替身測試另驗 canary 後的受管 fleet 啟動：保存 intent、daemon 重啟保持 holder PID／UUID、持久 UUID 改成另一合法值後拒絕接回且保留程序。fixture 停止自有 holder 並移除 lab；不執行真模型。

同一 native canary fixture 將正式報告的 AgEnD digest 改成不匹配值，確認新啟動准入拒絕，但已保存 UUID 的原 holder 仍可重連；這驗准入與重連分流，不宣稱替代所有跨版本 driver 相容性驗證。

`cargo test -p agend --test backend_switch` 用正式 Store 建立中斷的 Prepared，再經真 daemon／CLI 驗 status、空紀錄、精確 cancel、舊 ID／agent 拒絕與重啟保留；另以 exchange_once 解碼真 producer 的 Some／None。Failed fixture 不啟動 backend，故不證明成功 prepare 的 canary 准入或實際換版。Lab 負責程序與目錄清理。

`cargo test -p agend --test holder_runtime managed_stop_` 驗換版專用停止的精確身分與 socket 路徑替換反例；全部只啟動自有 sleep fixture，Lab 清理 holders 與 home，沒有模型或主機服務。

13C 回覆排空由 `cargo test -p agend --test client_protocol inbox_delivery_fence_` 跑正式 server 的 socket 背壓；不以 handler 已返回代替整段寫入完成。不證明遠端模型回合結束。

13C 終端切換保護：`cargo test -p agend --test terminal_hub`。新增 Prepared 下操作拒絕與真正 PTY 背壓排空案例；fixture panic 會保留 daemon 日誌於測試輸出，區分 holder 啟動失敗與輸入政策拒絕。

13C 晚 holder 清理回歸：`cargo test -p agend --test holder_process a_late_holder_cannot_recreate_its_removed_home`。真 shell child 在 exec 前與測試同步，home 清理後真 holder 必須拒絕，不能重建目錄。

`opencode_bridge` 的兩條 holder 重啟路徑也驗 `OpenCodeDriver::session_idle`：native REST 閒置／忙碌／abort 後閒置、session 與 handoff 不符、holder 停止後拒絕；只用 fake backend。

`claude_startup::startup_prepared_switch_holds_native_keys_until_operator_cancels` 在 SQLite 預備切換後啟動真 holder/parser 與 recorded-menu PTY producer：Prepared 不送鍵，正式 operator cancel RPC 後送三鍵。此 shell fixture 只驗暫停契約，不宣稱 managed canary 或真模型驗證。

`claude_idle_observation_requires_live_hook_session_and_original_holder_connection` 將實際 server 放在本程序，啟動原生 holder 與 hook helper，核 Stop 穩定期、busy 撤銷、session 變更及重連失效；新連線須先有 routing hook 才可重建 idle。只用 shell producer，不執行 Claude／模型。

13C：pending backend switch 阻止一般 boot start、自動重啟與 operator retry 改寫啟動意圖。`backend_switch::pending_switch_boot_preserves_launch_reservation_without_ordinary_restart` 經原生 daemon 驗三種 pending phase 跨 boot 保留精確資料；專用換版恢復仍待串接。

13C 新增 `backend switch activate／rollback --switch-id`：精確持久 ID、目的版本准入、投遞排空與 native idle 後停止受管 holder；Committed／Restoring 保持暫停，核新 holder 綁定及 readiness 後才釋放。Activated／Committed 回滾先保存 RollbackPrepared；daemon 重啟後由定期協調器繼續。三 backend 原生假版本往返與目的 holder 消失後自動回退已驗；完整 crash matrix 尚未完成。

`holder_runtime::managed_identity_queries_preserve_the_native_terminal_connection` 使用真 holder 檢查正確與錯 binding／agent PID，並核原 terminal connection 未失效且仍可讀 snapshot。`backend_canary` 的 Claude／OpenCode 第二版本為同協定 fixture，不代表真新版相容；Claude fixture 閒置持續等待，測試期限與清理由 harness 管理。

`backend_canary::canary_scope_rejects_other_homes_instances_and_changed_artifacts` 經正式 import CLI 與 scope producer 驗正常版本觀察，及跨 home、錯 instance、額外 argv、修改 executable 的拒絕；另核 scope 本身沒有成功 canary 報告，不可取得 fleet 准入。此案例不執行 backend。

版本往返測試在 target holder 已啟動、尚未建立 backend ready 時用私有 workspace marker 暫停 fixture，確認正式 activate 回覆 Committed，硬殺自有 daemon 並重啟，核同一 holder PID 存活才釋放 marker；後續須完成 Activated／RolledBack 並保留 session。這個切點不涵蓋 Ready 已觀察但尚未持久完成、目的 holder 死亡或還原中再次中斷。

`native_codex_committed_switch_recovers_when_the_target_holder_is_absent` 在硬殺 daemon 後由 Lab 停止目的 holder，再重開 daemon，核新 holder 啟動並完成原 session 的啟用／回滾。probe 清理先等待 SIGKILL 子程序成為可回收狀態，再檢查群組；setsid fixture 使用正式 5 秒 probe 預算且必須留下 ready marker，不能把尚未執行到 setsid 算作通過。

經身分驗證的目前代 `AgentExited` 也可觸發目的版本回退：先保存 RollbackPrepared、排空既有回覆及 worker，再以持久 UUID／holder PID／agent PID 停止精確 holder，恢復舊版本。driver Gone 或 StartFailed 不等於原生退出，不能走這條捷徑。Codex fixture 在 app-server 交接完成後讓 TUI 自行退出，已驗恢復舊版及原 session；app-server 單獨退出而包裝仍存活的情況尚待處理。

13C 換版問題的原生 Store 測試驗重開保留原因／等待時間、重複觀察不刷新、舊快照拒絕、取消及成功啟用／回退清除問題。`native_codex_disconnected_candidate_reports_a_durable_problem_without_stopping_holder` 讓假 app-server 自行退出但保留包裝／holder，經正式 status 核持久問題、「需要你」通知及 daemon 重啟後同一等待時間／holder，沒有真 backend 或模型呼叫。

13C activation deadline 的 Store 測試使用明確時鐘值，核到期前一毫秒／邊界、時間倒退、重開、問題更新不延長期限，以及 Restoring 才重新計時。原生 pending boot 測試用正式 Store 建立已過期 Committed／Restoring，兩次 daemon 啟動核相同 deadline／problem、原設定與啟動意圖不變；Prepared 不誤報逾時。這是故障狀態恢復證據，不代表真模型啟動耗時測量。
