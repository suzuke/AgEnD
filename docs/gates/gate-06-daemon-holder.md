# 第 6 施工關：daemon ↔ holder（整合施工關）（`daemon-holder`）

> **TL;DR**
> - 真 daemon 接真 holder（agent runtime adapter）：daemon 重啟時 agent 不斷線；holder 或 agent 死了，daemon 用 `--resume` 接回。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：實作在 draft PR（branch `feat/gate-06-daemon`）；先看「待你追認」，再照「你親自驗收」一步一步做。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**實作中，draft PR**（2026-09-26）：P1–P9 已實作，自動驗收由實作者自跑通過；fresh-context verifier r2 CONFIRMED；H1–H16 使用者已追認；使用者親自驗收 9 步通過，等 merge。名詞表不在本 PR 改（見「自動驗收」）。

## 範圍

- `agend daemon`：只在前景跑、必須設 `AGEND_HOME`、同時只有一個 daemon（P1）
- `instances` 表與開機計畫 `plan_boot`；demo 用 example `daemon_probe` 加減 instance（P2）
- daemon 起 holder、agent 環境白名單、shim symlink（P3）
- daemon 端的 holder 協定 client（agent runtime adapter `HolderRuntime`）與三層契約測試（P4）
- daemon 重啟後重連 holder、取回畫面（P4、P5）
- 開機巡查孤兒 holder：掃 `$AGEND_HOME/run/holders/` 鎖檔，DB 裡沒有的 instance → 送 `Shutdown`（第 4 施工關 P2，使用者 2026-09-25 決定；做法見 P2、P5）
- holder 被 `kill -9` 之後的補救：偵測 holder 死了 → 用 backend 的 `--resume <id>` 把 agent 接回（第 4 施工關 P2／第 3 施工關 T18；做法見 P6）
- daemon 對自己啟動的 holder 收屍（`wait`），避免殭屍（P3）
- 綁定／釋放 worktree 時呼叫 `agend_shim::install_hooks`／`uninstall_hooks`（agend 的 git hook 只裝在該 agent worktree，見[第 3 施工關](gate-03-shim.md#範圍)）：**移到第 10 施工關（P9）**
- 本頁步驟 5（Ctrl-C 停 daemon，agent 還在）依賴第 4 施工關 P2 的 session 分離；`pgrep` 一律只比對 `g6-` 開頭的 instance id
- daemon 開機時與之後每天跑一次 store 的 `prune` 與每日 DB 快照（第 5 施工關 P8/P9；做法見 P5）
- 開 DB 時重試到 10 秒，因為重啟時 EXCLUSIVE lock 交接需要時間（第 5 施工關風險；做法見 P1）
- daemon log 與 audit 輪替（P8）
- D2 的重啟預檢**不在本關**，移到第 9 施工關（P7）

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子。文件已定、這裡不重問的：daemon 重啟時 agent 不斷線、holder 被硬殺時 agent 一起死（D3）；daemon 常駐交給 launchd／systemd（D2）；holder 協定版本協商、新 daemon 要能跟舊一個 major 的 holder 溝通（D26）；holder 的行為（第 4 施工關 P1–P9：`setsid`、忽略訊號、`run/holders/` 三個檔、新連線踢舊連線、`Shutdown`）；store 的行為（第 5 施工關 P1–P9：EXCLUSIVE 鎖、migration、`prune`、DB 快照）；重啟類契約用四次開機、跨真的 process 驗（[CONTRACTS.md](../../crates/agend-testkit/CONTRACTS.md)、第 2 施工關 A25）。

### P1：daemon 是什麼樣的程序、怎麼保證只有一個

- 問題：daemon 要不要自己 fork 到背景？`AGEND_HOME` 沒設怎麼辦？怎麼保證同一個 home 只有一個 daemon？收到 Ctrl-C 做什麼？
- 建議：
  - `agend daemon` 只在前景跑，不自己 fork；常駐交給 launchd／systemd（第 13 施工關）。沒有 `--foreground` 旗標。
  - 必須設 `AGEND_HOME`，沒設就印 `AGEND_HOME is not set` 並 exit 1（預設值交給第 9、13 施工關）。
  - 單一 daemon＝DB 的 EXCLUSIVE 鎖（第 5 施工關 P2）。打不開就每 200 ms 重試，**10 秒**後放棄，印 `agend.db is in use by another process …` 並 exit 1。重試是給重啟交接用的：舊 daemon 正在結束、還沒放鎖。
  - run 目錄只有 `run/holders/`（第 8 施工關再加 `run/daemon.sock`）。不做 v1 的 `.daemon`、`.ready`、`api.cookie`、`api.port`。
  - 收到 SIGINT／SIGTERM：5 秒內依序結束（停排程、關 holder 連線、關 DB）。holder 繼續跑，**絕不送 `Shutdown`**。
- 理由：自己 fork 正是 v1 反覆失敗的地方（D2 的 #881、#882、#903）；一把 DB 鎖同時保護 DB 與 holder，不必另做 pid 檔。
- 替代方案：另做 `daemon.lock` 鎖檔（兩把鎖可能不一致）；自己 fork 成背景（v1 的路）；結束時停掉所有 holder（違反 D3）。
- 例子：daemon 跑著時按 Ctrl-C，2 秒內回到 shell；`pgrep -fl "agend holder g6-"` 仍列出原本的 holder。
- [x] 使用者確認（2026-09-25）

### P2：instance 存哪、開機怎麼決定要做什麼

- 問題：daemon 怎麼知道該有哪些 instance？開機時接回、新起、巡查孤兒，邏輯放哪？第 9 施工關的 CLI 還沒有，demo 怎麼加 instance？
- 建議：
  - migration `0002_instances` 加 `instances` 表；保留期限規則是「永久」（第 5 施工關 P8 的規則表）。instance id 只能是 `[a-z0-9-]{1,24}`，讓 socket 路徑較短；真正保證不超過 100 bytes 的是第 4 施工關 P3 的啟動時檢查（`$AGEND_HOME` 太長時 holder 拒絕啟動）。
  - 純函式 `plan_boot(DB 的 instances, run/holders/ 的鎖檔) -> 動作清單`，一次決定三件事：鎖被持有而且 DB 有 → **接回**；DB 有、鎖沒被持有 → **啟動**（有 session id 就帶 `--resume`，見 P6）；鎖被持有、DB 沒有 → **孤兒**，送 `Shutdown`。純函式用表格測試逐列驗。
  - demo 用 `agend-daemon` 的 example `daemon_probe add|remove|list`，直接開 DB，所以只能在 daemon 停著時用（daemon 跑著時它拿不到鎖，照 P1 報錯）。不做 production 命令，也不做 `agend debug spawn-fake`。
- 理由：開機決策集中在一個純函式，孤兒巡查、接回、補救不會分散在三處各自判斷；example 不必為了測試多一個正式命令。
- 替代方案：instance 寫在 `config.toml`（D8 說持久狀態只在 DB）；現在就做 `agend instance add`（第 9 施工關的事）；隱藏子命令 `agend debug …`。
- 例子：DB 有 `g6-1`、`g6-2`，`run/holders/` 有 `g6-1`、`g6-9` 的鎖 → `reconnect g6-1`、`start g6-2`、`shutdown g6-9`。
- [x] 使用者確認（2026-09-25）

### P3：怎麼起 holder、agent 拿到什麼環境、怎麼收屍

- 問題：daemon 用什麼指令起 holder？怎麼確定它起來了？agent 的環境變數從哪來？holder 死了誰收？
- 建議：
  - 起 holder：`current_exe() holder <id>`，`env_clear()`；**不設** `process_group(0)`（holder 自己 `setsid()`，已經是 process-group leader 時 `setsid` 會失敗 EPERM；見第 4 施工關 P2 與實作偏離 G7。夜間驗證 2026-09-26 抓到並更正，使用者 2026-09-26 追認）。之後每 50 ms 試連 socket，5 秒連不上算失敗（交給 P6）。
  - 先把 instance 狀態寫進 DB，再送 `Spawn`；daemon 在兩者之間當掉，重啟後重送 `Spawn`，holder 回 `already_spawned`（第 4 施工關要補，見風險）。
  - agent 環境用白名單組出來：`AGEND_*`、`PATH`（以 `$AGEND_HOME/bin` 開頭，shim 才會先被找到）、`HOME`、`USER`、`LANG`、`TMPDIR`、`TERM`…（完整清單開工時細化）。不在清單上的一律不給。
  - 開機時確認 `$AGEND_HOME/bin/` 的 shim symlink 存在、指向目前的 binary；缺了或指錯就重建。
  - 收屍：自己起的 holder，每個一條 thread 呼叫 `wait()`。接回的 holder 不是自己的子程序，死活看「連線斷了＋鎖放掉了」。
- 理由：`env_clear` 加白名單，daemon 環境裡的 secret（例如 Telegram token）不會漏給 agent（第 4 施工關 P8 只清 holder 這一層）；先寫 DB 再 Spawn，當掉時不會出現「DB 不知道的 agent」。
- 替代方案：繼承 daemon 環境再刪黑名單（新 secret 會漏）；只用 socket 判斷死活（第 4 施工關 P3 已否決）；只靠 `SIGCHLD` 收屍（接回的 holder 不是自己的子程序，收不到）。
- 例子：daemon 環境有 `TELEGRAM_BOT_TOKEN`；agent 裡 `env | grep TELEGRAM` 什麼都不印，`echo $PATH` 開頭是 `$AGEND_HOME/bin`。
- [x] 使用者確認（2026-09-25）

### P4：`HolderRuntime` 怎麼做、契約怎麼驗

- 問題：`Runtime` trait 的真實作怎麼接 holder？recover 怎麼判斷 holder 還在？RTM 規則怎麼對真東西跑？
- 建議：
  - 實作叫 `HolderRuntime`。`recover_holders` **只看鎖檔的 `flock` 是否被持有、不連 socket**：holder 的「新連線踢舊連線」（第 4 施工關 P4）會讓試連的動作踢掉 daemon 自己的長連線。
  - holder 協定是阻塞 I/O，放 `spawn_blocking`。每個 holder 一條長連線；連線被搶走（別的 client 連上）就 1 秒後重連。
  - 契約三層：
    1. RTM-1..9 對真 holder 程序跑（同 process 的 fixture、真的 `agend holder`）。
    2. 重新執行自己（`current_exe()`）的四次開機，比照第 5 施工關 P7。
    3. 真的 `agend daemon` 四次開機（CONTRACTS 的「做事／閒置／做事／檢查」），其中一次 daemon 被自己的測試硬殺。
  - 反向檢查：同流程「每次開機用新的 `AGEND_HOME`」必須失敗，證明測試真的在驗跨重啟。
- 理由：CONTRACTS 規定重啟類規則（RTM-8、RTM-9）要跨真的 process；只驗同 process 會讓「holder 跟著最後一個 runtime 死」這類實作混過去（`LastOneOut`）。
- 替代方案：recover 時連 socket 確認（會踢掉自己）；每次操作才連線（拿不到持續的畫面與 `Exited`）；只做第 1 層（驗不到真 daemon）。
- 例子：`boot 1 daemon pid=5101 holder pid=5102`、`boot 2 daemon pid=5110 (idle) holder pid=5102`、`boot 3 daemon pid=5117 killed -9 by test`、`boot 4 daemon pid=5125 holder pid=5102 counter=41 ok`。
- [x] 使用者確認（2026-09-25）

### P5：開機順序、就緒訊號、每天的工作

- 問題：daemon 開機依什麼順序？外面怎麼知道它好了？每天的 `prune` 與 DB 快照在筆電睡眠時會不會漏掉？
- 建議：
  - 開機順序：開 DB（P1，重試 10 秒）→ `prune`、DB 快照、log 輪替（失敗只記錯，不擋開機）→ shim symlink（P3）→ 照 `plan_boot` 接回／啟動／對孤兒送 `Shutdown`（P2）→ 印一行 `agend daemon ready: …`。
  - 就緒訊號就是這行 log；第 8 施工關起改成「`run/daemon.sock` 連得上」。不做 `.ready` 檔。
  - 每天的工作改成「每小時醒來一次，補做今天還沒做的」（今天的 DB 快照不存在才做，第 5 施工關 P9 已是這個判斷）。
- 理由：先確定 DB 是自己的，才動 holder；每天的工作失敗不該讓 agent 全部接不回來。筆電睡眠時「每 24 小時」的計時器會延後或漏掉，每小時補做不會。
- 替代方案：`.ready` 檔（v1 的做法，要處理當機殘留）；固定 24 小時計時器（睡眠會漏）；每天的工作失敗就停止開機。
- 例子：`agend daemon ready: instances=2 recovered=1 started=1 orphans=1`。
- [x] 使用者確認（2026-09-25）

### P6：holder 或 agent 死掉之後

- 問題：agent 自己結束、holder 被 `kill -9`、holder 起不來，daemon 要做什麼？會不會無限重起？
- 建議：
  - 不管哪一種死法，都等 5 秒、帶 `--resume <session id>` 重起（沿用原本的對話）。
  - 10 分鐘內重起 3 次仍死 → 標 `failed`、停止重起，交給人。
  - **絕不自動全新啟動**：沒有 session id 可接，就直接 `failed`。
  - 重起次數只放記憶體，daemon 重啟後重新計算。
- 理由：全新啟動會丟掉 agent 讀過的脈絡與進行中的工作，比停下來等人更糟；次數上限擋住「一起來就死」的迴圈燒 token。
- 替代方案：立刻重起（死循環時太快）；指數退避無上限（永遠不交給人）；次數存 DB（多一個欄位；daemon 重啟本來就少見，歸零無妨）。
- 例子：假 agent 一起來就 `exit 1`：`restart 1/3 --resume s-abc`、`restart 2/3`、`restart 3/3`、`g6-1 failed: died 3 times in 10m`，之後不再起。
- [x] 使用者確認（2026-09-25）

### P7：D2 的重啟預檢

- 問題：D2 要求「重啟前預檢新 binary，失敗就不切換」。這關要做嗎？
- 建議：移到第 9 施工關（`agend daemon restart` 在那關才有）。設計先定：新 binary 用最新 DB 快照的**複本**跑 migration 加 `quick_check`；在暫存 home 起一個自己的 holder 跑一次（hello、Spawn、Shutdown）；都過了才切換，任何一步失敗就留在舊版。core 的「`SUPPORTED_VERSIONS` 包含前一個 major」測試等到第一次升 major 時才有意義（目前只有 V1），開工時細化。
- 理由：本關沒有觸發 restart 的命令，現在做沒有呼叫點、驗不到；用快照複本預檢，DB 本身一個 byte 都不動。
- 替代方案：本關就做（沒有使用者）；直接對 `agend.db` 跑 migration 預檢（失敗時 DB 已被改）。
- 例子：新版 migration 有錯：預檢印 `preflight failed: migration 0003 …`，舊 daemon 照常跑。
- [x] 使用者追認（2026-09-26）

### P8：daemon log 與輪替

- 問題：daemon 的 log 寫哪、留多久？audit 與 holder log 誰清？
- 建議：
  - daemon 自己寫 `$AGEND_HOME/logs/daemon-YYYY-MM-DD.log`（tracing），留 7 天。
  - audit（第 3 施工關 T10 的 `audit/shim.jsonl`）每天輪替，留 14 天；規則列進第 5 施工關 P8 的同一張規則表。
  - holder log（`run/holders/<id>.log`）：holder 不活、且 7 天沒動過才刪。
  - launchd／systemd 不另外導 log（stdout／stderr 只剩 panic）。
  - 刪除只在 P5 的開機與每小時工作裡做。
- 理由：v1 的 home 長到 161G，log 一定要有期限；自己寫檔，macOS 與 Linux 看 log 的方法一樣。
- 替代方案：只寫 stdout、交給 launchd／journald（兩個平台不同，launchd 不輪替）；`tracing_appender::rolling`（v1 用過，刪檔規則不在同一張表）。
- 例子：`logs/` 只剩最近 7 個 `daemon-*.log`；`audit/` 有 14 份；還活著的 holder 的 log 就算 30 天沒動也不刪。
- [x] 使用者追認（2026-09-26）

### P9：hooks 與 binding 快照、依賴規則

- 問題：範圍寫了「綁定／釋放 worktree 時呼叫 `install_hooks`」，binding 快照也還沒人寫。這關做嗎？
- 建議：兩者都移到第 10 施工關：這關沒有 worktree，沒有呼叫點。`check-deps` 加規則：`agend-daemon` 不能依賴 `agend-holder` 或 `agend-shim`（daemon 只經 holder 協定和子命令跟它們互動）。
- 理由：沒有呼叫點的功能只能用假資料測，第 10 施工關有真的綁定流程時才驗得到；依賴規則讓「daemon 直接呼叫 holder 內部」編譯不過。
- 替代方案：本關先做、用假 worktree 測（驗不到真正的坑）。
- 例子：有人在 `agend-daemon` 的 `Cargo.toml` 加 `agend-holder`，`cargo xtask check-deps` 失敗。
- [x] 使用者追認（2026-09-26）

### 已知風險（開工時處理）

- 第 4 施工關要補 `already_spawned`：holder 收到第二個 `Spawn` 回錯誤、不重起 agent（P3；已通知第 4 施工關 executor）。
- 新連線踢舊連線：任何「順便連一下看看」的程式都會踢掉 daemon 的長連線，所以 recover 絕不連 socket（P4）。
- `--resume` 要有 session id：claude 本關可用；codex、opencode 的 session id 要靠第 7、12 施工關，在那之前這兩個 backend 死掉就直接 `failed`。
- launchd 給的 `PATH` 很短、brew 升級後 shim symlink 可能暫時指到不存在的 binary：交給第 13 施工關（P3 開機時重建只救得了 daemon 重啟的那一次）。
- EXCLUSIVE 鎖交接的 10 秒（P1）還沒實測，本關的四次開機要量實際等了多久。
- 第 5 施工關 verifier 實測：多個程序同時建立**全新** home 時，約 10% 的機率全部拿到 `InUse`、沒有人建成（下一次 `open` 會成功）。P1 的 200 ms 重試已涵蓋；本關測試不要假設「同時開、一定有一個成功」。

## 自動驗收（完成定義）

- [x] `~/.cargo/bin/cargo test -p agend-daemon`、`~/.cargo/bin/cargo test -p agend-holder` 單獨通過，包括：`plan_boot` 表格測試（`boot::tests`，P2）；agent 環境白名單、secret 不外漏（`runtime::env::tests`，P3）；重起 3 次後 `failed`、不全新啟動（`supervisor::tests`，P6）；log 與 audit 輪替用假時鐘（`housekeeping::tests`，P8）；`instances` 表有保留規則（`tests/store.rs` 的 `every_table_in_the_database_has_a_retention_rule`，P2）
- [x] 契約三層都通過（P4），測試在 `crates/agend/tests/`（要真的 `agend` binary）：RTM-1..9 對真 holder（`holder_runtime.rs` 的 `contract_rtm_1_to_9_passes_against_real_holders`，9/9）；重新執行自己的四次開機（`four_boots_in_four_processes_recover_the_same_holders`）；真 `agend daemon` 四次開機、開機 3 被測試 `kill -9`（`daemon_process.rs` 的 `four_daemon_boots_one_killed_keep_the_holder_and_its_counter`）。反向檢查「每次開機用新的 `AGEND_HOME`」兩層都在開機 2 失敗（`four_boots_with_a_new_home_each_boot_fail`、`four_daemon_boots_with_a_new_home_each_boot_fail`）。重啟類 case（RTM-8、RTM-9，見 [CONTRACTS.md](../../crates/agend-testkit/CONTRACTS.md)）跨真的 process 重啟跑（分開的 process、真的檔案／DB）**（已追認 2026-09-25，第 2 施工關 A25）**
- [x] 第二個 daemon 在 10 秒重試後被拒絕（實測 10.2 秒）、第一個不受影響（log 檔內容不變、程序還在）；Ctrl-C 後 holder 還在、holder log 沒有 `shutdown requested`（P1；`a_second_daemon_is_refused_after_ten_seconds_and_the_first_is_untouched` 與四次開機的開機 1）
- [x] core 加測試：`SUPPORTED_VERSIONS` 包含前一個 major（P7）。細化：目前只有 V1，測試寫成「最新 major 是 1，或清單裡有最新 major − 1」，第一次升 major 時才會真的擋（`protocol::holder::tests::supported_versions_keep_the_previous_major`）
- [x] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [x] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `check-deps: ok (5 rules, 8 crates checked for agend-testkit, agend-core metadata ok, no-std build ok)`，並有新規則：`agend-daemon` 不能依賴 `agend-holder`、`agend-shim`（P9）。細化：在 `crates/agend-daemon/Cargo.toml` 的 `[dependencies]` 加 `agend-holder.workspace = true` 後印出 `check-deps: agend-daemon depends on agend-holder (…gate 6 P9)`、`xtask: 1 dependency rule violation(s)` 並失敗；還原後恢復 ok（2026-09-26 實測；單元測試 `the_daemon_may_not_link_the_holder_or_the_shim` 也釘這條）
- [x] `~/.cargo/bin/cargo xtask accept daemon-holder` 通過，並印出下方「你親自驗收」用到的 demo（最後一行 `gate 6 (daemon-holder): checks passed`）
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新（agend-daemon、agend、agend-holder 已做）；名詞表加上 `daemon_probe`、孤兒 holder、instance 狀態（含 `failed`）、`logs/`（名詞表是共用文件，不在本 PR 改，文字交給 orchestrator）
- [x] 測試不留殘留：結束時沒有 `g6-` 或測試 id 的 holder（每個測試結尾檢查 lock 與 `ps`；跑完全部測試後 `pgrep -fl "agend holder"` 什麼都不印）；kill 只對自己起的、大於 1 的 pid
- [x] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」（verifier 的 kill 只對自己起的 pid、在沙箱裡跑；r1 REFUTED 已修，r2 CONFIRMED `6316d42`）

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。`<N>` 這類尖括號是會變的數字或 id。

**draft PR 還沒 merge 時**，下面所有的 `~/Documents/Hack/AgEnD-v2` 都改成 PR 的 worktree `~/Documents/Hack/AgEnD-v2-gate06`。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.0.0`。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

步驟 1–3 看的是同一次 `accept` 的輸出。步驟 4 起用同一個暫存 home，第二個終端也要貼上步驟 4 印出的那行 `export AGEND_HOME=…`。

1. 跑 demo。

   **這步在驗什麼**：demo 在 `/tmp/g6-…` 起真 daemon、真 holder，下面各段都跑完。錯了代表 daemon 最基本的「起 holder、接得上」不成立。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo xtask accept daemon-holder
   ```

   應該看到：依序 `== restart`、`== give-up`、`== crash-before-spawn`、`== env`、`== second-daemon`、`== orphan`、`== cleanup`，倒數第二行 `daemon demo: all sections passed`，最後一行 `gate 6 (daemon-holder): checks passed`。整段約 3–5 分鐘（前面是測試，demo 本身約 50 秒）。

   - [x] 通過

2. 四次開機，其中一次 daemon 被硬殺。

   **這步在驗什麼**：daemon 換了四次、其中一次沒機會好好結束，holder 與計數器都沒斷（D3、P4）。錯了的話 daemon 當掉一次，所有 agent 就跟著死。

   操作：同一次輸出，找 `== restart`。應該看到（2026-09-26 實跑）：

   ```text
   boot 1 daemon pid=<A> holder pid=<H> (work: started g6-da); Ctrl-C: exited in <t> ms, holder still runs, no Shutdown
     agend.db opened (waited <w> ms for the lock)
   boot 2 daemon pid=<B> (idle) holder pid=<H> counter=<c2>
   boot 3 daemon pid=<C> holder pid=<H> counter=<c3> (work: started g6-db pid=<H2>); killed -9 by test (signal: 9 (SIGKILL))
   boot 4 daemon pid=<D> holder pid=<H> counter=<c4> ok (g6-db pid=<H2> too)
     after the kill -9: agend.db opened (waited <w> ms for the lock)
   4 boots, 4 daemon pids, the same holder pid=<H>, counter kept growing
   negative check (new AGEND_HOME each boot): boot 2 failed: daemon pid=<E> recovered 0 holder(s), expected 1 (agend daemon ready: instances=0 recovered=0 started=0 orphans=0)
   ```

   | 看什麼 | 意思 |
   |---|---|
   | A、B、C、D 都不同，H 每行都一樣 | daemon 換了四次，holder 沒換 |
   | c2 < c3 < c4 | 計數器一直在跑，daemon 拿到的是當下的畫面 |
   | `killed -9 by test`，下一行照樣接回 | 硬殺 daemon 也不影響 holder |
   | 最後一行 `boot 2 failed` | 反向檢查：每次換新的 home 就接不回來——證明這套檢查分得出「真的跨重啟」 |

   - [x] 通過

3. 故意弄壞：agent 一起來就死。

   **這步在驗什麼**：daemon 會用 `--resume` 補救，但只試 3 次，之後交給人，絕不全新啟動（P6）。錯了的話會無限重起燒 token，或把 agent 的對話丟掉重來。

   操作：找 `== give-up`。應該看到（`<S>` 是同一個 session id）：

   ```text
   g6-dx: start --session-id <S>
   agent g6-dx exited (code=1)
   g6-dx: restart 1/3 --resume <S>
   agent g6-dx exited (code=1)
   g6-dx: restart 2/3 --resume <S>
   agent g6-dx exited (code=1)
   g6-dx: restart 3/3 --resume <S>
   agent g6-dx exited (code=1)
   g6-dx failed: restarted 3 times in 10m and it still died; not restarting
   agent's own record of its starts: --session-id <S> | --resume <S> | --resume <S> | --resume <S> (1 first start, then only --resume)
   DB status: failed; no restart 4/3 in the 7 s after
   ```

   第一行是 instance 的第一次啟動（全新、帶 `--session-id`）；之後每次重起都帶 `--resume`。「agent's own record」是假 agent 自己把收到的參數寫進檔案，不是 daemon 說的。

   再找 `== crash-before-spawn`（daemon 在第一次 `Spawn` 前被硬殺，verifier r1 F1）。應該看到：

   ```text
   attempt 1: daemon killed -9 right after `g6-dc1: start --session-id <S>`; agent had not run; holder <…>
   next boot: g6-dc1: <reconnected … it had no agent, started one 或 start --session-id <S>>
   agent's own record: --session-id <S> (the session is created, not resumed)
   DB status: running
   ```

   重點：session 還沒建立時，下次開機仍用 `--session-id`，不會 `--resume` 一個不存在的 session。

   - [x] 通過

4. 你自己動手：加一個 instance，前景啟動 daemon。

   **這步在驗什麼**：真的 `agend daemon` 從 DB 讀到 instance、起 holder、印出就緒（P1、P2、P5）。錯了的話後面的步驟都沒有東西可驗。

   在第一個分頁（先跑過開頭那段）：

   ```bash
   export AGEND_HOME="$(mktemp -d /tmp/g6.XXXX)" && echo "export AGEND_HOME=$AGEND_HOME"
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- add g6-1
   agend daemon
   ```

   應該看到：

   ```text
   added g6-1: claude session <S> in /tmp/g6.<xxxx>/workspace/g6-1
   <時間> agend daemon starting: pid=<D1> home=/tmp/g6.<xxxx>
   <時間> agend.db opened (waited <w> ms for the lock)
   <時間> housekeeping: DB snapshot /tmp/g6.<xxxx>/backups/agend-<日期>.db (<n> bytes)
   <時間> shims: git, kill, killall, pkill -> <…>/target/debug/agend
   <時間> g6-1: start --session-id <S>
   <時間> g6-1: holder pid=<H> started
   <時間> agend daemon ready: instances=1 recovered=0 started=1 orphans=0
   ```

   daemon 留在前景，不要關這個分頁。記下 `export AGEND_HOME=…` 那一行。

   - [x] 通過

5. 故意弄壞：在 daemon 的分頁按 Ctrl-C。

   **這步在驗什麼**：daemon 5 秒內結束，holder 和裡面的計數器還在（P1、第 4 施工關 P2）。錯了的話每次停 daemon 所有 agent 都會死。

   操作：在 daemon 的分頁按 Ctrl-C。應該看到兩行 `agend daemon stopping (SIGINT); holders keep running`、`agend daemon stopped`，並馬上回到 shell（實測 0.01 秒）。

   開第二個終端（先跑開頭那段、再貼上 `export AGEND_HOME=…`）：

   ```bash
   pgrep -fl "agend holder g6-"
   ~/.cargo/bin/cargo run -q -p agend-holder --example holder_probe -- snapshot g6-1 | grep counter | tail -1
   ```

   應該看到：一行 `<H> …/target/debug/agend holder g6-1`（H 與步驟 4 相同）；`counter=<n>`。隔幾秒再跑第二行，n 變大。

   - [x] 通過

6. 再啟動 daemon，看它接回。

   **這步在驗什麼**：新 daemon 接回原本的 holder、拿到當下的畫面，不會多生一個（P2、P4）。錯了的話重啟 daemon 會多出 holder，或 agent 被重起。

   操作：在第一個分頁再跑 `agend daemon`；在第二個分頁再跑一次 `pgrep -fl "agend holder g6-"`。

   應該看到：

   ```text
   <時間> g6-1: reconnected to holder pid=<H>; screen: counter=<m>
   <時間> agend daemon ready: instances=1 recovered=1 started=0 orphans=0
   ```

   H 與步驟 5 相同、m 比步驟 5 的 n 大；`pgrep` 仍只有一行、同一個 pid。

   - [x] 通過

7. 故意弄壞：在第二個分頁再起一個 daemon。

   **這步在驗什麼**：同一個 home 只能有一個 daemon（P1）。錯了的話兩個 daemon 同時驅動同一批 agent。

   ```bash
   agend daemon; echo "exit=$?"
   ```

   應該看到：約 10 秒後（實測 10.2 秒）`agend daemon: agend.db is in use by another process (is another agend daemon running?)`，接著 `exit=1`；第一個分頁的 daemon 沒有多印任何一行。

   - [x] 通過

8. 故意弄壞：硬殺 holder，看 daemon 用 `--resume` 接回（沙箱裡跑，只砍步驟 6 那個 pid）。

   **這步在驗什麼**：D3 的上限（holder 被硬殺、agent 一起死）發生後，daemon 會補救（P6）。錯了的話 holder 一死，這個 agent 就一直停著。

   在第二個分頁：

   ```bash
   PID=$(cat "$AGEND_HOME/run/holders/g6-1.lock"); echo "$PID"
   ```

   確認印出的就是步驟 6 的 H，而且大於 1。然後：

   ```bash
   ~/Documents/Hack/AgEnD-ops/probe-sandbox.sh /bin/kill -9 "$PID"
   ```

   第一個分頁（daemon）應該看到：

   ```text
   <時間> holder g6-1 died
   <時間> g6-1: restart 1/3 --resume <S>
   <時間> g6-1: holder pid=<H3> started
   ```

   `died` 在 1–2 秒內出現，`restart` 在它之後約 5 秒。接著在第二個分頁：

   ```bash
   pgrep -fl "agend holder g6-"
   ~/.cargo/bin/cargo run -q -p agend-holder --example holder_probe -- snapshot g6-1 | head -1
   ```

   應該看到：`pgrep` 又是一行，pid 是新的 H3；第二行 `agent args: --resume <S>`（新的 agent 自己印出它收到的參數，S 與步驟 4 相同）。`snapshot` 會暫時搶走 daemon 的連線，daemon 1 秒後自己接回，不影響 agent。

   - [x] 通過

9. 孤兒巡查與收尾。

   **這步在驗什麼**：DB 裡已經沒有的 instance，它的 holder 在下次開機被收掉，不會變孤兒（第 4 施工關 P2；本頁 P2）；最後什麼都不留。

   操作：第一個分頁按 Ctrl-C 停 daemon，然後：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- remove g6-1
   agend daemon
   ```

   應該看到：`removed g6-1`，接著

   ```text
   <時間> orphan g6-1: Shutdown sent
   <時間> agend daemon ready: instances=0 recovered=0 started=0 orphans=1
   ```

   在第二個分頁 `pgrep -fl "agend holder g6-"` 什麼都不印。最後在第一個分頁按 Ctrl-C，再 `rm -rf "$AGEND_HOME"`。

   - [x] 通過

## 待你追認

實作時做了、提案沒寫到或與提案字面不同的選擇。確認前照目前的做法運作。每項：決定 · 理由 · 反悔的成本。

**追認結果**：使用者 2026-09-26 全部追認 H1–H16（H14 單獨明確允許：測試可以對自己起的 daemon 子程序送 SIGINT，pid > 1、還沒回收）。

| # | 決定 | 理由 | 反悔成本 |
|---|---|---|---|
| H1 | instance 狀態只有 `new`（session 還沒建立）、`running`（第一次 `Spawn` 已被 holder 確認：`Spawned` 或 `already_spawned`＝agent 程序已經啟動；之後每次啟動都 resume）、`failed`。**與 P3 字面不同**：instance 那一列在任何啟動前就在 DB，但 `running` 是在第一次 `Spawn` 被確認**之後**才寫；接回時一律重送 `Spawn`（holder 已有 agent 就回 `already_spawned`、什麼都不變）。所以 daemon 死在「起 holder」與「第一次 `Spawn` 確認」之間時，下次開機重送的仍是 `--session-id`，不是 `--resume` 一個沒建立過的 session（verifier r1 F1，測試 `a_daemon_killed_before_the_first_spawn_still_starts_the_session_fresh`）；第一次啟動失敗的重起也帶 `--session-id` | 先寫 `running` 再 `Spawn`（原本的做法）會讓那個窗口裡的 claude 只拿到 `--resume`、3 次後 `failed`；「DB 不知道的 agent」本來就不會發生，因為 instance 那一列早就在 DB。剩下的極小窗口：agent 已跑、`running` 還沒寫、holder 又死掉 → 下次用 `--session-id` 起，claude 會拒絕、3 次後 `failed`、交給人（不會全新丟掉對話）。另一個已知限制（verifier r2 R1，沒有真 claude 無法測）：「`Spawn` 被確認」只代表程序起來了，不保證 claude 已經把 session 存檔；claude 在存檔前就死掉（或 session 要等第一則訊息才建立、而 agent 還沒收到訊息就連 holder 一起死掉，例如重開機），之後每次都 `--resume` 一個不存在的 session，3 次後 `failed`、交給人——照 P6 不會全新啟動。第 12 施工關接真 claude 時要實測 session 何時落地 | 改回先寫 `running`：`supervisor::start`／`reconnect` 各搬一行 |
| H2 | claude 的 session id 在加 instance 時就產生（UUID v4，`daemon_probe add`）：session 建立前（`new`）的每次啟動帶 `--session-id <id>`，建立後（`running`）只帶 `--resume <id>`。codex、opencode 還沒有 session id：可以全新啟動直到第一次 `Spawn` 被確認，之後一死就 `failed` | 頁面只說「有 session id 就帶 `--resume`」；自己給 id 就不必從 claude 的輸出抓，第一次啟動就有 id 可接 | 改成啟動後從 hook／輸出讀 session id：多一個寫回 DB 的步驟 |
| H3 | agent 環境白名單定為：daemon 設 `AGEND_HOME`、`AGEND_INSTANCE`、`PATH`（`$AGEND_HOME/bin:` + daemon 的 `PATH`，沒有時 `/usr/bin:/bin`）；從 daemon 複製 `HOME`、`USER`、`LOGNAME`、`LANG`、`LC_ALL`、`LC_CTYPE`、`TMPDIR`、`TZ`。**不**轉傳其他 `AGEND_*`（例如 `AGEND_SHIM_BYPASS`）與 daemon 的 `TERM`（holder 自己設 `xterm-256color`，那才是 agent 的終端） | 頁面寫「`AGEND_*`…、`TERM`…（開工時細化）」；轉傳 `AGEND_SHIM_BYPASS` 會讓 agent 繞過 shim；daemon 的 `TERM` 描述的是 daemon 的終端 | `runtime/env.rs` 的 `PASS_THROUGH` 加名字 |
| H4 | daemon log 用自己寫的約 40 行 writer，不用 tracing：每行同時寫 stderr 與 `logs/daemon-YYYY-MM-DD.log`；拿到 `agend.db` 之前只寫 stderr | 少一組依賴（tracing + subscriber + 按日期換檔）；前景跑時人要在終端看到 `ready`；第二個 daemon 不能改到第一個的 log（步驟 7） | 換成 tracing：`log::line` 改成 `tracing::info!`，加一個按日期開檔的 writer |
| H5 | 「每小時補做今天還沒做的」做成**每小時把全部做一次**：`prune`、今天的 DB 快照（已存在就跳過）、log／audit／holder log 期限都是冪等的 | 不必記「今天做過沒」；結果相同 | 加一個「上次做的日期」變數 |
| H6 | 期限的「留 N 天」＝含今天共 N 個日曆日（UTC）：daemon log 留 7 個檔；audit 留 13 份輪替檔＋今天的 `shim.jsonl`＝14 份。audit 依 `shim.jsonl` 的修改日期輪替成 `shim-<那天>.jsonl`；那個名字已存在就不輪替、記一行 log | 用檔名的日期刪、不看 mtime（複製會改 mtime），與第 5 施工關 S15 一致；同名只會在時鐘倒退時發生 | 改算法：`housekeeping.rs` 的 `delete_dated` 一行 |
| H7 | 每個 holder 的長連線是一條 std thread（活得跟 holder 一樣久），`spawn_blocking` 只用在一次性的呼叫（起、停）；不在 tokio runtime 裡時（testkit 的 `block_on` 跑契約）一次性呼叫直接在呼叫端執行 | tokio 建議長時間阻塞的工作用自己的 thread，`spawn_blocking` 的 pool 有上限；契約測試不必另外起 runtime | 長連線改 `spawn_blocking`：`link.rs` 一行，但每個 holder 永久佔一個 blocking slot |
| H8 | `failed` 的 instance：它的 holder（agent 已結束）**留著**，保留最後的畫面給人看；daemon 放棄時關掉自己對它的連線（verifier r1 F4），所以沒有人連上時 holder 的 24 小時安全網（第 4 施工關 P2）會讓它自己結束；開機時不接回也不算孤兒 | P6「交給人」：畫面是人判斷的依據 | 放棄時順便 `Shutdown`：`supervisor::fail` 一行 |
| H9 | 重起前若 holder 還在（agent 自己結束的情況），先送 `Shutdown` 等它結束，再起新的 holder 帶 `--resume` | holder 一生只跑一個 agent（第 4 施工關 G10），不能在同一個 holder 裡重起 | 無（這是唯一做法），列出來只因為頁面沒寫 |
| H10 | 事件依序處理：Ctrl-C 剛好碰上「起 holder／重起」時，要等那一步做完（可能超過 P1 的 5 秒）才結束 | 依序處理最簡單，不用處理同一個 instance 同時被兩件事改；平常 Ctrl-C 實測 0.01–0.03 秒 | 起 holder 改成背景工作、Ctrl-C 時取消：多一套取消邏輯 |
| H11 | 起不來（5 秒內連不上、`Spawn` 被拒）算一次「死掉」，走同一套 5 秒／3 次／`failed` | 頁面 P6 列了「holder 起不來」；同一個計數最簡單 | 改成起不來立刻 `failed`：`supervisor` 一個分支 |
| H12 | 要真 `agend` binary 的測試放 `crates/agend/tests/`（`holder_runtime.rs`、`daemon_process.rs`），各段程式放 `crates/agend-daemon/tests/common/daemon_process.rs`，測試與 `daemon_probe demo` 跨 crate 用 `#[path]` 共用；`xtask accept daemon-holder` 的 crates 因此是 agend-daemon、agend-holder、agend、agend-core | agend-daemon 的測試拿不到 `agend` binary（比照第 4 施工關 G8）；demo 印的就是測試驗的（比照第 5 施工關 S14） | 各自一份：demo 與測試分開寫 |
| H13 | 測試與 demo 的 home 放 `/tmp/g6-<pid>-<n>`，不放 `$TMPDIR` | macOS 的 `$TMPDIR` 約 49 bytes，加上 `run/holders/contract-boot1.sock` 超過 holder 的 100 bytes 上限 | 無 |
| H14 | **與你的安全清單字面不同，請明確決定**：你列的測試可用手段是 `Child::kill()`／`wait()`、協定 `Shutdown`、對自己 home 鎖檔裡的 pid 送 SIGKILL；測試另外對**自己的** daemon 子程序送 SIGINT（`libc::kill`，先確認 pid > 1、還沒回收）來模擬 Ctrl-C。硬殺 daemon 仍用 `Child::kill()` | 頁面 P1 要驗 Ctrl-C，只有真的送 SIGINT 才驗得到；對象仍只有自己起的程序 | 不驗 Ctrl-C：刪掉 `Daemon::interrupt`，四次開機改用 `Child::kill`（就驗不到「Ctrl-C 後 holder 沒收到 `Shutdown`」） |
| H15 | 開機的 `ready` 行數字是計畫的動作數（`recovered` = 要接回的個數），不是成功數；接回失敗會另外記一行、走 H11 | 數字與 `plan_boot` 一一對應，好對照 | 改成成功數：`BootReport` 計數位置 |
| H16 | 契約 fixture 的 `is_running` 只看鎖檔（pid 等於 handle 的 pid），不試連 socket；`daemon_probe add` 只做兩種 agent：bash 計數器（預設）與 `--dies`（立刻 `exit 1`），不能指定任意程式 | CONTRACTS 寫「程序存在且 socket 連得上」，但連 socket 會搶走 runtime 自己的長連線（第 4 施工關 P3 的註）；`daemon_probe` 不是 production 命令，第 9 施工關 `agend instance add` 才做完整參數 | 讓 `add` 收 `-- <program> [args]`：約 10 行 |

另記（事實）：migration 0002 讓 schema 版本變成 2，第 5 施工關頁步驟 2、8 的「應該看到」已照實跑改好（該頁進度紀錄有記）。

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
| 2026-09-26 | 通過 | 在 `feat/gate-06-daemon`（merge 前）由 agent 帶著走 9 步。步驟 1–3：同一次 `accept daemon-holder` 輸出對上（四次開機 holder 都是 87708、Ctrl-C 21 ms、give-up 1 次 `--session-id` + 3 次 `--resume` 後 failed、crash-before-spawn 下次仍 `--session-id`）。步驟 4–9 兩個分頁手動：Ctrl-C 後 holder 96226 還在（counter 53→82 接回、`recovered=1 started=0`）；第二個 daemon 10 秒後 in use、exit=1；`kill -9` holder 後 5 秒 `restart 1/3 --resume <S>`、新 agent 自己印 `--resume <S>`；remove 後 `orphan g6-1: Shutdown sent`、`no holders`；暫存 home 已刪。 |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-26 verifier r1 REFUTED（`8222f7d`）修正：F1 MEDIUM claude 第一次 `Spawn` 前 daemon 當掉 → 之後只拿到 `--resume` 沒建立過的 session：`running` 改成第一次 `Spawn` 被確認後才寫、`new` 一律 `--session-id`（H1、H2 改寫；回歸測試 `a_daemon_killed_before_the_first_spawn_still_starts_the_session_fresh` 修前失敗、修後通過；demo 加 `== crash-before-spawn`）；F2 只有 instance 的 DB 也做每日快照（`a_database_with_only_instances_takes_its_daily_snapshot`）；F3 一個鎖住卻沒有活 pid 的鎖檔不再讓整個開機失敗，跳過並記警告（`a_locked_file_without_a_live_pid_is_skipped_not_fatal`）；F4 放棄時關掉對 holder 的連線（H8 改寫）；F5 第 5 施工關頁的 demo 輸出更新；H14 標明與安全清單字面不同。
- 2026-09-26 使用者親自驗收 9 步通過（merge 前，branch `feat/gate-06-daemon`）。
- 2026-09-26 使用者追認 H1–H16（H14 單獨明確允許）。
- 2026-09-26 fresh-context verifier r2 CONFIRMED（`6316d42`）：r1 五項都修好（F1 的回歸測試拿掉修正會失敗）；另記兩個 LOW 文件差異（步驟 4 少了 `housekeeping: DB snapshot` 那行、第 5 施工關步驟 6 少了 `instances`）已補，H1 補上 R1 已知限制（「`Spawn` 被確認」≠ claude session 已存檔，第 12 施工關實測）。
- 2026-09-26 實作（draft PR，branch `feat/gate-06-daemon`）：`agend daemon`、`instances` 表（migration 0002）、`plan_boot`、`HolderRuntime`（長連線、環境白名單、shim symlink）、P6 重起、housekeeping（prune、DB 快照、log／audit／holder log 期限）、`daemon_probe` 與 demo、契約三層、check-deps 規則、core 版本測試；「你親自驗收」步驟 1–9 改成確切指令與實跑輸出（步驟 4–9 由實作者用只對自己子程序送訊號的 harness 預演過）；「待你追認」H1–H16。已知風險「EXCLUSIVE 鎖交接的 10 秒」實測：舊 daemon 收到 Ctrl-C 時新 daemon 等了約 420 ms；四次開機（前一個已結束才起下一個，含 `kill -9` 之後）等 2–4 ms。fresh-context verifier 尚未跑。
- 2026-09-26 使用者追認 P3 夜間更正（不設 `process_group(0)`）與 P7–P9；第 4、5 施工關已 merge。
- 2026-09-26 開工前提案 P1–P9 寫定：P1–P6 使用者 2026-09-25 確認；P7–P9 夜間照建議代填、待你追認。舊步驟修正（拿掉 `--foreground`、`spawn-fake` 改 `daemon_probe`、`pgrep` 限 `g6-`）；`install_hooks` 移到第 10 施工關（P9）；狀態改為提案中。

## 下一步

```bash
cat docs/gates/gate-06-daemon-holder.md          # 先看「待你追認」
~/.cargo/bin/cargo xtask accept daemon-holder    # 你親自驗收步驟 1
```
