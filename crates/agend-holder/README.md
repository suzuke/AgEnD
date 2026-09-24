# agend-holder

> **TL;DR**
> - 每個 instance 一個 holder 程序：PTY、畫面、附屬程序。
> - 記住：**PTY 只收單一控制鍵**；holder 被硬殺時 agent 會一起死（共同上限）。
> - 下一步：第 4 施工關實作。

## 負責

- 以 portable-pty 啟動 agent、注入環境（身分、home、含 shim 的 PATH）
- 以 alacritty_terminal 維護畫面、提供快照與輸出串流
- 啟動並持有附屬程序（codex app-server、opencode serve）
- 回報 exit code／signal
- holder 協定 server：有版本協商、daemon 斷線後接受重連

## 不負責

- 流水線或送達邏輯
- 開 DB
- 螢幕分類（core 的 `screen`）
- 自己重啟 agent

## 模組

| 模組 | 職責 |
|---|---|
| `pty` | PTY 讀寫、resize、signal；`control_key_bytes` |
| `screen` | 畫面維護與快照（重連給畫面，不重播位元組） |
| `sidecar` | 附屬程序 |
| `exit` | 結束狀態 |
| `server` | holder 協定 server |

## 依賴規則

- 一般依賴：`agend-core`（之後加 portable-pty、alacritty_terminal）
- 與 daemon 同一 binary（`agend holder`），但不依賴 `agend-daemon`

## 入口

- `agend_holder::pty::control_key_bytes`
- 之後：`agend holder` 子命令

## 下一步

```bash
cargo test -p agend-holder
```
