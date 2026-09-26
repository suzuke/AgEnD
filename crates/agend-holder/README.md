# agend-holder

> **TL;DR**
> - 每個 instance 一個 holder 程序（`agend holder <instance-id>`）：在 PTY 裡跑 agent、記住畫面、回報結束狀態，活過 daemon 重啟（D3）。
> - 記住：**PTY 只收三種位元組**：列舉過的控制鍵、操作者輸入、終端查詢回覆；只有協定 `Shutdown` 停得掉 holder。
> - 下一步：第 6 施工關起由 daemon 的 agent runtime（`agend_daemon::runtime`）啟動與接回 holder；daemon 不依賴本 crate，只經 holder 協定與 `agend holder` 子命令。

## 負責

- 以 portable-pty 啟動 agent；環境**只有** `Spawn.env`（沒給 `TERM` 時補 `xterm-256color`）
- 以 alacritty_terminal 維護畫面（50 列 × 200 欄，scrollback 1,000 列只在記憶體），提供純文字快照與輸出串流
- 回報 exit code 或 signal；agent 結束後保留，每次連上都在快照後送 `Exited`
- holder 協定 server：版本協商、同時一條連線、新的接手、落後 1 MiB 斷線
- 脫離終端（`setsid`）、忽略 HUP／INT／QUIT／TERM、`run/holders/<id>.lock` 防重複
- 安全網：`AGEND_HOME` 被刪，或沒有 agent 在跑且連續 24 小時沒人連上 → 自行停止

## 不負責

- 流水線或送達邏輯；訊息內容不在 PTY 打字
- 開 DB
- 螢幕分類（core 的 `screen`）
- 自己重啟 agent（一個 holder 一生只跑一個 agent；第二個 `Spawn` 回 `already_spawned`）
- 附屬程序（codex app-server、opencode serve）：移到第 7 施工關

## 程序與檔案

| 項目 | 內容 |
|---|---|
| 指令 | `agend holder <instance-id>`，需要絕對路徑的 `AGEND_HOME`；instance id 只能用 `A-Z a-z 0-9 _ -`、最多 32 字 |
| 檔案 | `$AGEND_HOME/run/holders/`（0700）下的 `<id>.sock`、`<id>.lock`（內容是 holder pid）、`<id>.log` |
| exit code | 0：停止（`Shutdown` 或安全網）；1：已有 holder（印 `holder for <id> already running (pid N)`）；2：用法或設定錯誤（例如 socket 路徑超過 100 bytes） |
| 在跑嗎 | 只看 lock：`paths::is_running`，只回活著、大於 1 的 pid（絕不回 0）。不連 socket，因為新連線會搶走 daemon 的連線 |
| 上限 | `hello` 10 秒內送完；一行請求最多 1 MiB（`request_too_large`）；`Resize` 1–1000（`invalid_size`） |
| 測試用環境變數 | `AGEND_HOLDER_IDLE_EXIT_SECS`：把 24 小時安全網改短 |

## 模組

| 模組 | 職責 |
|---|---|
| `lib`（`run`） | 程序啟動：檢查、lock、`setsid`、訊號、stdio 導到 log、bind socket |
| `paths` | run 目錄路徑、instance id 規則、lock 判斷存活 |
| `server` | holder 協定 server、停止流程、安全網 |
| `pty` | spawn agent、控制鍵位元組、唯一的 PTY 寫入 thread（佇列 64） |
| `screen` | alacritty 畫面、純文字快照、終端查詢回覆 |
| `exit` | `waitpid` 與 signal 名稱 |
| `client` | 同步的協定 client（探測 example 與測試用） |
| `sidecar` | 只有說明：不做附屬程序協定（第 7 施工關 P2 推翻第 4 施工關 P8 的 `SpawnSidecar`）；codex app-server 由 PTY 裡的 `sh` 包裝在背景起，跟 agent 同一個 process group |

## Shutdown 做什麼

1. agent 還在：對 agent 的 process group（＝ agent pid，一定大於 1）送 SIGHUP。
2. 最多等 5 秒，再對同一個 group 送 SIGKILL。agent 早就結束的，直接對 group 送 SIGKILL 清掉留下的子程序：結束的 agent 一直保留成 zombie 直到這裡才回收，所以 group id 不會被別的程序重用。
3. 刪 socket、放掉 lock、exit 0。

## 依賴規則

- 一般依賴：`agend-core`、`portable-pty`、`alacritty_terminal`（關掉預設的 `serde`）、`parking_lot`、`base64`、`serde_json`、`libc`
- `cargo xtask check-deps`：不能依賴 async runtime、SQLite、`agend-daemon`
- alacritty_terminal 的 `event_loop`（連帶 `polling`）無法用 feature 關掉；holder 不用它

## 入口

- `agend_holder::run`（`agend holder` 子命令）
- `agend_holder::server::serve`、`agend_holder::client::HolderClient`、`agend_holder::paths::is_running`
- 探測工具：`cargo run -p agend-holder --example holder_probe -- <start|snapshot|key|shutdown|demo> …`

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-holder
~/.cargo/bin/cargo xtask accept holder
```
