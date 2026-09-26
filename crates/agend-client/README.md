# agend-client

> **TL;DR**
> - 同步 I/O 連 daemon 的 `run/daemon.sock`、重試、協定版本檢查；CLI、TUI、未來 Rust GUI 共用（第 8 施工關）。
> - 記住：**不建 async runtime、不讀環境變數與設定檔**；socket 路徑與呼叫者身分由呼叫端給。
> - 下一步：第 9 施工關的 CLI 命令用 `Client::connect` + `request`；TUI 改接本 crate 是第 11 施工關 B 段。

## 負責

- unix socket 連線、`hello`（帶選填的 `caller`）、協定版本檢查（要 1.1）
- daemon 重啟中重試：每 100 ms 一次、最多 10 秒，之後印出明確訊息
- 請求依 `request_id` 等回應（10 秒）；送出後斷線只重送標明可重做的請求
- 事件：`subscribe_events` 之後的 `next_event`

## 不負責

- 啟動或重啟 daemon
- 讀 `AGEND_HOME`、`AGEND_INSTANCE`、設定檔、開 DB（呼叫端傳入）
- 決定哪些 agent 命令可重做（第 9 施工關逐一決定）

## API

| 呼叫 | 做什麼 |
|---|---|
| `Client::connect(socket, caller)` | 連線＋`hello`；socket 不存在、連線被拒、`hello` 前就斷 → 每 100 ms 重試，最多 10 秒 |
| `Client::connect_once(socket, caller)` | 只試一次（TUI、`agend debug watch` 有自己的重連） |
| `request(&req, Redo::Safe \| Redo::Never)` | 送出、等同一個 `request_id` 的回應（10 秒）；寫不出去＝沒送到 → 重連再送；送出後斷線：`Safe` 重連再送，`Never` 回 `Restarted` |
| `get_fleet()` | 全貌（`Redo::Safe`） |
| `resolve_attention(id, action)` | 操作者處理「需要你」項目（`Redo::Never`） |
| `subscribe_events(after)`、`next_event()` | 訂閱事件、阻塞讀下一個（等回應時讀到的事件先留著） |
| `retried()` | 這次為了重連花了多久（`agend debug ping` 印 `retried 1.4 s`） |

`caller`：agent 裡填 `AGEND_INSTANCE`，操作者填 `None`（`agend` 從環境變數算好再傳入）。

## 錯誤（`ClientError`）

| 種類 | 什麼時候 | 訊息 |
|---|---|---|
| `Unreachable` | `connect` 10 秒內連不上 | `cannot reach the AgEnD daemon at <path> after 10 s (<原因>). Is it running? Start it with: agend daemon` |
| `Connect` | `connect_once` 失敗，或重試也修不好（例如權限不足） | `cannot reach the AgEnD daemon at <path> (<原因>)` |
| `Version` | daemon 協商到 1.0（立刻失敗、不重試），或 major 不合 | `the daemon speaks client protocol 1.0; this agend needs 1.1 — restart the daemon with this binary` |
| `Daemon { code, message }` | daemon 回的錯誤；`code` 是 core 的 `client::error_code` | `<code>: <message>`，例如 `forbidden: only the operator can resolve needs-you items; ask the operator with agend ask` |
| `Restarted` | `Redo::Never` 的請求送出後斷線 | `daemon restarted during the request; check with agend status` |
| `Disconnected` | 讀事件時連線結束、10 秒沒有回應、收到不是協定的行 | `the daemon closed the connection` 等 |

第 9 施工關的 CLI 依種類選 exit code 與訊息。

## 模組

| 模組 | 職責 |
|---|---|
| `connection` | `Client`：連線、`hello`、請求／回應、事件 |
| `retry` | `RESTART_RETRY_WINDOW` = 10 秒、`RETRY_EVERY` = 100 ms、`Redo` |
| `version` | 要 1.1；協商到 1.0 的訊息 |

## 依賴規則

- 一般依賴：`agend-core`、`serde_json`
- dev 依賴：`agend-testkit`（假 daemon、proxy）
- 禁止：async runtime、SQLite、`agend-daemon`（`cargo xtask check-deps`）；反過來 `agend-daemon` 也不能依賴本 crate（server 與 client 各自編碼，契約才驗得到兩邊一致，第 8 施工關 P10）

## 入口

- `agend_client::{Client, ClientError, Redo, RESTART_RETRY_WINDOW}`
- `examples/client_probe.rs`：`client_probe resolve <attention-id> <action>`（第 9 施工關有命令前給「你親自驗收」用）

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-client
```
