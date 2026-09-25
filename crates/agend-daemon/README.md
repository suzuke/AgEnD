# agend-daemon

> **TL;DR**
> - 唯一的大型 I/O 層：常駐、單一 tokio runtime、DB 專屬執行緒。
> - 記住：**agent 與附屬程序不是 daemon 的子程序**；daemon 可隨時重啟；`agend.db` 只有 daemon 開（`store`）。
> - 下一步：第 5 施工關（store）draft PR；還原 DB 快照的步驟見下方「store」。

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
| `server` | protocol v1 server |
| `handlers` | 命令處理，與傳輸分離；依身分限權 |
| `ingest` | hook 與結構化事件接收、磁碟佇列補送 |
| `pipeline` | 執行 core 狀態機、只由 daemon merge |
| `delivery` | 送達模型 |
| `supervisor` | 卡住、額度、轉派、例外才找人 |
| `scheduler` | timeout、cron |
| `reconcile` | 開機與每日 DB ↔ git 對帳 |
| `driver::{codex,claude,opencode}` | backend 結構化 API；`codex::socket_connect_path` |
| `runtime` | holder 協定 client |
| `forge::{local,github}` | 提交與 merge |
| `git` | 建立／移除 worktree 與 branch（先記錄再建立） |
| `runner` | 在 head 的臨時 worktree 跑 `command` 關卡 |
| `store` | SQLite，唯一持久狀態：`SqliteStore`（`Store` trait + `save_workflow`、`load_events`、`prune`、`snapshot`） |
| `notifier` | Telegram topic |

## store（第 5 施工關）

| 項目 | 內容 |
|---|---|
| 檔案 | `$AGEND_HOME/agend.db`（建立時 0600；home、home 不存在的上層目錄、`backups/` 建立時 0700，已存在的目錄不改）；home 由呼叫端傳入 |
| 建立 | 只有 `agend.db` 不存在時才建新 DB：先在 `.agend.db.new` 建好、所有 migration commit 後才 hard link（檔案系統不支援 hard link 時改 rename）成 `agend.db`；上次建到一半留下的 `.agend.db.new` 刪掉重建。`agend.db` 比 SQLite 檔頭（100 bytes）短、schema 版本 0、或缺它那個版本的表 → 拒絕開啟、檔案不動：`agend.db exists but is empty (0 bytes); refusing to start with an empty database — restore a snapshot from <home>/backups (see README)`，照下方步驟還原；指向不存在檔案的 symlink 或不是一般檔案 → `refusing to use <path>: …`。**刪掉 `agend.db` 等於從空 DB 重新開始**；空 DB 不做每日快照，但寫進第一個 task 後每天的快照照常輪替、一天擠掉一份舊的好快照：要還原請在那之前照下方步驟做 |
| 執行緒 | 一條 `agend-db` 執行緒持有唯一連線；async 方法經 channel（256）送 closure；該執行緒 panic 後每個呼叫回 `store thread stopped` |
| 同時開 | `locking_mode=EXCLUSIVE`，第二個程序：`agend.db is in use by another process (is another agend daemon running?)` |
| 表 | `tasks`、`workflows`、`task_events`（STRICT）；schema 版本在 `PRAGMA user_version`，migration 在 `src/store/migrations/` |
| 耐久 | WAL、`synchronous=FULL`、`foreign_keys=ON` |
| 保留期限 | `store::retention::RETENTION`：task、workflow 永久；事件 14 天；`audit/shim.jsonl` 每日輪替留 14 天（第 6 施工關實作） |
| DB 快照 | `backups/agend-YYYY-MM-DD.db`（UTC；DB 沒有任何 task 與事件時不做），升級前 `agend-YYYY-MM-DD-pre-vN.db`；只留最新 7 份，其他檔案不動 |
| 還沒存 | `PipelineState`（第 10 施工關：task 列上一欄、與 task 一起 CAS，不重播事件） |

### 還原 DB 快照（手動）

1. 停 daemon（`agend.db` 有 daemon 開著時複製會被它的下一次寫入蓋掉）。
2. `cp "$AGEND_HOME/backups/agend-YYYY-MM-DD.db" "$AGEND_HOME/agend.db"`
3. `rm -f "$AGEND_HOME/agend.db-wal"`（舊的 WAL 屬於被蓋掉的 DB）
4. 啟動 daemon。快照比 binary 新（`schema version N is newer…`）時，改用 `-pre-vN` 那份或裝回較新的 agend。

只想查資料、不還原：`sqlite3 -readonly "$AGEND_HOME/backups/agend-YYYY-MM-DD.db"`（daemon 跑著時 `agend.db` 本身打不開）。

## 依賴規則

- 一般依賴：`agend-core`、`rusqlite`（`bundled`）、`tokio`（只開 `sync`）、`serde_json`、`toml`；`agend-tui`、`agend-shim`、`agend-client` 不能依賴 SQLite 或本 crate（`cargo xtask check-deps`）
- dev 依賴：`agend-testkit`
- 領域模組只透過 `agend_core::traits` 呼叫 adapter

## 入口

- `agend_daemon::driver::codex::socket_connect_path`
- `agend_daemon::store::SqliteStore::open(home, now_unix_ms)`
- daemon 入口之後由 `agend` binary 的子命令啟動

## 下一步

```bash
cargo test -p agend-daemon
cargo run -p agend-daemon --example store_demo   # 或 cargo xtask accept store
```
