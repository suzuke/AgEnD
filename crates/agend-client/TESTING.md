# agend-client 測試

> **TL;DR**
> - 對 testkit 的假 daemon 跑（`tests/client.rs`）；真 daemon 那一側由 CLP 契約保證假 daemon 跟真的一樣（`crates/agend/tests/client_protocol.rs`）。
> - 記住：「送出後斷線」用 testkit 的 proxy 吞掉請求、再重啟假 daemon 做出來，不手寫 server。
> - 下一步：`~/.cargo/bin/cargo test -p agend-client`（約 12 秒，其中 10 秒是「連不上」那個測試）。

## 第 10 施工關驗證

protocol 1.3 版本底線、action note 與新增命令的 wire golden 在 xtask；真 pipeline 和 TUI 操作在 `agend` 的 Gate 10 程序測試。

## 第 11 施工關 C 段（已驗收並合併 #145）

`tests/full_terminal.rs` 的 native socket peer 用 core serializer 與真 holder `Screen` parser 產生 frame，驗新 reader／Sender；它不是 daemon 控制權策略的假替代。涵蓋能力不足先拒絕、generic retry 不送 attach 請求、viewport／控制／拒絕的 request id、CJK／組合字／mode、含換行的 8 MiB 邊界、錯誤 frame 永久失效、partial EOF、整次貼上拒絕、替換 socket 不重連，以及真 socket 背壓的 5 秒寫入期限。peer 半關閉後 EOF release 的回歸曾在原 SHUT_RDWR 邏輯失敗，修正後通過；原 log 保留。

legacy 9 MiB 純文字畫面的測試保留。`agend-holder` 只作 dev-dependency，讓 consumer 讀到真 parser 產物；一般 client 依賴仍不含 holder 或 async runtime。端到端／最新 head verifier／CI、使用者實機紀錄與後續自動驗證方式見 [驗收收尾](../../docs/gates/gate-11c-closeout.md)；使用者已明確確認 #145 合併。
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

## 第 12A client 基礎

第 12A 的 `tests/once.rs` 使用 FakeDaemon 的真正 protocol producer，經 native Unix socket／Proxy 驗成功、舊版本拒絕、無 daemon／過期期限立即失敗、hello 與 reply 共用期限、回覆遺失只送一次、逐 byte 慢 hello 不延長期限、超限回覆提早拒絕，以及無 request id 請求先拒絕。`once::tests` 用真 Unix socket pair 與小 send buffer 驗已寫前綴後逾時，沒有補寫或重送。另以 32 MiB 的 send／ask／workflow／request id／caller 驗本機提早拒絕。

四批 fresh verifier 的 CPU 逾時反例均保留；另一次平台中斷只記未完成。回歸包含大量 escaping、接近 8 MiB 的 plain 字串、由真正 producer 的 Fleet 擴增成大型回覆，以及跨 4 KiB 邊界的 UTF-8／控制字元與原生 JSON byte 等價。另用真正 producer 的 2,790,000 個空 stages、合法 data-before-type 順序及 native socket 驗最後一次讀取後的 CPU 尾段；巢狀提問用 2,785,000 個空 options 重驗原反例窗口。分段 decoder 與 producer 的多種 command result、ask 各 entry／reply 型別比對兩種欄位順序。壁鐘斷言包含 100 ms 餘裕；不宣稱作業系統硬即時保證，配置／釋放記憶體仍須完成才能返回。

大型回覆的 CPU 期限另直接解析真 producer JSON，斷言解析時逾時；native socket 路徑接受先發生的讀取逾時（macOS WouldBlock／TimedOut）或解析逾時。兩路都維持 80 ms 期限與 100 ms 餘裕，避免把排程先後誤當協定錯誤。原 macOS push CI 的錯誤型別斷言失敗 log 保留。

```bash
cargo test -p agend-client --test once --lib
```

這些測試只驗新 client 基礎，尚未證明 Claude channel／Stop／ACK 或第 12A 完成。

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

`full_terminal` 的 8 MiB 邊界 case 在 consumer 呼叫 close 後才開始 peer 的 5 秒 EOF 檢查；正向邊界必須完整 write，負向仍需作廢所有 Sender clone。原 macOS PR CI 的 timeout log 保留。

## 第 12A Claude bridge

`tests/once.rs` 加驗 1.5 操作遇舊 daemon 在 hello 後先拒絕、不送 RPC；generic `request` 的 Safe／Never 都不能送 Claude 操作；32 MiB hook 在 connect 前拒絕。新 reply decoder 與完整 native daemon producer 配對的 channel／Stop／ACK 測試見 [bridge 基礎](../../docs/gates/gate-12a-bridge.md)。

## 真 CLI 啟動畫面診斷

`startup_frame` example 經 native daemon／holder 取得 frame，保存原始 cells、尺寸、revision 與 startup classifier 結果；只發 terminal subscribe，沒有控制權或輸入請求。native fixture 回歸觀察 Unknown → 三個 production keys → Ready，核 reader 未增加按鍵；它不是新的真 Claude 驗收。只讀範圍及新真 CLI 計畫另依 [D40](../../docs/decisions/d40.md) 授權。

第三次 observed smoke 證實 instance add 可早於 terminal 註冊；consumer 的 no_terminal 拒絕正確。新的首次觀察 runner 以真 daemon／holder／shell 重現原拒絕，再驗有界等候，沒有修改 client API，見[紀錄](../../docs/gates/gate-12a-observed-smoke-v3.md)。
