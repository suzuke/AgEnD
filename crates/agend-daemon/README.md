# agend-daemon

> **TL;DR**
> - 唯一的大型 I/O 層：常駐、單一 tokio runtime、DB 專屬執行緒；`agend daemon` 起 holder、接回 holder、agent 死了用 `--resume` 接回；開機計畫做完才開 `run/daemon.sock` 講 client protocol 1.5（一般 client 仍只需 1.3）；codex 經 app-server 的 JSON-RPC 送達、自己建 thread、死了照樣 resume（第 7 施工關）；`agend send`／`inbox`、`agend instance add|remove`、`agend daemon restart`（預檢後原地 `exec`）在這裡處理（第 9 施工關）。
> - 記住：**daemon 停掉時 holder 與 agent 照跑（D3）**；同一個 `AGEND_HOME` 只有一個 daemon（`agend.db` 的鎖）；`agend.db` 只有 daemon 開（`store`）；socket 連得上＝daemon 好了。
> - 下一步：第 10 施工關 pipeline 驗證：`cargo xtask accept pipeline`；還原 DB 快照的步驟見下方「store」。

正式 P5 啟動按鍵與 P6 初始 idle 的實作、schema v9 與 native 驗證邊界見 [啟動處理](../../docs/gates/gate-12a-startup-runtime.md)。

第 12B 已接入 OpenCode Driver、supervisor／holder、REST 分頁對帳與 operator 權限回覆；原生契約及固定版本真測通過，#155 最終覆核／CI 進行中。範圍與限制見 [OpenCode](../../docs/gates/gate-12b-opencode.md)。
private startup capture 在啟動前登記 manual 模式，停用 daemon 自動鍵與初始 resize，所有蒐證輸入仍受原授權計畫限制。

## 第 10 施工關（已驗收，2026-10-02）

單一 pipeline queue、SQLite schema v7（pipeline migration `0005`）、真 git／Runner／LocalForge、binding 與 hook 生命週期、checks 沙箱、重啟／每日對帳、team／workflow／task／請示／提醒已接通；執行規則見 [pipeline runtime](../../docs/architecture/pipeline-runtime.md)。人工核准、attention_reason 清除與結果 receipt 在同一筆 store CAS transaction 完成後才發布真 action；失敗或衝突不改任何投影；通知型 timeout 留在同一核准 ticket 時不移除再重建 attention。

## 第 11 施工關 C 段（已驗收並合併 #145）

holder 1.1／runtime／client 1.4 已接通完整 frame、歷史、多視窗控制、resize 及 TUI；fake C 契約與完整 U17 已通過。真 Codex 0.159.3 首次 U17 有獨立核對，使用者要求剩餘行為自動驗證；最終 head verifier／CI、清理與已確認合併紀錄見 [驗收收尾](../../docs/gates/gate-11c-closeout.md)。 [版本政策](../../docs/gates/gate-11c-codex-input.md)。

Codex history 對帳已拒絕外來 clientId 的文字 fallback；no-turn Queued 的舊 crash fallback 要有 attempted_at，未嘗試送出不算 receipt。[U17 基礎證據與仍存歧義](../../docs/gates/gate-11c-u17-validation.md)。

pipeline 補 failed attention 的 unblocks 時，經 core port 原子比對捕捉值再更新；Retry 已移除或新失敗已替換的項目不被舊快照重建。[CI 反例](../../docs/gates/gate-11c-regression-validation.md)。

一般 daemon 只允許 holder 啟動時辨識為 codex-cli 0.159.3 的人工輸入；未知、其他版本、舊 holder 缺版本記錄及未連線均拒絕。曾允許輸入的 thread 以 migration 0006 永久記錄，live／reconcile／events 都只接受自己的 clientId；版本降級或 daemon 重啟不回到文字匹配。Queue 的 Confirmed row 保存實際 turn id。 [版本政策](../../docs/gates/gate-11c-codex-input.md)。

## 負責

- 入口：protocol server、command handlers、hook／事件接收（含磁碟佇列補送）
- 領域：驅動流水線、送達、監督（卡住、額度、轉派）、排程（timeout、cron）、DB ↔ git 對帳
- adapter：driver（codex、claude、opencode）、agent runtime（holder client）、forge（local、github）、git、runner、store（SQLite）、notifier（Telegram）

## 不負責

- 持有 PTY 或 agent 程序（holder 做）
- 從 agent 的 cwd 推 task（改用身分 → binding）
- 在 PTY 打字送訊息
- 實作流水線轉換規則（在 core）

## 模組

| 模組 | 職責 |
|---|---|
| `daemon` | `agend daemon`：前景跑、開 DB（重試 10 秒）、開機順序、訊號（第 6 施工關）；重啟時收尾後 `exec` 新 binary（第 9 施工關） |
| `preflight` | `agend daemon preflight <dir>`：新 binary 在暫存 home 對 DB 複本跑 migration＋`quick_check`、起一個自己的 holder（第 9 施工關 P7） |
| `reaper` | `exec` 重啟後，只對開機時鎖檔裡的 pid `waitpid(pid, WNOHANG)`，收掉繼承來的 holder（第 9 施工關 P7） |
| `boot` | `plan_boot`：開機時接回／啟動／孤兒，純函式 |
| `housekeeping` | 開機與每小時：`prune`、DB 快照、daemon log、audit、holder log 的期限 |
| `log` | daemon log：stderr ＋ `logs/daemon-YYYY-MM-DD.log` |
| `server` | client protocol server：`run/daemon.sock`、JSON Lines、`hello`、寫入 5 秒逾時、落後就斷（第 8 施工關） |
| `terminal_hub` | 入口服務：每 instance 有界操作、socket-scoped view／owner、dirty frame 與 EOF 清理（C 段） |
| `handlers` | 請求處理，與傳輸分離；依身分限權（`command` 只收 agent、`operator`／`resolve_attention` 只收操作者）；`handlers::agent`（`status`、`send`、`inbox`）、`handlers::operator`（instance、restart 預檢） |
| `fleet` | 全貌（instance、task、「需要你」）與事件記錄（最近 1024 筆、broadcast），同一把鎖（第 8 施工關） |
| `ingest` | hook 與結構化事件接收、磁碟佇列補送 |
| `pipeline` | 經 core ports 注入 Store／Driver／executor／Clock／view，單一 queue 執行 core 狀態機 |
| `pipeline_runtime` | 組裝 SQLite、Codex、git、bindings 與 checks adapters；queue 不依賴 concrete adapter |
| `delivery` | 送達模型：四個狀態各代表什麼（第 7 施工關 P5）、`render`（`From:`／`Task:` 標頭＋完整 body） |
| `supervisor` | 讓 DB 裡的 instance 保持在跑：死了等 5 秒 `--resume`、10 分鐘 3 次仍死就 `failed`（變成「需要你」項目，操作者可 `retry`）；之後：卡住、額度、轉派、例外才找人 |
| `scheduler` | timeout、cron |
| `reconcile` | 開機與每日 DB ↔ git 對帳 |
| `driver::{codex,claude,opencode}` | backend 結構化 API；codex 見下方「codex（第 7 施工關）」；Claude push 已合併，見[接入說明](../../docs/gates/gate-12a-driver.md)；OpenCode REST 接入見[第 12B](../../docs/gates/gate-12b-opencode.md) |
| `runtime` | `HolderRuntime`：起 holder、holder 協定 client、每個 holder 一條長連線（轉出 `PtyBytes` 給終端訂閱者）、agent 環境白名單、shim symlink |
| `forge::{local,github}` | 提交與 merge |
| `git` | 建立／移除 worktree 與 branch（先記錄再建立） |
| `runner` | 在 head 的臨時 worktree 跑 `command` 關卡 |
| `store` | SQLite，唯一持久狀態：`SqliteStore`（`Store` trait + `save_workflow`、`load_events`、`prune`、`snapshot`、instance 的增刪查） |
| `notifier` | Telegram topic |

## daemon（第 6 施工關）

| 項目 | 內容 |
|---|---|
| 啟動 | `agend daemon`，只在前景跑；home 由 `agend` 的 `home::resolve` 給（第 9 施工關 P3：沒設 `AGEND_HOME` → `agend: AGEND_HOME is not set; choose a directory …`、exit 2；v1 home 拒絕、exit 1）；其他參數 → CLI 的用法錯誤、exit 2 |
| 只有一個 | `agend.db` 被別的程序開著就每 200 ms 重試，10 秒後 `agend daemon: agend.db is in use by another process (is another agend daemon running?)`、exit 1；拿到 DB 前不寫 `logs/` |
| 開機順序 | 開 DB → housekeeping（失敗只記錯）→ `bin/` 的 shim symlink（git、gh、kill、killall、pkill → 目前的 binary）→ `plan_boot` → `agend daemon ready: instances=N recovered=R started=S orphans=O` |
| `plan_boot` | DB 有、鎖被持有 → 接回（重送 `Spawn`，holder 回 `already_spawned`）；DB 有、沒鎖 → 啟動；DB 沒有、鎖被持有 → 孤兒，送 `Shutdown`；`failed` 的不動 |
| instance | `instances` 表（migration 0002，永久保留）：id `[a-z0-9-]{1,24}`、backend、program、args、working_directory、session_id、status（`new`／`running`／`failed`） |
| 啟動 holder | `agend holder <id>`，環境只有 `AGEND_HOME`；每 50 ms 試連、5 秒內連不上算失敗；第一次 `Spawn` 被 holder 確認後才把 `new` 改成 `running`（session 存在，之後都 resume）；自己起的 holder 由一條 thread `wait` 收屍 |
| agent 環境 | 白名單：`AGEND_HOME`、`AGEND_INSTANCE`、`PATH`（`$AGEND_HOME/bin` 開頭）、`HOME`、`USER`、`LOGNAME`、`LANG`、`LC_ALL`、`LC_CTYPE`、`TMPDIR`、`TZ`；其他（例如 `TELEGRAM_BOT_TOKEN`、`AGEND_SHIM_BYPASS`）一律不給 |
| session | claude：`new` 時 `--session-id <id>`，`running` 後只用 `--resume <id>`；codex：daemon 建的 thread，TUI 一律 `resume <thread>`（第 7 施工關）；opencode：私人 serve／attach handoff 保存原 session，holder 重啟恢復原 session；遺失時拒絕替換上下文 |
| 死掉之後 | agent 結束、holder 死了（連線斷＋鎖放掉）、啟動失敗 → log → 5 秒後 `restart N/3 --resume <id>`（還是 `new` 就 `--session-id`）；10 分鐘內 3 次仍死 → `<id> failed: …`，不再起、關掉對 holder 的連線；次數只在記憶體 |
| Ctrl-C | SIGINT／SIGTERM：關 holder 連線、關 DB、exit 0；**不送 `Shutdown`**，holder 照跑 |
| log | stderr ＋ `logs/daemon-YYYY-MM-DD.log`（UTC，0600），留 7 天；`audit/shim.jsonl` 每天輪替成 `shim-YYYY-MM-DD.jsonl`、留 14 天；`run/holders/<id>.log` 在 holder 不在、7 天沒動時刪 |

加減 instance：`agend instance add|remove|list`（第 9 施工關，daemon 跑著時）。`daemon_probe add|remove|list` 只剩開發與舊驗收用（daemon 停著時直接改 DB）。

## codex（第 7 施工關）

| 項目 | 內容 |
|---|---|
| 程序 | holder 的 PTY 子程序是固定的 `sh` 包裝（`driver::codex::launch::WRAPPER`，`$0`＝`agend-codex`）：背景起 `codex -c … app-server --listen unix://$AGEND_HOME/run/holders/<id>.codex.sock <instance 的 args>`（輸出接到 holder log），等交接檔 `run/holders/<id>.codex-go`（最多 60 秒，否則 exit 1），再 `exec codex -c … resume <thread> --remote unix://<解析後的 socket>`；兩個程序同一個 process group。holder 協定不變 |
| 設定 | 每次啟動的 `-c`：trust `projects={"<realpath 工作目錄>"={trust_level="trusted"}}`、`check_for_update_on_startup=false`；app-server 另有 `approval_policy="never"`、`sandbox_mode="danger-full-access"`（`thread/start` 也帶）。不設 `CODEX_HOME`，**不寫 `~/.codex`** |
| shim | codex agent 的環境多一個 `ZDOTDIR=$AGEND_HOME/zsh`；每次開機重寫 `zsh/.zprofile`，在 `/etc/zprofile`（`path_helper`）之後把 PATH 還原成 agent 啟動時的樣子：`$AGEND_HOME/bin`，然後 daemon 的 PATH（去掉 `$AGEND_HOME/bin`）；不 source 使用者的 dotfile（K8） |
| 啟動 | `Spawn` 前刪舊的 socket（連它指到的 `codex-daemon-<uid>/` 檔）與 `$GO` → holder → 背景 `connect`：每 100 ms 試連、`initialize`，20 秒放棄；沒有 thread 就 `thread/start`、**先存進 `instances.session_id`**，有就 `thread/resume {excludeTurns:true}`（找不到而且從沒送過訊息 → 建新的，否則 `failed`）；各限 30 秒 → 寫 `$GO`（暫存檔再 rename）→ 對帳、送出 `queued` 的訊息 → 開長連線。失敗＝一次死亡（5 秒／3 次／`failed`） |
| log | `<id>: app-server ready (… ms)`、`thread <T> created`／`thread <T> resumed (idle)`（或 `busy`）、`go (resume <T>)`、`<m> (<level>) → <方法> → sent (turn …)`、`<m> confirmed (turn …)`、`approval declined (gate 7 has no handler): …`、`app-server is gone (no connection for 20 s)`、`sweep of agent group <G> (…): already gone`（或 `SIGKILL sent (…)`） |
| 長連線 | 每個 instance 一條 std thread（寫入 10 秒沒進度＝斷線、走重連；關閉時先 shutdown socket、最多等 5 秒 thread：第 9 施工關）：`thread/status/changed` → 忙／閒（不去抖動）；user message 的 `item/completed` → `confirmed`；授權請求一律回 `decline`；斷線每 100 ms 重連 20 秒（重連後 resume、對帳、補送），不行就是 app-server 死了 → 下一次重起先 `Shutdown` holder；turn 結束、閒置而 codex 佇列非空時送一次 `thread/queue/start`（中斷後 codex 不會自己開始，K16） |
| 清掃 | 發現 holder 死了（決定重起或 `failed` 之前）、起新 holder 之前、開機時 holder 已不在的 `failed`：`agent_pid` 有值才做；1 < pgid ≤ `i32::MAX`、group 還有程序、而且有程序的 argv 有一個元素完全等於 socket 路徑（或 `unix://` 加上它）或 `resume` 後面等於 thread id，才對 group 送一次 SIGKILL；之後清掉 `agent_pid` |
| 送達 | `CodexDriver::deliver(instance, message, level)`：`messages` 表是唯一的冪等（同 id 同內容 → 目前狀態、不呼叫 codex；同 id 不同內容 → `invalid_request`）；閒置一律 `turn/start`；忙碌時 `Queue` → `thread/queue/add`（回覆時已閒置 → `thread/queue/start` 一次）、`Steer` → `turn/steer`（`-32600` → `turn/start`）、`Interrupt` → `turn/interrupt`、等 5 秒、`turn/start`。沒有連線、或 backend 還沒有 driver → 停在 `queued`；instance `failed` → `failed`。送過一次（`attempted_at`）還是 `queued` 的，重送前先對帳，thread 有 turn 在跑時等閒置再看；忙碌但還不知道 turn id 時 `Queue` 照樣 `thread/queue/add` |
| 事件 | `CodexDriver::events(instance, cursor)`：`thread/turns/list` 展開（每個 turn：`BusyChanged{true}`、我們的 user message 各一個 `MessageConfirmed`、結束時 `TurnCompleted{狀態}`、`BusyChanged{false}`）；cursor＝`<turn id>:<slot>` |
| `failed` 的 codex | `retry` 照常（resume 同一個 thread；沒有 thread 就建）；migration 0004 標了 `legacy_no_thread` 的（第 6 施工關留下、有對話沒 thread）不能 `retry`，holder 不動 |

## client protocol server（第 8 施工關）

socket、身分、事件與舊版終端規則見 [protocol server](PROTOCOL.md)；1.4 新終端路徑見 [完整終端入口](TERMINAL.md)。

## CLI 的 daemon 端（第 9 施工關）

權限、status／send／inbox、instance 管理、restart 預檢與 stop 協定見 [protocol server](PROTOCOL.md#cli-的-daemon-端第-9-施工關)。

## 第 12A Claude store 基礎

migration `0007` 及 DB-thread API 保存投遞開始／寫出／ACK／人工放棄；訊息 id 仍是唯一訊息冪等層。protocol 1.5／helper／spool 已接入本批 native bridge；完整 Claude Driver／啟動配置已合併，完整真模型驗收見 [12A smoke](../../docs/gates/gate-12a-complete-smoke.md)。API、retention 與重驗指令見 [Claude store](../../docs/gates/gate-12a-store.md)。

## store（第 5 施工關）

| 項目 | 內容 |
|---|---|
| 檔案 | `$AGEND_HOME/agend.db`（建立時 0600；home、home 不存在的上層目錄、`backups/` 建立時 0700，已存在的目錄不改）；home 由呼叫端傳入 |
| 建立 | 只有 `agend.db` 不存在時才建新 DB：先在 `.agend.db.new` 建好、所有 migration commit 後才 hard link（檔案系統不支援 hard link 時改 rename）成 `agend.db`；上次建到一半留下的 `.agend.db.new` 刪掉重建。`agend.db` 比 SQLite 檔頭（100 bytes）短、schema 版本 0、或缺它那個版本的表 → 拒絕開啟、檔案不動：`agend.db exists but is empty (0 bytes); refusing to start with an empty database — restore a snapshot from <home>/backups (see README)`，照下方步驟還原；指向不存在檔案的 symlink 或不是一般檔案 → `refusing to use <path>: …`。**刪掉 `agend.db` 等於從空 DB 重新開始**；空 DB 不做每日快照；instance 或 Codex thread 歸屬資料也算非空。但寫進第一個 task 後每天的快照照常輪替、一天擠掉一份舊的好快照：要還原請在那之前照下方步驟做 |
| 執行緒 | 一條 `agend-db` 執行緒持有唯一連線；async 方法經 channel（256）送 closure；該執行緒 panic 後每個呼叫回 `store thread stopped` |
| 同時開 | `locking_mode=EXCLUSIVE`，第二個程序：`agend.db is in use by another process (is another agend daemon running?)` |
| 表 | `tasks`、`workflows`、`task_events`、`instances`、`messages`、`teams`、`bindings`、`asks`、`ask_turns`、`reminders`、`codex_input_threads`、`driver_events`、`claude_deliveries`、`claude_owned_files`、`claude_startup`（STRICT）；schema 版本在 `PRAGMA user_version`（目前 12；另含 `opencode_permissions`、`opencode_observed`、`opencode_attempts`），migration 在 `src/store/migrations/`；`0003` 在 `instances` 加 `session_started`（0／1，第一次 `Spawn` 被確認、寫 `running` 的同一個 statement 設 1；既有的 `running` 與 `failed` 的 codex／opencode 設 1）；`0004`（第 7 施工關）加 `messages`（`seq INTEGER PRIMARY KEY AUTOINCREMENT`（清空後也不重用號碼）、`attempted_at_unix_ms`（送出前寫入）、`id` UNIQUE、`from_instance`、`to_instance`、`task_id`、`body`、`level`、`state`、`turn_id`、時間）與 `instances.agent_pid`、`instances.legacy_no_thread`（那一刻 `codex`、沒有 thread、`running`／`failed` 而且 `session_started = 1` 的列設 1 並標 `failed`） |
| 耐久 | WAL、`synchronous=FULL`、`foreign_keys=ON` |
| 保留期限 | `store::retention::RETENTION`：task、workflow、instance、team、binding、請示／回答 receipt、reminder 與 Codex thread 輸入歸屬永久（binding／reminder 按生命週期刪除）；事件與 checks log 14 天；WIP archive 與一般訊息 30 天（`created_at_unix_ms`）；Claude push／OpenCode attempts 未終結訊息與投遞歸屬持續保留，confirmed／failed 由 terminal update 起留 30 天、driver_events 從入庫時間留 14 天；`audit/shim.jsonl` 每日輪替留 14 天、daemon log 7 天、holder log 7 天（第 6 施工關 `housekeeping`） |
| DB 快照 | `backups/agend-YYYY-MM-DD.db`（UTC；DB 沒有任何 task、task event、instance、driver event、Claude 投遞／啟動／自有檔案、OpenCode 權限／歷史去重／attempts 與 Codex thread 輸入歸屬時不做；只剩一般 messages 的既有行為不變，仍視為空），升級前 `agend-YYYY-MM-DD-pre-vN.db`；只留最新 7 份，其他檔案不動 |
| pipeline | `PipelineSnapshot` 存 task 的 `pipeline` 欄，workflow 固定建立時版本；snapshot、task、事件、attention_reason 清除與被接受結果的 dispatch confirmation 同一筆 CAS transaction，保留 failure acknowledgement，不重播事件；`0005` 加 team／role、binding、請示與提醒，詳見 [runtime](../../docs/architecture/pipeline-runtime.md) |

### 還原 DB 快照（手動）

1. 停 daemon（`agend.db` 有 daemon 開著時複製會被它的下一次寫入蓋掉）。
2. `cp "$AGEND_HOME/backups/agend-YYYY-MM-DD.db" "$AGEND_HOME/agend.db"`
3. `rm -f "$AGEND_HOME/agend.db-wal"`（舊的 WAL 屬於被蓋掉的 DB）
4. 啟動 daemon。快照比 binary 新（`schema version N is newer…`）時，改用 `-pre-vN` 那份或裝回較新的 agend。

只想查資料、不還原：`sqlite3 -readonly "$AGEND_HOME/backups/agend-YYYY-MM-DD.db"`（daemon 跑著時 `agend.db` 本身打不開）。

## 依賴規則

- 一般依賴：`agend-core`、`libc`（holder 鎖檔的 `flock`；清掃讀 process group 與 argv、`killpg`）、`rusqlite`（`bundled`）、`tokio`（`io-util`、`macros`、`net`、`rt-multi-thread`、`signal`、`sync`、`time`）、`serde_json`、`toml`、`tungstenite`（只開 `handshake`；codex app-server 的 WebSocket，阻塞 I/O）；`agend-tui`、`agend-shim`、`agend-client` 不能依賴 SQLite 或本 crate；**本 crate 不能依賴 `agend-holder`、`agend-shim`**（只經 holder 協定與子命令，第 6 施工關 P9），**也不能依賴 `agend-client`**（server 與 client 各自編碼，第 8 施工關 P10）（`cargo xtask check-deps`）
- dev 依賴：`agend-testkit`
- 領域模組只透過 `agend_core::traits` 呼叫 adapter

## 入口

- `agend_daemon::driver::codex::{CodexDriver, launch, sweep, socket_connect_path}`（`CodexDriver` 實作 `Driver` trait）
- `agend_daemon::store::SqliteStore::open(home, now_unix_ms)`
- `agend_daemon::daemon::run(home)`（`agend daemon`）、`preflight::run`（`agend daemon preflight <dir>`）
- `agend_daemon::runtime::HolderRuntime`（`Runtime` trait）、`runtime::shutdown_holder`
- `agend_daemon::server::{bind, Server}`、`handlers::Context`、`fleet::Fleet`（測試在同一個程序裡跑 daemon 的 server）


停止訊號直接記在 signal-context atomic flag，Tokio 關閉後到 exec 前仍可讀，避免 restart handoff 遺失 Ctrl-C；完成連線／runtime 清理後，最後 check 到 exec 的窗口由 signal handler 直接成功退出，避免訊號與 exec 競賽；async signal stream 負責把停止事件送進 supervisor。

## 第 12A Claude

`claude_bridge` 提供 client 1.5 Attach／Poll／Hook／Written／Ack；先預約才回完整內容，歷史 hook 不建立 idle。`ingest` 補送 hooks／acks，入庫才刪，不重送內容。[bridge 範圍](../../docs/gates/gate-12a-bridge.md) · [Claude 測試](CLAUDE-TESTING.md)。

[Startup capture](../../docs/gates/gate-12a-startup-capture.md) 預設被動保存真 holder 畫面；額外 opt-in 的 trust／development 模式最多兩鍵／三鍵，只核本次 workspace 與完整已錄製選單，任何 frame 必須符合本次 instance／view／generation。這是無 revision CAS 的 operator 蒐證工具，`startup=not_assessed`，不送模型 prompt／團隊訊息；首次真三鍵執行已按首個失敗停止；新核准 ready 計畫的兩寬蒐證各完成三鍵並保存真主介面，經全新 verifier 有限範圍 CONFIRMED。受控 startup capture 對完整已知選單加一秒穩定等待，仍核控制身分與 completion，不重送；私有 `cleanup-identity.json` 提供本次 session／canonical workspace 的清理歸屬。自有 lab、trust 條目與 session 暫存已清理；保留證據支持目前指定殘留不存在，不能重演已刪除的原始清理身分。真蒐證失敗仍停下，成功也不代表正式 P5／P6 啟動完成。

## 下一步

```bash
cargo test -p agend-daemon
cargo xtask accept daemon-holder   # 第 6 施工關 demo：daemon_probe demo
cargo xtask accept client          # 第 8 施工關 demo：client_demo
cargo xtask accept codex           # 第 7 施工關 demo：codex_demo
cargo xtask accept cli             # 第 9 施工關 demo：cli_demo（在 agend crate）
```

2026-10-06 [D41](../../docs/decisions/d41.md) 允許 Ready 建議內容可變，其他完整畫面／版本／路徑／尺寸不變。
`claude_startup::startup_variable_ready_suggestions_replay_actual_v5_and_both_widths` 經真 daemon／holder／PTY 重播 v5 兩份捕獲及 140 欄變體，核五秒初始 idle 與 Ready 不加鍵；
`startup_variable_ready_rejects_unknown_footer_and_split_hint_without_idle_or_more_keys` 拒絕未知 footer／分行建議。
既有無 SessionStart、人工控制、結果不明與四次開機回歸維持；這些測試不啟動真 Claude、不送模型訊息。

12B OpenCode push 以 supervisor worker 接 loopback REST：claim 與傳輸分離，先持久化 attempt 再送一次，REST 歷史確認收件。原 session 經私人 holder wrapper handoff 恢復；權限由 operator 回覆，unknown 投遞提供 Abandon。原生恢復／權限／DRV 及固定版本模型真測已通過，最終覆核與 CI 以 [12B 紀錄](../../docs/gates/gate-12b-opencode.md) 為準。

12D 設定 parser、private token reference 與固定 Telegram HTTPS API 已建立；通知全文分段與持久逐段收據已接 Notifier 契約；daemon worker 已觀察 needs-you 並持久去重，手機操作已接 guarded pipeline，完整驗收尚未完成。進度見 [Telegram](../../docs/gates/gate-12d-telegram.md)。

Telegram 手機操作先保存 update 與通知消耗意圖，再進入 operator 路徑；未知結果不重送。通知保存任務 CAS 版本與注意事項版本，pipeline 在執行時重新比對。Instance retry 在 supervisor queue 內檢查失敗事件並完成處理後回報；要求修改先提示回覆原因。Inbound polling 與 outbound 分段送出各自執行，停機等待有限 HTTP 呼叫收束。完整第 12D 驗收仍以施工關頁為準。

Protocol 1.6 新增共用已讀收據：`mark_attention_read`、`attention_read` 事件與 fleet `read_keys`。識別沿用事項 ID＋問題次數；後續追問重新未讀。daemon 保存 SQLite，TUI 與 Telegram 共用；已讀不等於回答、核准或解除。舊 daemon 仍使用 TUI 本機已讀。

Telegram team topic 保存目前任務摘要（任務、狀態與階段）；needs-you topic 保留完整請示與操作按鈕。摘要按內容對帳，重啟不重送；未 claim 的輔助通知可恢復，in-flight 未知結果不重送。既有通知綁定原 destination，改 topic 不會自動搬移舊通知。

Telegram delivery 區分 in_flight 與 outcome_unknown，後者供本機 `telegram-delivery:<id>` 處置。操作員 Abandon 保存理由、原文及未知收據前綴，不確認送達、不重送；一般 agent 不可操作。daemon 開機在取得 DB 後恢復未確認意圖，本機處置不依賴 token；此類通知不經 Telegram 再投遞。

第 12C 正式 GithubForge／pipeline 已接入 submit、checks、merge、重啟對帳與持久化 remote cleanup。schema 0017 固定 task／repo ID／branch／nonce／PR；unknown mutation 只對帳、不重送。remote cleanup 受阻仍保存本機 WIP 並釋放 agent，等 operator Retry。原生離線 Forge／daemon 測試、嚴格 base 政策及 production Forge 受控真 GitHub 測試已通過；最終整合驗收與 CI 以 #157 為準，見 [GitHub forge](../../docs/gates/gate-12c-github.md)。

GitHub merge 前要求可讀的 classic branch protection：strict、非空 required checks、enforce_admins 且未要求 linear history；設定不足先受阻，daemon 不代改共享 repo。已 merge 的收據對帳維持只讀。[政策與限制](../../docs/gates/gate-12c-github.md)。

本機 WIP 存檔失敗時，取消／完成當下仍回報原錯誤並保留 binding；背景 wake 可稍後重試。遠端收尾失敗另記 cleanup-remote attention，不吞掉本機存檔錯誤。

第 13B 維護排他：`SqliteStore` 在建立／開啟資料庫前取得 `.agend-maintenance.lock` 的共享 flock，持有到 DB thread 關閉。服務解除安裝以排他 flock 阻止新 store 啟動，並取得既有 DB 的原生 SQLite 鎖以拒絕舊 daemon；不建立缺少的 DB、不執行 migration。鎖檔保留同一 inode，避免其他程序鎖到被替換的檔案。

13C agent 環境覆寫 Claude／OpenCode 更新開關；本機版本匯入使用 `maintenance::Activity` 共享 lease，阻止解除安裝在發布期間刪資料。操作員共用設定不變，見[版本管理](../../docs/architecture/backend-versions.md)。

client protocol 1.7 的 operator `message_delivery` 只讀持久化收據：message ID、sender／target、state、turn ID 與時間，不回 body、不推進狀態。供 canary 核對真正的 confirmed，尚未接完整 canary 升級流程。

13C 施工中的 protocol 1.7：操作員 `send_message` 固定以 `@operator` 真人身分 queue 投遞，必填 UUID v4；`driver_status` 回傳 instance 與就緒狀態，Codex 必須有連線，unknown 不代表 idle。這些 RPC 不切換 backend 版本。

正常 daemon 將執行映像固定至 home/runtime-binaries 的私有副本，holder／hook 與 shim 使用該副本；原始 binary 升級不改變本次啟動路徑。副本保留供存活 holder 使用；重連受管 backend 的啟動身分對帳尚未完成，版本准入仍關閉。

固定 launcher 的快照驗證直接回傳摘要與檔案身分 binding，供 runtime 沿用；重啟時仍重新驗證 running image 與快照內容，但不再第三次雜湊相同快照。既有 holder 跨 daemon 重啟保留（D3／D5）。

13C migration 0018 的 `managed_launches` 保存每個 instance 的啟動意圖：在任何 SpawnBound I/O 前提交，重連只讀回核對；新啟動須由 supervisor 先證明舊 holder 已離開，再以舊 binding 做 CAS。instance 明確移除時 cascade 刪除，重建同名 instance 不能繼承舊紀錄。儲存 API 已實作，supervisor／runtime 串接仍待完成，受管准入維持拒絕。

受管 runtime 提供 `start_reserved`／`attach_reserved`：前者送持久 UUID 的 SpawnBound，再核 holder 回報的 UUID／PID；後者只讀 GetLaunchBinding，絕不補送 Spawn。連線須核對成功才開放輸入，transport 重連也重新核對；不符時保留 holder 並回報失敗。核對前的 Exited 暫存，避免未驗證事件觸發 supervisor 重啟；主動取消不誤報身分失敗。這些 API 仍待接入 supervisor 的受管啟動決策，版本准入保持關閉。

13C supervisor 在受管 canary 核對後，以 canonical 匯入程式建立 launch，確認舊 holder／orphan 已離開才保存 SQLite 意圖並送 SpawnBound。重連只讀原意圖，核對設定、artifact 與 holder UUID；不符保留程序並標記失敗。版本切換／回退仍待完成。

版本切換儲存層以 BackendSwitch 保存原啟動證據、目標與階段；program 與 phase 原子提交／回退，過期請求拒絕。Prepared 可取消並保留原 program 與執行中 agent，已提交的切換須走回滾。canary、停止 holder 及 CLI／supervisor 切換編排仍由後續流程接入。

13C OpenCode runtime 保留尚未結束的舊代 worker，取消旗標與 `workers_stopped` 分開；只有所有執行緒退出才回報停止。這是本機投遞執行緒的證據，backend 回合是否結束仍需原生狀態核對。

13C Prepared 切換記錄會暫停 Claude channel／Stop、Codex 與 OpenCode 的新 push attempt；檢查與 reservation 在同一 DB 工作序列執行。訊息保留 queued，取消後可恢復；既有 attempt 的回執仍可確認。準備前已取得的寫入權仍須排空，此限制不等於 holder 或 backend 已閒置；完整 supervisor 切換編排尚未接入。

Prepared 也暫停 agent 的 inbox 讀取：狀態檢查與內容查詢在同一 DB 工作執行，回覆明確暫停原因。操作員歷史查詢不受影響；取消後沿用原有最後 20 筆／after 游標。已完成讀取但尚未寫回 socket 的回覆仍需由切換編排排空。

Codex 的 `workers_stopped` 追蹤連線建立與已移出 link 表但仍在退出的 worker。disconnect 的有限等待逾時不會讓這項證據消失；呼叫端仍須序列化新 connect，並另查 backend 回合是否結束。

1.8 backend switch 操作由 supervisor 序列處理：prepare 驗受管來源與目標 canary，持久化 Prepared 暫停新投遞；status 查紀錄，cancel 核精確 ID／設定後恢復。這些 RPC 尚不停止 holder、換版或回滾。

取得 DB 鎖後立即啟用 daemon 日誌，記錄 executable／私有 launcher 驗證的開始、結果與耗時；在建立 socket 前卡住也能定位。失敗仍拒絕啟動，不因已有快取略過雜湊。

13C runtime `stop_reserved` 核對目前 holder PID，再在同一條 holder 1.3 連線查啟動 UUID／instance／agent PID 後送 Shutdown；不重連重送，不停止替代或 legacy holder。呼叫者仍須先暫停投遞、確認回合結束並序列化新啟動；拒絕可能斷開 runtime link，但保留程序。這是停止身分契約，尚未接入完整換版編排。

13C server 追蹤 Claude helper 與 inbox 回覆，從 handler 執行前持有到序列化與 bounded socket write 結束。prepare 先持久化投遞暫停，再捕捉既有回覆並最多等 10 秒排空；後續空輪詢不延長這個範圍。逾時保留 Prepared 並要求查 status，不改程式或停止 holder。這只證明本機寫入結束，backend 消費／回合完成仍須另驗。

13C Committed／Restoring 仍暫停新投遞；只有 native readiness 呼叫者提供且 DB 核對未變的 Running instance、PID／session、新 launch 意圖及目標 artifact，`finish_backend_switch` 才記 Activated／RolledBack 並恢復。原啟動 UUID、過期快照與未完成切換覆寫會拒絕；Store 不代替 native readiness，完整 supervisor 編排待接。

13C 正式 TerminalHub 在完整終端 acquire／resize／input 與 legacy input 執行前查持久切換暫停；唯讀 frame／viewport 與 release 保留。操作先加入同一排空範圍再查 DB，所以競爭中的操作不是被拒絕，就是被先前回覆排空捕捉。完整終端追蹤到 holder 控制請求返回；legacy guard 隨實際 blocking write 工作持有。這仍不代表遠端回合結束；失敗／斷線不能當成 native readiness。

Codex `thread_idle` 透過目前連線重讀完整分頁回合，核連線物件、generation 與 instance 快照未變；僅已知終止狀態可判閒置，缺失／異常分頁拒絕。這是當下 thread 觀察，呼叫者仍須先暫停並排空輸入、核受管 holder 身分；尚未接入換版 coordinator。

OpenCode `session_idle` 重讀 REST session 狀態，查詢前後核 holder／instance／session handoff／endpoint／私有憑證；缺失或改變拒絕。不以 daemon 的閒置快取、訊息回執或 HTTP 接受當成回合完成；換版仍須先暫停排空並核 managed launch。

換版 Prepared 也暫停 Claude startup key reservation；reservation 與暫停檢查同 SQLite 交易，既有按鍵操作納入 server 排空追蹤。Committed／Restoring 允許新 launch 完成啟動選單，否則無法驗 readiness。操作逾時仍是未知結果，不能因此推論 backend 已結束。

私有 launcher 在 daemon 準備階段先以清空環境執行 `--version`，最多等待 30 秒，前後核 executable binding。首次執行可能耗在 OS 載入／驗證，不能挪用 holder 的 5 秒 socket 連線期限；失敗停止 daemon 準備，逾時只清理自己的短命 child。

`Server::claude_observer` 提供只讀 session_idle：本次連線的 SessionStart／UserPromptSubmit 與 Stop 候選、5 秒穩定期、live screen 無 hard gate，查詢後再核 session／revision／連線。重連不能沿用舊候選；初始 Ready 仍須完整錄製規則。這是觀察，不代替暫停排空、managed launch 身分及 supervisor 停止授權。

13C：pending backend switch 阻止一般 boot start、自動重啟與 operator retry 改寫啟動意圖。`backend_switch::pending_switch_boot_preserves_launch_reservation_without_ordinary_restart` 經原生 daemon 驗三種 pending phase 跨 boot 保留精確資料；專用換版恢復仍待串接。

13C Codex 閒置觀察在同一 worker 排序於先前 RPC 之後，先確認原生 queue 的 data 為空且 nextCursor 明確為 null，再讀完整 turns；後端佇列非空或欄位缺失不當作閒置。原生 fake app-server 測試包含第二筆排隊訊息、消化後空佇列，以及真 producer 回覆的缺欄位／錯形狀反例。

13C 新增 `backend switch activate／rollback --switch-id`：精確持久 ID、目的版本准入、投遞排空與 native idle 後停止受管 holder；Committed／Restoring 保持暫停，核新 holder 綁定及 readiness 後才釋放。Activated／Committed 回滾先保存 RollbackPrepared；daemon 重啟後由定期協調器繼續。三 backend 原生假版本往返與目的 holder 消失後自動回退已驗；目前代原生 AgentExited 亦可核精確 holder 後回退，Codex 已驗。driver 斷線不當作退出；完整 crash matrix 尚未完成。

受管 holder 的就緒身分查詢走既有 runtime socket；另開查詢連線會取代 holder 的唯一 client，不能用於保持 Ready 觀察的驗證。回覆核 binding／instance／agent PID、holder PID 與原 terminal connection 仍有效；斷線或逾時拒絕完成切換。

OpenCode 版本核對：受管程式使用匯入 artifact 的版本（supervisor 仍須先核成功 canary）；私有 canary daemon 使用精確 CanaryScope；一般未受管程式維持 1.18.34。REST health 必須與 wrapper 回報的版本相同。scope 存在但不匹配時直接拒絕，不退回一般路徑。

換版的 `problem` 保存具體失敗原因與首次等待時間；driver 斷線、啟動失敗或自動回退被拒時，在「需要你」顯示 `backend-switch:<instance>:<switch-id>`，並由 `backend switch status` 顯示原因。daemon 重啟恢復同一通知，pipeline 同步不移除它。通知沒有一般 Retry 動作；操作員先查狀態，仍循正式換版／回退入口。成功啟用、回退、取消或移除 instance 才清除通知。這不代表斷線本身授權停止存活 holder；啟動逾時政策仍待完成。

目的版本提交為 Committed 或 Restoring 時，同一交易保存 300 秒 activation deadline；重啟、重複觀察及問題通知都不重新計時，真正開始還原才建立新的期限。到期且仍未通過 native readiness，下一次協調檢查保存逾時問題、保持投遞暫停與原 holder。到期不等於閒置，不授權強制停止；稍後通過 readiness 仍可完成並清除通知／期限。舊持久紀錄若沒有 deadline，未完成時明確提示缺少期限，不能當作重新獲得五分鐘。

13D 配對驗證已加入純邏輯與 notifier adapter：10 分鐘 nonce、GetMe 身分、直接人類 /start、時間／目的地核對、精確操作員確認後產生 token reference／單一 user allowlist 設定。觀察與確認重查 bot，拒絕中途換 bot；群組 topic 綁 message_thread_id。SQLite schema 20 另保存單一配對收據，候選對象與更新游標同交易發布；過期、舊快照或已關閉操作拒絕。CLI／RPC 已接入，設定套用由操作員 CLI 的 setup apply 完成，也未執行真 Telegram。daemon 不改寫人寫的 config.toml（D8）；設定套用由操作員 CLI 負責。

13D `PairingService` 串行執行 HTTP 與 SQLite 發布；caller 取消不釋放正在執行的操作，後續請求以 Status 查收據。已配置 notifier 時拒絕 Begin／Poll／Confirm，避免兩個 getUpdates consumer；停止介面先關閉准入再等待發布。此服務已接 daemon 啟停及 protocol 1.9 操作員 RPC；停止時關閉准入並等待既有發布，之後才停止 server。

13E 的 `pipeline_probe install` 強制指定 AGEND_BIN，先在全新 HOME 以該 binary init，再共用正式 pipeline fixture 驗首任務及清理。這是已安裝 binary 的 native fake-worker smoke；不啟動真模型或主機服務。

私有 CanaryScope 對 instance args 做精確比對，讓 canary 明確選模型，同時拒絕未記錄的啟動參數。既有省略 args 的 scope 仍只接受空 args。

13C 明確 `--auth-file` 的格式、私有路徑與真測邊界見[canary 認證](../../docs/architecture/backend-canary-auth.md)。本機測試涵蓋 private copy、來源不變、OpenCode 正式 Layout 保留認證、scope 錯配與權限拒絕；完整 native canary 使用測試用憑證，沒有真帳戶或模型呼叫。

Fleet 的 program 取自 instance 設定，保留 driver wrapper 前的程式，供 doctor 診斷；不包含 args 或認證。

backend_versions::registry 提供阻塞唯讀 latest 查詢，daemon 使用時需放在 worker；固定 npm HTTPS、拒轉址、5 秒期限與 256 KiB 上限。沒有認證或下載／執行步驟。每日檢查與通知排程尚未接入。
