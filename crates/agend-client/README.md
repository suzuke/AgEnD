# agend-client

> **TL;DR**
> - 同步 I/O 連 daemon、重試、協定版本檢查；CLI、TUI、未來 Rust GUI 共用。
> - 記住：**不建 async runtime**，CLI 啟動要輕（實測 p50 4.1 ms）。
> - 下一步：第 8 關實作。

## 負責

- unix socket 連線
- daemon 重啟中重試最多 10 秒，之後印出明確訊息
- 連線時檢查協定版本

## 不負責

- 啟動或重啟 daemon
- 讀設定、開 DB

## 模組

| 模組 | 職責 |
|---|---|
| `connection` | 同步連線 |
| `retry` | `RESTART_RETRY_WINDOW` = 10 秒 |
| `version` | 協定版本檢查 |

## 依賴規則

- 一般依賴：`agend-core`
- 禁止：async runtime、SQLite、`agend-daemon`（`cargo xtask check-deps`）

## 入口

- `agend_client::retry::RESTART_RETRY_WINDOW`

## 下一步

```bash
cargo test -p agend-client
```
