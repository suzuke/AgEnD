# agend-client

> **TL;DR**
> - 同步 I/O 連 daemon 的 `run/daemon.sock`、重試、協定版本檢查；CLI、TUI、未來 Rust GUI 共用（第 8 施工關）。
> - 記住：**不建 async runtime、不讀環境變數與設定檔**；socket 路徑與呼叫者身分由呼叫端給。
> - 下一步：第 9 施工關的 CLI 用 `Client::connect` + `request`（要 1.3）；第 11 施工關 B 段的 TUI（`ClientSource`）用 `connect_once`、`next_event`、`next_terminal` 與寫入端 `Sender`。

## 第 10 施工關（已驗收，2026-10-02）

一般連線要求 client protocol 1.3；`resolve_attention_with_note` 可送退回修改理由。`agend daemon restart` 仍只要求 1.2，讓新 CLI 能重啟舊 daemon。

## 第 11 施工關 C 段（實作中）

client 1.4 的型別與專用傳輸已加入；一般 `NEEDED` 維持 1.3。hello 提供 1.4／1.3，真 daemon 已協商 1.4，FakeDaemon 預設 1.3，注入真 producer 的 C fixture 提供 1.4；新 API 先檢查 1.4，不向舊 daemon 送操作。

完整終端使用專用連線：背景 thread 以 `Sender::subscribe_terminal_frames`／`set_terminal_viewport`／`terminal_control` 寫一次，另一條 thread 用 `Client::next_full_terminal` 讀 `FullTerminalUpdate`。回覆與拒絕保留 request id／view id，caller 在 daemon 端驗；generic `request` 拒絕這些連線範圍請求，不能經 Redo 重連重送。

新讀取行含換行最多 8 MiB；無效 frame／partial EOF／超限會關閉所有 clone 並永久作廢該完整終端 reader。macOS peer 已半關閉時，SHUT_RDWR 失敗改分別關閉寫／讀方向。新請求含換行最多 1 MiB，輸入 base64 與 resize 尺寸先驗，整次拒絕；所有 Sender clone 共用寫入鎖，完整終端一行從等待鎖到寫完最多 5 秒，失敗明示可能部分送出、不重送。

daemon 多視窗／frame 更新與 TUI 已接通，六項 fake／真 C 契約及真 Source／App 回歸已建立；C 段驗收收尾與 merge 仍待確認，見 [驗收收尾](../../docs/gates/gate-11c-closeout.md)。
## 負責

- unix socket 連線、`hello`（帶選填的 `caller`）、協定版本檢查（要 1.3；`agend daemon restart` 只要 1.2＝有 `daemon_restart` 的版本）
- daemon 重啟中重試：每 100 ms 一次、最多 10 秒，之後印出明確訊息
- 請求依 `request_id` 等回應（預設 10 秒，`request_within` 可以更久）；送出後斷線只重送標明可重做的請求
- 記住 daemon 的 `hello`（1.2：版本、pid、`boot_id`）；等連線被 daemon 關掉（`wait_closed`，重啟用）
- 事件：`subscribe_events` 之後的 `next_event`
- 終端（第 11 施工關 B 段 P1）：`next_terminal` 阻塞讀畫面與 PTY 位元組；`sender()` 給只寫不讀的 `Sender`，另一條 thread 用它送 `subscribe_terminal`、`terminal_input`（base64 由本 crate 編碼）、`close()`

## 不負責

- 啟動或重啟 daemon
- 讀 `AGEND_HOME`、`AGEND_INSTANCE`、設定檔、開 DB（呼叫端傳入）
- 決定哪些命令可重做（CLI 決定，第 9 施工關 P5：讀取與 `send`）

## API

| 呼叫 | 做什麼 |
|---|---|
| `Client::connect(socket, caller)` | 連線＋`hello`；socket 不存在、連線被拒、`hello` 前就斷 → 每 100 ms 重試，最多 10 秒 |
| `Client::connect_once(socket, caller)` | 只試一次（TUI、`agend debug watch` 有自己的重連） |
| `request(&req, Redo::Safe \| Redo::Never)` | 送出、等同一個 `request_id` 的回應（10 秒）；寫不出去＝沒送到 → 重連再送；送出後斷線：`Safe` 重連再送，`Never` 回 `Restarted` |
| `Client::connect_needing(socket, caller, version)` | 同 `connect`，但只要求 daemon 至少選到 `version`（`agend daemon restart` 用 1.2） |
| `request_within(&req, redo, within)` | 同 `request`，等回應最多 `within`（重啟預檢 70 秒、`send` 70 秒） |
| `daemon()` | daemon 的 `hello` 回覆（1.2：`daemon_version`、`daemon_pid`、`boot_id`） |
| `wait_closed(within)` | 讀到 daemon 關掉這條連線為止；`within` 內沒關 → `false` |
| `get_fleet()` | 全貌（`Redo::Safe`） |
| `resolve_attention(id, action)` | 操作者處理「需要你」項目（`Redo::Never`） |
| `subscribe_events(after)`、`next_event()` | 訂閱事件、阻塞讀下一個（等回應時讀到的事件先留著） |
| `retried()` | 這次為了重連花了多久（`agend debug ping` 印 `retried 1.4 s`） |
| `answer_ask(ask_id, source, reply)` | 操作者回答請示（`Redo::Never`；第 11 施工關 B 段） |
| `next_terminal()` | 阻塞讀下一個 `TerminalUpdate`：`Screen`（畫面）或 `Bytes`（解碼後的 PTY 位元組）；任何錯誤行（`no_terminal`、`forbidden`、`not_supported`）是 `Daemon`，連線結束是 `Disconnected` |
| `sender()` → `Sender` | 這條連線的寫入端（`try_clone`）：`subscribe_terminal(id)`、`terminal_input(id, bytes)`（只寫一行、不等回覆）、`close()`（`shutdown(Both)`：卡在讀取的 thread 立刻讀到 EOF；只丟掉 `Sender` 不會關連線） |

`caller`：agent 裡填 `AGEND_INSTANCE`，操作者填 `None`（`agend` 從環境變數算好再傳入）。

## 錯誤（`ClientError`）

| 種類 | 什麼時候 | 訊息 |
|---|---|---|
| `Unreachable` | `connect` 10 秒內連不上 | `cannot reach the AgEnD daemon at <path> after 10 s (<原因>). Is it running? Start it with: agend daemon` |
| `Connect` | `connect_once` 失敗，或重試也修不好（例如權限不足） | `cannot reach the AgEnD daemon at <path> (<原因>)` |
| `Version` | daemon 協商到比 1.3 舊（立刻失敗、不重試），或 major 不合 | `the daemon speaks client protocol 1.1; this agend needs 1.3 — stop the daemon (Ctrl-C) and start this binary: agend daemon`（daemon 有 `daemon_restart`（1.2 以上）時改成 `— run: agend daemon restart`） |
| `Daemon { code, message }` | daemon 回的錯誤；`code` 是 core 的 `client::error_code` | `<code>: <message>`，例如 `forbidden: only the operator can resolve needs-you items; ask the operator` |
| `Restarted` | `Redo::Never` 的請求送出後斷線 | `daemon restarted during the request; check with agend status` |
| `Disconnected` | 讀事件時連線結束、10 秒沒有回應、收到不是協定的行 | `the daemon closed the connection` 等 |

第 9 施工關的 CLI 依種類選 exit code 與訊息。

## 模組

| 模組 | 職責 |
|---|---|
| `connection` | `Client`：連線、`hello`、請求／回應、事件 |
| `retry` | `RESTART_RETRY_WINDOW` = 10 秒、`RETRY_EVERY` = 100 ms、`Redo` |
| `terminal` | client 1.4 專用傳輸、有限行與失敗關閉 |
| `version` | 要 1.3（`NEEDED`）；`RESTART_SINCE` = 1.2；不合時的訊息 |

## 依賴規則

- 一般依賴：`agend-core`、`serde_json`、`base64`（PTY 位元組在 wire 上是 base64，由 adapter 編碼；第 11 施工關 B 段）
- dev 依賴：`agend-testkit`（假 daemon、proxy）、`agend-holder`（真 parser frame producer）
- 禁止：async runtime、SQLite、`agend-daemon`（`cargo xtask check-deps`）；反過來 `agend-daemon` 也不能依賴本 crate（server 與 client 各自編碼，契約才驗得到兩邊一致，第 8 施工關 P10）

## 入口

- `agend_client::{Client, ClientError, Redo, RESTART_RETRY_WINDOW}`
- `examples/client_probe.rs`：`client_probe resolve <attention-id> <action>`（第 9 施工關有命令前給「你親自驗收」用）

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-client
```
