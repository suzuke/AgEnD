# Client protocol 契約

> **TL;DR**
> - CLP-1–28 是 fake／真 daemon 共用的 client protocol 規則；C 段新增 CLP-23–28。
> - 每條規則有 case 與 mutant；C fixture 必須由真 producer 提供 frame，不手寫格子。
> - 下一步：跑 `cargo test -p agend-testkit --test contract_teeth`；真程序 C 契約跑 `cargo test -p agend --test full_terminal_contract`。

## Client protocol（`CLP`，12 條，第 8 施工關）

**第 9 施工關再加 5 條（`CLP-13`…`CLP-17`），第 11 施工關 B 段再加 5 條（`CLP-18`…`CLP-22`，終端與打字），共 22 條；標題留著第 8 施工關的字，因為名詞表連到這個錨點**。fixture 多五個方法：`agents`＝兩個能互傳訊息的 instance、`fresh_name`＝還沒人用的名字（第 9 施工關）；`make_output`＝讓終端印出新東西（假 daemon `push_terminal_bytes`，真 daemon 的計數器每秒自己印）、`typed`＝終端收到的輸入（假 daemon 記下的位元組；真 daemon 看畫面，PTY 會回顯）、`stopped_instance`＝一個 `failed` 的 instance（第 11 施工關）。對象不是 trait，是一個講 client protocol 的 server：同一套 case 對 testkit 的 `FakeDaemon`（`tests/contract_fakes.rs`）與真的 `agend daemon`（`crates/agend/tests/client_protocol.rs`，暫存 home、真 binary）跑（[第 8 施工關 P9](../../docs/gates/gate-08-client.md#p9client-協定契約假-daemon-與真-daemon-跑同一套)）。驅動端是 `ProbeClient`，不是 `agend-client`。mutant 是「假 daemon 前面加一個改行的 proxy」（`contract::client::proxy`），假 daemon 本身沒有「故意弄壞」的開關。

fixture（`ClientProtocolFixture`）：`socket`、`emit`（讓至少一個新事件發生）、`burst(n)`、`restart`（同一個 socket 重啟 server）、`retry_item`（一個可以 `retry` 的「需要你」項目）、`terminal_instance`。真 daemon 的 `emit` 是對一個 `failed` 的 instance 送 `retry`（真的事件，不是假資料）。

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| CLP-1 | 第一行不是 `hello`（別的請求、不是 JSON）→ `error hello_required`，然後關連線 | P3、D26 | `NoHelloRequired` |
| CLP-2 | `hello` 沒有共同的 major → `version_mismatch` 並關閉；1.0 的 client 拿到 1.0；帶 `caller` 的 1.1 `hello` 拿到 1.1 | P2、P3、D26 | `AcceptsAnyMajor` |
| CLP-3 | `get_fleet` 回 `fleet`（有 `general` team）；用它的 `as_of_event_id` 訂閱，拿到的事件 id 從 `as_of + 1` 開始連號，包括 `get_fleet` 與訂閱之間發生的 | P4 | `SkipsFirstEventAfterAsOf` |
| CLP-4 | 重啟前的游標、比最新的 id 還大的游標 → 只有 `event_gap`，沒有事件 | P4 | `IdsFromOne`（反向檢查：事件 id 改回從 1 開始）、`HidesEventGap` |
| CLP-5 | 不帶游標訂閱 → 重播留著的事件（1.0 的意思）；重播從最舊的一筆開始（「最舊 − 1」接得上，再前一個是 `event_gap`） | P4、D26 | `NoneBecomesZero` |
| CLP-6 | 未知的請求 → `unknown_request`、不是 JSON 的行 → `invalid_request`，連線不斷、之後的請求照常回 | P3 | `GoesSilentOnUnknown` |
| CLP-7 | 兩個 client 用同一個游標訂閱，收到的事件相同、順序相同 | P8 | `ReordersForSecondClient` |
| CLP-8 | 2000 個事件：一直讀的 client 全部收到且順序正確；每 10 ms 讀一行的收到 `event_gap` 後被關；完全不讀的在 5 秒寫入逾時後被關、收不到 `event_gap` | P8 | `NeverDropsLaggers` |
| CLP-9 | server 重啟：舊連線被關；新連線的全貌 `as_of_event_id` 比重啟前看過的每個 id 都大 | P1、P4 | `KeepsOldConnection` |
| CLP-10 | 對沒有終端的 instance（不存在的 id）送 `terminal_input` → `no_terminal`、沒有這個請示的 `answer_ask` → `unknown_ask`（帶 request id）；都不發事件、全貌不變（第 11 施工關 B 段前 `terminal_input` 一律 `not_supported`，B 段起改成這樣） | P6；第 11 施工關 P6 | `AcceptsRefused` |
| CLP-11 | agent（`hello` 帶 `caller`）送 `resolve_attention`：不管 id 存不存在都是 `forbidden`；操作者送不存在的 id、或不在 `actions` 裡的操作 → `unknown_attention`；列出的操作 → `accepted`、`attention_resolved` 事件、全貌裡不再有它 | P2、P5 | `AgentMayResolve`、`HidesResolvedEvent` |
| CLP-12 | `subscribe_terminal` 先回那個 instance 的 `terminal_snapshot` | P6 | `DropsSnapshot` |
| CLP-13 | 權限兩個方向（第 9 施工關 P1）：agent（`hello` 帶 `caller`）送 `operator` 的 `instance_add`／`instance_remove`／`daemon_restart`／`task_cancel` 一律 `forbidden`；操作者送 `command` 的 `status`／`send`／`done` 一律 `forbidden`；全貌不變 | 第 9 施工關 P1、D17 | `AgentsAreOperators` |
| CLP-14 | 操作者 `instance_add` 新名字 → `instance_added`（名字、非空的 `working_directory`），全貌出現它且 `working_directory` 相同；同名再加 → `instance_exists`；不合規則的名字 → `invalid_request`；`instance_remove` → `accepted`、全貌不再有它；再刪 → `unknown_instance` | 第 9 施工關 P6 | `ReaddIsAccepted` |
| CLP-15 | `daemon_restart { binary }` 的形狀：`binary` 是跑不起來的路徑 → `preflight_failed`；連線不斷、沒有事件、全貌不變 | 第 9 施工關 P6、P7 | `DropsRestartBinary` |
| CLP-16 | 操作者取消未知 task → `invalid_request`（帶 request id），沒有事件、全貌不變 | 第 10 施工關 P10 | `AcceptsTaskCancel` |
| CLP-17 | `send` 同一個 `message_id`（UUID v4）再送一次仍 `accepted`、收件者的 `inbox` 只有一則；同 id 不同內容、不是 UUID v4 的 id、超過 1 MiB 的 body（第 9 施工關 L17）→ `invalid_request`；`inbox --after` 自己的一則 → 只回之後的；不存在的 id、別人的訊息 → `unknown_message` | 第 9 施工關 P2、P5；第 7 施工關 P5 | `NewIdOnResend`、`InboxIgnoresAfter`、`AcceptsHugeBody` |
| CLP-18 | `subscribe_terminal` 之後，終端印出新東西 → `terminal_bytes`；同一條連線再訂一次 → 新的畫面，而且看得到那段輸出 | 第 11 施工關 P5；第 8 施工關 C8 | `FreezesScreen` |
| CLP-19 | 對沒有終端的 instance（不存在的 id）`subscribe_terminal` → `no_terminal`（不帶 request id），連線不斷；這條連線之前訂的終端**不再**送 `terminal_bytes`（失敗的重訂也取代舊的） | 第 11 施工關 P1、P5 | `ScreenForAnyInstance` |
| CLP-20 | `terminal_input`：agent（`hello` 帶 `caller`）不管 instance 存不存在都是 `forbidden`（先查身分）；操作者對不存在的 instance → `no_terminal`；操作者對有終端的 instance → 不回任何東西，位元組原樣到終端，agent 送的沒有；錯誤都不帶 request id | 第 11 施工關 P6、D17 | `AnyoneMayType` |
| CLP-21 | 操作者對 `failed` 的 instance（沒有活的終端）送 `terminal_input` → `no_terminal`（不帶 request id），連線不斷 | 第 11 施工關 P6（verifier） | `TypesIntoTheLiveOne` |
| CLP-22 | 操作者送的 `terminal_input` 轉成 holder 的請求行（含換行）剛好是 holder 的上限（`protocol::holder::MAX_REQUEST_LINE`，1048576 bytes）→ 照常轉給 holder、不回任何東西；多 1 byte（1048577）→ `invalid_request`（不帶 request id，訊息寫明上限），什麼都沒轉；兩種之後連線都不斷、之後的輸入照常到終端 | 第 11 施工關 P6（verifier r2、r3） | `ForwardsHugeInput`、`RefusesAtTheLimit` |
| CLP-23 | client 1.4 的完整 frame 保留 instance／view／generation／request id；Acquire／Resize 的完整格子尺寸符合已完成的 PTY 尺寸，每次 Acquire 產生新 attach | D39 P2、P3 | `GrantBeforeMatchingFrame` |
| CLP-24 | 最後 Acquire 控制；舊視窗收到唯讀通知，舊 input／resize 拒絕且實收輸入沒有舊內容；可明確重取控制 | D39 P3 | `HidesLostControl` |
| CLP-25 | caller 優先 forbidden；foreign view、舊 generation、非法尺寸、legacy 繞過拒絕；PTY 尺寸與實收內容不變，合法輸入仍可用 | D39 P3 | `IgnoresTerminalCallerPriority` |
| CLP-26 | owner EOF 釋放控制、保留最後尺寸；前 owner 的舊 token 不恢復；沒有 owner 時 legacy 輸入可到達 | D39 P3 | `ForgetsOldOwnerRefusal` |
| CLP-27 | 各 view 歷史獨立，固定絕對 row 的文字不因新輸出跳走；底部跟隨，最後 dirty 有完整 frame | D39 P2、P4 | `UnpinsHistoryViewport` |
| CLP-28 | 失敗的重訂也作廢舊 view；新請求必須協商 1.4，1.3 明示 not_supported 並保留 request id | D39 P2、P3 | `ForgetsOldTerminalCapability` |

不釘：`status`、`send`、`inbox` 以外的 agent 命令（假 daemon 會處理，真 daemon 第 10 施工關前回 `not_supported`）；成功的 `daemon_restart`（CLI 測試對假、真 daemon 各跑一次）；codex instance 的 `terminal_input` 回 `not_supported`（fixture 沒有 codex instance；假 daemon 在 `tests/fake_daemon.rs`、真 daemon 在 `crates/agend/tests/tui_daemon.rs` 各測一次）。第 11 施工關 B 段起假 daemon 只對登記過的 instance 回畫面（改掉第 8 施工關 C2）。

**真 daemon 跑 CLP-8 的方式**：真的 `agend daemon` binary 沒辦法在測試裡產生 2000 個真事件（每個事件都要一個 instance 狀態改變），所以 CLP-8 的「真」是同一份 daemon server 程式碼（`agend_daemon::server` + `fleet`）在測試程序裡跑、直接發事件；其他條對真 binary 跑（見第 8 施工關「待你追認」）。

## C 段 fixture

`contract::terminal::FullTerminalFixture` 提供 socket、instance、input consumer 的實收紀錄及輸出觸發；六個 case 對同一介面執行。fake 由 `traits::TerminalProducer` 注入真的 holder Screen（僅 dev-dependency）；真 fixture 是 daemon、holder 與 native PTY。frame 與控制回覆均由 core 型別 serializer 編碼。

六個 mutant 改寫真正 producer 的 wire 回覆，分別破壞 grant 尺寸、失去控制通知、caller 優先錯誤、舊 token 拒絕、歷史 pinning 與舊版本錯誤；它們證明對應斷言有牙齒，不代表涵蓋每一種內部故障。

這六條不代替完整 C 驗收：native 背壓、daemon 重啟、資源清理、TUI renderer／鍵鼠／貼上與 Codex U17 還須各自證據。300 ms 時效仍依驗收矩陣另測，不以 10 秒契約 deadline 宣稱通過。

## 下一步

```bash
cargo test -p agend-testkit --test full_terminal --test contract_teeth
cargo test -p agend --test full_terminal_contract
```
