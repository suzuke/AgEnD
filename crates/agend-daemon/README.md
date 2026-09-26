# agend-daemon

> **TL;DR**
> - 唯一的大型 I/O 層：常駐、單一 tokio runtime、DB 專屬執行緒；`agend daemon` 起 holder、接回 holder、agent 死了用 `--resume` 接回。
> - 記住：**daemon 停掉時 holder 與 agent 照跑（D3）**；同一個 `AGEND_HOME` 只有一個 daemon（`agend.db` 的鎖）；`agend.db` 只有 daemon 開（`store`）。
> - 下一步：第 6 施工關（daemon ↔ holder）draft PR；還原 DB 快照的步驟見下方「store」。

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
| `server` | protocol v1 server |
| `handlers` | 命令處理，與傳輸分離；依身分限權 |
| `ingest` | hook 與結構化事件接收、磁碟佇列補送 |
| `pipeline` | 執行 core 狀態機、只由 daemon merge |
| `delivery` | 送達模型 |
| `supervisor` | 讓 DB 裡的 instance 保持在跑：死了等 5 秒 `--resume`、10 分鐘 3 次仍死就 `failed`；之後：卡住、額度、轉派、例外才找人 |
| `scheduler` | timeout、cron |
| `reconcile` | 開機與每日 DB ↔ git 對帳 |
| `driver::{codex,claude,opencode}` | backend 結構化 API；`codex::socket_connect_path` |
| `runtime` | `HolderRuntime`：起 holder、holder 協定 client、每個 holder 一條長連線、agent 環境白名單、shim symlink |
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
| session | claude：`new` 時 `--session-id <id>`，`running` 後只用 `--resume <id>`；codex、opencode 還沒有 session id（第 7、12 施工關），死了就 `failed` |
| 死掉之後 | agent 結束、holder 死了（連線斷＋鎖放掉）、啟動失敗 → log → 5 秒後 `restart N/3 --resume <id>`（還是 `new` 就 `--session-id`）；10 分鐘內 3 次仍死 → `<id> failed: …`，不再起、關掉對 holder 的連線；次數只在記憶體 |
| Ctrl-C | SIGINT／SIGTERM：關 holder 連線、關 DB、exit 0；**不送 `Shutdown`**，holder 照跑 |
| log | stderr ＋ `logs/daemon-YYYY-MM-DD.log`（UTC，0600），留 7 天；`audit/shim.jsonl` 每天輪替成 `shim-YYYY-MM-DD.jsonl`、留 14 天；`run/holders/<id>.log` 在 holder 不在、7 天沒動時刪 |

手動加減 instance（daemon 停著時）：`cargo run -q -p agend-daemon --example daemon_probe -- add g6-1`、`remove g6-1`、`list`。

## store（第 5 施工關）

| 項目 | 內容 |
|---|---|
| 檔案 | `$AGEND_HOME/agend.db`（建立時 0600；home、home 不存在的上層目錄、`backups/` 建立時 0700，已存在的目錄不改）；home 由呼叫端傳入 |
| 建立 | 只有 `agend.db` 不存在時才建新 DB：先在 `.agend.db.new` 建好、所有 migration commit 後才 hard link（檔案系統不支援 hard link 時改 rename）成 `agend.db`；上次建到一半留下的 `.agend.db.new` 刪掉重建。`agend.db` 比 SQLite 檔頭（100 bytes）短、schema 版本 0、或缺它那個版本的表 → 拒絕開啟、檔案不動：`agend.db exists but is empty (0 bytes); refusing to start with an empty database — restore a snapshot from <home>/backups (see README)`，照下方步驟還原；指向不存在檔案的 symlink 或不是一般檔案 → `refusing to use <path>: …`。**刪掉 `agend.db` 等於從空 DB 重新開始**；空 DB 不做每日快照，但寫進第一個 task 後每天的快照照常輪替、一天擠掉一份舊的好快照：要還原請在那之前照下方步驟做 |
| 執行緒 | 一條 `agend-db` 執行緒持有唯一連線；async 方法經 channel（256）送 closure；該執行緒 panic 後每個呼叫回 `store thread stopped` |
| 同時開 | `locking_mode=EXCLUSIVE`，第二個程序：`agend.db is in use by another process (is another agend daemon running?)` |
| 表 | `tasks`、`workflows`、`task_events`、`instances`（STRICT）；schema 版本在 `PRAGMA user_version`（目前 2），migration 在 `src/store/migrations/` |
| 耐久 | WAL、`synchronous=FULL`、`foreign_keys=ON` |
| 保留期限 | `store::retention::RETENTION`：task、workflow、instance 永久；事件 14 天；`audit/shim.jsonl` 每日輪替留 14 天、daemon log 7 天、holder log 7 天（第 6 施工關 `housekeeping`） |
| DB 快照 | `backups/agend-YYYY-MM-DD.db`（UTC；DB 沒有任何 task、事件與 instance 時不做），升級前 `agend-YYYY-MM-DD-pre-vN.db`；只留最新 7 份，其他檔案不動 |
| 還沒存 | `PipelineState`（第 10 施工關：task 列上一欄、與 task 一起 CAS，不重播事件） |

### 還原 DB 快照（手動）

1. 停 daemon（`agend.db` 有 daemon 開著時複製會被它的下一次寫入蓋掉）。
2. `cp "$AGEND_HOME/backups/agend-YYYY-MM-DD.db" "$AGEND_HOME/agend.db"`
3. `rm -f "$AGEND_HOME/agend.db-wal"`（舊的 WAL 屬於被蓋掉的 DB）
4. 啟動 daemon。快照比 binary 新（`schema version N is newer…`）時，改用 `-pre-vN` 那份或裝回較新的 agend。

只想查資料、不還原：`sqlite3 -readonly "$AGEND_HOME/backups/agend-YYYY-MM-DD.db"`（daemon 跑著時 `agend.db` 本身打不開）。

## 依賴規則

- 一般依賴：`agend-core`、`libc`（holder 鎖檔的 `flock`）、`rusqlite`（`bundled`）、`tokio`（`rt-multi-thread`、`signal`、`sync`、`time`）、`serde_json`、`toml`；`agend-tui`、`agend-shim`、`agend-client` 不能依賴 SQLite 或本 crate；**本 crate 不能依賴 `agend-holder`、`agend-shim`**（只經 holder 協定與子命令，第 6 施工關 P9）（`cargo xtask check-deps`）
- dev 依賴：`agend-testkit`
- 領域模組只透過 `agend_core::traits` 呼叫 adapter

## 入口

- `agend_daemon::driver::codex::socket_connect_path`
- `agend_daemon::store::SqliteStore::open(home, now_unix_ms)`
- `agend_daemon::daemon::run`（`agend daemon`）
- `agend_daemon::runtime::HolderRuntime`（`Runtime` trait）、`runtime::shutdown_holder`

## 下一步

```bash
cargo test -p agend-daemon
cargo xtask accept daemon-holder   # 第 6 施工關 demo：daemon_probe demo
```
