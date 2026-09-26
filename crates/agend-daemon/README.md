# agend-daemon

> **TL;DR**
> - 唯一的大型 I/O 層：常駐、單一 tokio runtime、DB 專屬執行緒；`agend daemon` 起 holder、接回 holder、agent 死了用 `--resume` 接回；開機計畫做完才開 `run/daemon.sock` 講 client protocol 1.1（第 8 施工關）；codex 經 app-server 的 JSON-RPC 送達、自己建 thread、死了照樣 resume（第 7 施工關）。
> - 記住：**daemon 停掉時 holder 與 agent 照跑（D3）**；同一個 `AGEND_HOME` 只有一個 daemon（`agend.db` 的鎖）；`agend.db` 只有 daemon 開（`store`）；socket 連得上＝daemon 好了。
> - 下一步：第 7 施工關（codex）draft PR：`cargo xtask accept codex`；還原 DB 快照的步驟見下方「store」。

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
| `daemon` | `agend daemon`：前景跑、開 DB（重試 10 秒）、開機順序、訊號（第 6 施工關） |
| `boot` | `plan_boot`：開機時接回／啟動／孤兒，純函式 |
| `housekeeping` | 開機與每小時：`prune`、DB 快照、daemon log、audit、holder log 的期限 |
| `log` | daemon log：stderr ＋ `logs/daemon-YYYY-MM-DD.log` |
| `server` | client protocol server：`run/daemon.sock`、JSON Lines、`hello`、寫入 5 秒逾時、落後就斷（第 8 施工關） |
| `handlers` | 請求處理，與傳輸分離；依身分限權（`resolve_attention` 只收操作者） |
| `fleet` | 全貌（instance、task、「需要你」）與事件記錄（最近 1024 筆、broadcast），同一把鎖（第 8 施工關） |
| `ingest` | hook 與結構化事件接收、磁碟佇列補送 |
| `pipeline` | 執行 core 狀態機、只由 daemon merge |
| `delivery` | 送達模型：四個狀態各代表什麼（第 7 施工關 P5）、`render`（`From:`／`Task:` 標頭＋完整 body） |
| `supervisor` | 讓 DB 裡的 instance 保持在跑：死了等 5 秒 `--resume`、10 分鐘 3 次仍死就 `failed`（變成「需要你」項目，操作者可 `retry`）；之後：卡住、額度、轉派、例外才找人 |
| `scheduler` | timeout、cron |
| `reconcile` | 開機與每日 DB ↔ git 對帳 |
| `driver::{codex,claude,opencode}` | backend 結構化 API；codex 見下方「codex（第 7 施工關）」；claude、opencode 還只有說明（第 12 施工關） |
| `runtime` | `HolderRuntime`：起 holder、holder 協定 client、每個 holder 一條長連線（轉出 `PtyBytes` 給終端訂閱者）、agent 環境白名單、shim symlink |
| `forge::{local,github}` | 提交與 merge |
| `git` | 建立／移除 worktree 與 branch（先記錄再建立） |
| `runner` | 在 head 的臨時 worktree 跑 `command` 關卡 |
| `store` | SQLite，唯一持久狀態：`SqliteStore`（`Store` trait + `save_workflow`、`load_events`、`prune`、`snapshot`、instance 的增刪查） |
| `notifier` | Telegram topic |

## daemon（第 6 施工關）

| 項目 | 內容 |
|---|---|
| 啟動 | `agend daemon`，只在前景跑；沒設 `AGEND_HOME` → `AGEND_HOME is not set`、exit 1；有參數 → exit 2 |
| 只有一個 | `agend.db` 被別的程序開著就每 200 ms 重試，10 秒後 `agend daemon: agend.db is in use by another process (is another agend daemon running?)`、exit 1；拿到 DB 前不寫 `logs/` |
| 開機順序 | 開 DB → housekeeping（失敗只記錯）→ `bin/` 的 shim symlink（git、kill、killall、pkill → 目前的 binary）→ `plan_boot` → `agend daemon ready: instances=N recovered=R started=S orphans=O` |
| `plan_boot` | DB 有、鎖被持有 → 接回（重送 `Spawn`，holder 回 `already_spawned`）；DB 有、沒鎖 → 啟動；DB 沒有、鎖被持有 → 孤兒，送 `Shutdown`；`failed` 的不動 |
| instance | `instances` 表（migration 0002，永久保留）：id `[a-z0-9-]{1,24}`、backend、program、args、working_directory、session_id、status（`new`／`running`／`failed`） |
| 啟動 holder | `agend holder <id>`，環境只有 `AGEND_HOME`；每 50 ms 試連、5 秒內連不上算失敗；第一次 `Spawn` 被 holder 確認後才把 `new` 改成 `running`（session 存在，之後都 resume）；自己起的 holder 由一條 thread `wait` 收屍 |
| agent 環境 | 白名單：`AGEND_HOME`、`AGEND_INSTANCE`、`PATH`（`$AGEND_HOME/bin` 開頭）、`HOME`、`USER`、`LOGNAME`、`LANG`、`LC_ALL`、`LC_CTYPE`、`TMPDIR`、`TZ`；其他（例如 `TELEGRAM_BOT_TOKEN`、`AGEND_SHIM_BYPASS`）一律不給 |
| session | claude：`new` 時 `--session-id <id>`，`running` 後只用 `--resume <id>`；codex：daemon 建的 thread，TUI 一律 `resume <thread>`（第 7 施工關）；opencode 還沒有 session id（第 12 施工關），死了就 `failed` |
| 死掉之後 | agent 結束、holder 死了（連線斷＋鎖放掉）、啟動失敗 → log → 5 秒後 `restart N/3 --resume <id>`（還是 `new` 就 `--session-id`）；10 分鐘內 3 次仍死 → `<id> failed: …`，不再起、關掉對 holder 的連線；次數只在記憶體 |
| Ctrl-C | SIGINT／SIGTERM：關 holder 連線、關 DB、exit 0；**不送 `Shutdown`**，holder 照跑 |
| log | stderr ＋ `logs/daemon-YYYY-MM-DD.log`（UTC，0600），留 7 天；`audit/shim.jsonl` 每天輪替成 `shim-YYYY-MM-DD.jsonl`、留 14 天；`run/holders/<id>.log` 在 holder 不在、7 天沒動時刪 |

手動加減 instance（daemon 停著時）：`cargo run -q -p agend-daemon --example daemon_probe -- add g6-1`、`remove g6-1`、`list`。

## codex（第 7 施工關）

| 項目 | 內容 |
|---|---|
| 程序 | holder 的 PTY 子程序是固定的 `sh` 包裝（`driver::codex::launch::WRAPPER`，`$0`＝`agend-codex`）：背景起 `codex -c … app-server --listen unix://$AGEND_HOME/run/holders/<id>.codex.sock <instance 的 args>`（輸出接到 holder log），等交接檔 `run/holders/<id>.codex-go`（最多 60 秒，否則 exit 1），再 `exec codex -c … resume <thread> --remote unix://<解析後的 socket>`；兩個程序同一個 process group。holder 協定不變 |
| 設定 | 每次啟動的 `-c`：trust `projects={"<realpath 工作目錄>"={trust_level="trusted"}}`、`check_for_update_on_startup=false`；app-server 另有 `approval_policy="never"`、`sandbox_mode="danger-full-access"`（`thread/start` 也帶）。不設 `CODEX_HOME`，**不寫 `~/.codex`** |
| shim | codex agent 的環境多一個 `ZDOTDIR=$AGEND_HOME/zsh`；開機時寫 `zsh/.zprofile`，在 `/etc/zprofile`（`path_helper`）之後把 `$AGEND_HOME/bin` 放回最前面 |
| 啟動 | `Spawn` 前刪舊的 socket（連它指到的 `/private/tmp/codex-daemon-<uid>/` 檔）與 `$GO` → holder → 背景 `connect`：每 100 ms 試連、`initialize`，20 秒放棄；沒有 thread 就 `thread/start`、**先存進 `instances.session_id`**，有就 `thread/resume {excludeTurns:true}`（找不到而且從沒送過訊息 → 建新的，否則 `failed`）；各限 30 秒 → 寫 `$GO`（暫存檔再 rename）→ 對帳、送出 `queued` 的訊息 → 開長連線。失敗＝一次死亡（5 秒／3 次／`failed`） |
| log | `<id>: app-server ready (… ms)`、`thread <T> created`／`thread <T> resumed (idle)`（或 `busy`）、`go (resume <T>)`、`<m> (<level>) → <方法> → sent (turn …)`、`<m> confirmed (turn …)`、`approval declined (gate 7 has no handler): …`、`app-server is gone (no connection for 20 s)`、`sweep of agent group <G> (…): already gone`（或 `SIGKILL sent (…)`） |
| 長連線 | 每個 instance 一條 std thread：`thread/status/changed` → 忙／閒（不去抖動）；user message 的 `item/completed` → `confirmed`；授權請求一律回 `decline`；斷線每 100 ms 重連 20 秒（重連後 resume、對帳、補送），不行就是 app-server 死了 → 下一次重起先 `Shutdown` holder |
| 清掃 | 發現 holder 死了（決定重起或 `failed` 之前）、起新 holder 之前、開機時 holder 已不在的 `failed`：`agent_pid` 有值才做；1 < pgid ≤ `i32::MAX`、group 還有程序、而且有程序的 argv 有一個元素完全等於 socket 路徑（或 `unix://` 加上它）或 `resume` 後面等於 thread id，才對 group 送一次 SIGKILL；之後清掉 `agent_pid` |
| 送達 | `CodexDriver::deliver(instance, message, level)`：`messages` 表是唯一的冪等（同 id 同內容 → 目前狀態、不呼叫 codex；同 id 不同內容 → `invalid_request`）；閒置一律 `turn/start`；忙碌時 `Queue` → `thread/queue/add`（回覆時已閒置 → `thread/queue/start` 一次）、`Steer` → `turn/steer`（`-32600` → `turn/start`）、`Interrupt` → `turn/interrupt`、等 5 秒、`turn/start`。沒有連線、或 backend 還沒有 driver → 停在 `queued`；instance `failed` → `failed` |
| 事件 | `CodexDriver::events(instance, cursor)`：`thread/turns/list` 展開（每個 turn：`BusyChanged{true}`、我們的 user message 各一個 `MessageConfirmed`、結束時 `TurnCompleted{狀態}`、`BusyChanged{false}`）；cursor＝`<turn id>:<slot>` |
| `failed` 的 codex | `retry` 照常（resume 同一個 thread；沒有 thread 就建）；migration 0004 標了 `legacy_no_thread` 的（第 6 施工關留下、有對話沒 thread）不能 `retry`，holder 不動 |

## client protocol server（第 8 施工關）

| 項目 | 內容 |
|---|---|
| socket | `$AGEND_HOME/run/daemon.sock`：`run/` 0700、socket 0600；路徑超過 100 bytes 開機就拒絕（`socket path too long: … (… bytes, max 100 bytes); use a shorter AGEND_HOME`，exit 1，不碰 `agend.db`）；拿到 DB 鎖後刪掉舊的 socket 檔 |
| 何時出現 | 開機計畫做完才 bind，接著印 `listening on …` 與 `agend daemon ready: …`；連得上的 client 一定看到完整的 instance 清單（不需要 `.ready`） |
| 停止 | Ctrl-C／SIGTERM：停止接受、刪 socket 檔、關所有 client 連線，再照第 6 施工關結束 |
| 身分 | `hello` 的 `caller`（CLI 在 agent 裡填 `AGEND_INSTANCE`）：有填＝agent，沒填＝操作者；不做 cookie |
| 請求 | `hello`、`get_fleet`、`subscribe_events`、`subscribe_terminal`、`resolve_attention`（只收操作者，先查身分再找 id；handler 當場拿掉項目並回覆，`retry` 交給 supervisor 之後做）；`terminal_input`、agent 命令 → `not_supported`；`answer_ask` → `unknown_ask`；未知請求 → `unknown_request`、連線不斷 |
| 事件 id | 第一個＝開機時間（unix ms）× 1000 + 1；只放記憶體最近 1024 筆；游標規則見 `agend_core::protocol::client` |
| 慢 client | 落後超過 1024 筆 → `event_gap` 後關連線；寫入 5 秒沒進度 → 關連線；都記一行 log（`client #N (…): …`） |
| instance 狀態 | `starting`（啟動中、等重起）、`unknown`（在跑；忙碌／閒置要 driver）、`failed` |
| 需要你 | `failed` 的 instance → `instance-failed:<id>`（等待時間＝這個 daemon 第一次看到它 `failed`）；`retry`：先 `Shutdown` 留著的 holder，session 建立過就 `running` + `--resume`（claude），沒建立過就 `new`（claude `--session-id`、codex／opencode 全新啟動）；codex／opencode 建立過 session 的沒有操作 |
| 終端 | 在跑的 instance：先回 holder 當下畫面、再轉送之後的 `terminal_bytes`（經 daemon 的長連線，client 不直接連 holder）；`failed` 且 holder 還在：短連一次、只回最後畫面；其他 → `no_terminal` |

## store（第 5 施工關）

| 項目 | 內容 |
|---|---|
| 檔案 | `$AGEND_HOME/agend.db`（建立時 0600；home、home 不存在的上層目錄、`backups/` 建立時 0700，已存在的目錄不改）；home 由呼叫端傳入 |
| 建立 | 只有 `agend.db` 不存在時才建新 DB：先在 `.agend.db.new` 建好、所有 migration commit 後才 hard link（檔案系統不支援 hard link 時改 rename）成 `agend.db`；上次建到一半留下的 `.agend.db.new` 刪掉重建。`agend.db` 比 SQLite 檔頭（100 bytes）短、schema 版本 0、或缺它那個版本的表 → 拒絕開啟、檔案不動：`agend.db exists but is empty (0 bytes); refusing to start with an empty database — restore a snapshot from <home>/backups (see README)`，照下方步驟還原；指向不存在檔案的 symlink 或不是一般檔案 → `refusing to use <path>: …`。**刪掉 `agend.db` 等於從空 DB 重新開始**；空 DB 不做每日快照，但寫進第一個 task 後每天的快照照常輪替、一天擠掉一份舊的好快照：要還原請在那之前照下方步驟做 |
| 執行緒 | 一條 `agend-db` 執行緒持有唯一連線；async 方法經 channel（256）送 closure；該執行緒 panic 後每個呼叫回 `store thread stopped` |
| 同時開 | `locking_mode=EXCLUSIVE`，第二個程序：`agend.db is in use by another process (is another agend daemon running?)` |
| 表 | `tasks`、`workflows`、`task_events`、`instances`、`messages`（STRICT）；schema 版本在 `PRAGMA user_version`（目前 4），migration 在 `src/store/migrations/`；`0003` 在 `instances` 加 `session_started`（0／1，第一次 `Spawn` 被確認、寫 `running` 的同一個 statement 設 1；既有的 `running` 與 `failed` 的 codex／opencode 設 1）；`0004`（第 7 施工關）加 `messages`（`seq INTEGER PRIMARY KEY`、`id` UNIQUE、`from_instance`、`to_instance`、`task_id`、`body`、`level`、`state`、`turn_id`、時間）與 `instances.agent_pid`、`instances.legacy_no_thread`（那一刻 `codex`、沒有 thread、`running`／`failed` 而且 `session_started = 1` 的列設 1 並標 `failed`） |
| 耐久 | WAL、`synchronous=FULL`、`foreign_keys=ON` |
| 保留期限 | `store::retention::RETENTION`：task、workflow、instance 永久；事件 14 天；訊息 30 天（`created_at_unix_ms`）；`audit/shim.jsonl` 每日輪替留 14 天、daemon log 7 天、holder log 7 天（第 6 施工關 `housekeeping`） |
| DB 快照 | `backups/agend-YYYY-MM-DD.db`（UTC；DB 沒有任何 task、事件與 instance 時不做），升級前 `agend-YYYY-MM-DD-pre-vN.db`；只留最新 7 份，其他檔案不動 |
| 還沒存 | `PipelineState`（第 10 施工關：task 列上一欄、與 task 一起 CAS，不重播事件） |

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
- `agend_daemon::daemon::run`（`agend daemon`）
- `agend_daemon::runtime::HolderRuntime`（`Runtime` trait）、`runtime::shutdown_holder`
- `agend_daemon::server::{bind, Server}`、`handlers::Context`、`fleet::Fleet`（測試在同一個程序裡跑 daemon 的 server）

## 下一步

```bash
cargo test -p agend-daemon
cargo xtask accept daemon-holder   # 第 6 施工關 demo：daemon_probe demo
cargo xtask accept client          # 第 8 施工關 demo：client_demo
cargo xtask accept codex           # 第 7 施工關 demo：codex_demo
```
