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
| `version::tests` | 協商到 1.0 回確切的「restart the daemon with this binary」訊息；1.1 以上可以 |
| `tests/client.rs::connect_retries_for_ten_seconds_then_says_what_to_do` | 沒有 daemon：10–11 秒後 `Unreachable`，訊息逐字比對 |
| `connect_once_does_not_retry` | `connect_once` 1 秒內失敗 |
| `connect_waits_for_a_daemon_that_comes_back` | 1.2 秒後才出現的假 daemon 連得上，`retried()` ≥ 1 秒 |
| `a_1_0_daemon_fails_at_once_with_what_to_do`、`a_version_mismatch_is_not_retried` | 版本不合 1 秒內失敗、不重試（P3） |
| `a_read_is_sent_again_after_the_daemon_restarts` | `get_fleet` 送出後 daemon 重啟：重連、重送、拿到新 daemon 的全貌（P7） |
| `a_request_that_may_change_something_is_not_sent_again` | `resolve_attention` 送出後 daemon 重啟：`Restarted`，新 daemon 沒收到它（P7） |
| `daemon_errors_keep_their_code_and_only_the_operator_resolves` | agent 身分 → `forbidden`（訊息逐字）；不存在的 id → `unknown_attention`；操作者成功後項目消失（P2、P5） |
| `events_follow_the_fleet_view_and_a_bad_cursor_is_a_gap` | 全貌之後的事件連號；等回應時讀到的事件留給 `next_event`；壞游標 → `event_gap`；daemon 關掉 → `Disconnected` |

## 用到的假實作

- `agend_testkit::fake_daemon::FakeDaemon`（`start_at` 在同一個路徑重啟）
- `agend_testkit::contract::client::proxy::Proxy`（吞掉一個請求，做出「送出後斷線」）

## 還沒測的

- [ ] daemon 卡住但沒死（回應逾時 10 秒）：沒有測試；心跳不在本關（第 8 施工關「本關不做」）
- [ ] 真 CLI 命令的 exit code 對應（第 9 施工關）

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-client
```
