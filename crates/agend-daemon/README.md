# agend-daemon

> **TL;DR**
> - 唯一的大型 I/O 層：常駐、單一 tokio runtime、DB 專屬執行緒。
> - 記住：**agent 與附屬程序不是 daemon 的子程序**；daemon 可隨時重啟。
> - 下一步：第 5 施工關（store）開始實作。

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
| `store` | SQLite，唯一持久狀態 |
| `notifier` | Telegram topic |

## 依賴規則

- 一般依賴：`agend-core`（之後會加 tokio、rusqlite 等，只能加在這裡）
- dev 依賴：`agend-testkit`
- 領域模組只透過 `agend_core::traits` 呼叫 adapter

## 入口

- `agend_daemon::driver::codex::socket_connect_path`（目前唯一的公開函式）
- daemon 入口之後由 `agend` binary 的子命令啟動

## 下一步

```bash
cargo test -p agend-daemon
```
