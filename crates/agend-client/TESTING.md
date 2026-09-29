# agend-client 測試

> **TL;DR**
> - 對 testkit 的假 daemon 跑（`tests/client.rs`）；真 daemon 那一側由 CLP 契約保證假 daemon 跟真的一樣（`crates/agend/tests/client_protocol.rs`）。
> - 記住：「送出後斷線」用 testkit 的 proxy 吞掉請求、再重啟假 daemon 做出來，不手寫 server。
> - 下一步：`~/.cargo/bin/cargo test -p agend-client`（約 12 秒，其中 10 秒是「連不上」那個測試）。

## 怎麼跑

```bash
~/.cargo/bin/cargo test -p agend-client
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `retry::tests` | 重試視窗 10 秒、每 100 ms 一次（規劃 §4.7、第 8 施工關 P7） |
| `version::tests` | 協商到 1.0、1.1 回確切的「stop the daemon (Ctrl-C) and start this binary」訊息；1.2 以上可以；以後要 1.3 的 CLI 遇到 1.2 的 daemon 說 `run: agend daemon restart`（第 9 施工關） |
| `tests/client.rs::connect_retries_for_ten_seconds_then_says_what_to_do` | 沒有 daemon：10–11 秒後 `Unreachable`，訊息逐字比對 |
| `connect_once_does_not_retry` | `connect_once` 1 秒內失敗 |
| `connect_waits_for_a_daemon_that_comes_back` | 1.2 秒後才出現的假 daemon 連得上，`retried()` ≥ 1 秒 |
| `a_1_0_daemon_fails_at_once_with_what_to_do`、`a_version_mismatch_is_not_retried` | 版本不合 1 秒內失敗、不重試（P3） |
| `a_read_is_sent_again_after_the_daemon_restarts` | `get_fleet` 送出後 daemon 重啟：重連、重送、拿到新 daemon 的全貌（P7） |
| `a_request_that_may_change_something_is_not_sent_again` | `resolve_attention` 送出後 daemon 重啟：`Restarted`，新 daemon 沒收到它（P7） |
| `daemon_errors_keep_their_code_and_only_the_operator_resolves` | agent 身分 → `forbidden`（訊息逐字）；不存在的 id → `unknown_attention`；操作者成功後項目消失（P2、P5） |
| `a_terminal_reader_blocks_while_the_sender_writes_and_close_ends_it` | 第 11 施工關 B 段 P1：一條 thread 卡在 `next_terminal`，另一條用 `Sender` 訂閱（收到畫面）、送 `terminal_input`（假 daemon 記下原樣位元組）；不存在的 instance 的錯誤由讀的一方收到；`close()` 1 秒內讓讀的 thread 讀到結束、之後寫入失敗 |
| `a_screen_line_over_the_request_limit_is_read_whole` | 比 `MAX_LINE_BYTES`（8 MiB）長的畫面行照樣讀得進來：8 MiB 只限制 daemon 讀進來的行 |
| `answer_ask_reaches_the_daemon_and_unknown_asks_are_refused` | `answer_ask` 被接受；沒有的請示 → `unknown_ask` |
| `events_follow_the_fleet_view_and_a_bad_cursor_is_a_gap` | 全貌之後的事件連號；等回應時讀到的事件留給 `next_event`；壞游標 → `event_gap`；daemon 關掉 → `Disconnected` |

## 用到的假實作

- `agend_testkit::fake_daemon::FakeDaemon`（`start_at` 在同一個路徑重啟）
- `agend_testkit::contract::client::proxy::Proxy`（吞掉一個請求，做出「送出後斷線」）

## 還沒測的

- [ ] daemon 卡住但沒死（回應逾時 10 秒）：沒有測試；心跳不在本關（第 8 施工關「本關不做」）
- [x] 真 CLI 命令的 exit code 對應（第 9 施工關：`crates/agend/tests/cli.rs` 的 CLI-n 表）

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-client
```
