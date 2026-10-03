# agend-holder

> **TL;DR**
> - 每個 instance 一個 holder 程序（`agend holder <instance-id>`）：在 PTY 裡跑 agent、記住畫面、回報結束狀態，活過 daemon 重啟（D3）。
> - 記住：**PTY 只收三種位元組**：列舉過的控制鍵、操作者輸入、終端查詢回覆；只有協定 `Shutdown` 停得掉 holder。
> - 下一步：第 6 施工關起由 daemon 的 agent runtime（`agend_daemon::runtime`）啟動與接回 holder；daemon 不依賴本 crate，只經 holder 協定與 `agend holder` 子命令。

## 第 11 施工關 C 段（實作中）

完整終端畫面、resize、滑鼠／貼上與歷史的 [P1–P6 提案](../../docs/gates/gate-11c-proposal.md) 已確認（D39），提案 #144 已合併。holder 1.1 已提供結構化 frame、request id、generation／revision 及只讀歷史 viewport；holder 的 `TerminalControl` 已用同一 FIFO 佇列完成實際 resize／input 回覆及交接；daemon／client 與 TUI App 已接通；fake U17 與真 Codex 0.159.3 首次 U17 已核實，後者限四個已核准模型回合。C 段驗收收尾與 merge 仍待確認，見 [驗收收尾](../../docs/gates/gate-11c-closeout.md)。

`GetTerminalFrame` 依同一 parser 的 grid／palette／mode 共用 50 ms 取樣，再取每個 view 的所需列；輸出解析與 classifier 仍讀 live grid。無效 viewport 先拒絕，resize 使取樣失效，control ack 用當下完整畫面。viewport 不改 classifier 的 live screen。normal screen 以絕對 row id 保留 1,000 列歷史；小 viewport 也能固定目前 live grid 內的列，最後一列不得超出 grid；淘汰時回覆 clamped，alternate screen 沒有歷史。resize／reflow 會重新編排 row id，舊 viewport 明確 clamped。單欄仍使用實際一欄尺寸；寬字放不下時顯示帶原樣式的空白，避免上游 reflow hang／spacer 越界，放大後的新寬字正常顯示。序列化 frame（含換行）最多 8 MiB，超限整份拒絕；原 1 MiB 請求上限不變。

`TerminalControl` 帶 request id／generation，提供 Acquire、Resize、Input、Release。Acquire／Resize 成功須附實際 PTY 尺寸的完整 frame；Input 完成實際 write／flush 才回覆；原生 PTY 為 nonblocking，整次寫入最多 5 秒，失敗明示可能已寫部分資料、不重送。每個操作在佇列執行時重新驗 owner；新 grant 等舊在途寫入完成，Release／重連保留尺寸。控制者存在時 legacy operator input／resize 被拒絕，舊的 queued input 也不能繞過新 grant。

## 負責

- 以 portable-pty 啟動 agent；環境**只有** `Spawn.env`（沒給 `TERM` 時補 `xterm-256color`）
- 以 alacritty_terminal 維護畫面（50 列 × 200 欄，scrollback 1,000 列），提供純文字快照、輸出串流及 1.1 結構化 viewport
- PTY reader 先讀完已到達的輸出再關 holder 的 slave handle，避免 macOS 快速退出的 agent 留下空白畫面；關閉檢查不會 reap child。
- 回報 exit code 或 signal；agent 結束後保留，每次連上都在快照後送 `Exited`
- holder 協定 server：版本協商、同時一條連線、新的接手；純文字連線落後 1 MiB 斷線，取 frame 的連線多保留一份最大 frame 空間
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
| `server` | holder 協定 server、停止流程、安全網；`server/control` 驗 generation／owner，實際操作後回覆 |
| `pty` | spawn agent、控制鍵位元組、唯一的 PTY FIFO thread（bytes 與 operation 共用佇列 64） |
| `screen` | alacritty 畫面、純文字快照、終端查詢回覆；`screen/frame` 取完整格子，`screen/history` 沿用同 parser 追蹤 row id；`screen/narrow` 處理單欄放不下的寬字 |
| `exit` | `waitpid` 與 signal 名稱 |
| `client` | 同步的協定 client（探測 example 與測試用） |
| `sidecar` | 只有說明：不做附屬程序協定（第 7 施工關 P2 推翻第 4 施工關 P8 的 `SpawnSidecar`）；codex app-server 由 PTY 裡的 `sh` 包裝在背景起，跟 agent 同一個 process group |

## Shutdown 做什麼

1. agent 還在：對 agent 的 process group（＝ agent pid，一定大於 1）送 SIGHUP。
2. 最多等 5 秒，再對同一個 group 送 SIGKILL。agent 早就結束的，直接對 group 送 SIGKILL 清掉留下的子程序：結束的 agent 一直保留成 zombie 直到這裡才回收，所以 group id 不會被別的程序重用。
3. 刪 socket、放掉 lock、exit 0。

## 依賴規則

- 一般依賴：`agend-core`、`portable-pty`、`alacritty_terminal`（關掉預設的 `serde`）、`parking_lot`、`base64`、`serde_json`、`libc`、`unicode-width`（parser 已有的依賴）
- `cargo xtask check-deps`：不能依賴 async runtime、SQLite、`agend-daemon`
- alacritty_terminal 的 `event_loop`（連帶 `polling`）無法用 feature 關掉；holder 不用它

## 入口

- `agend_holder::run`（`agend holder` 子命令）
- `agend_holder::server::serve`、`agend_holder::client::HolderClient`、`agend_holder::paths::is_running`
- 探測工具：`cargo run -p agend-holder --example holder_probe -- <start|snapshot|key|shutdown|demo> …`

真 PTY frame 的 holder／client wire golden 已固定，只正規化程序 generation；完整 serializer round-trip 與既有 terminal_frames 共 8 tests 通過。[證據](../../docs/gates/gate-11c-frame-order-validation.md)。

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-holder
~/.cargo/bin/cargo xtask accept holder
```
