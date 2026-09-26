# 第 11 施工關：agend-tui（`tui`）

> **TL;DR**
> - attention-first TUI；畫面層提前做（你同意與第 3–10 施工關並行，放寬 D22），資料先接假來源，draft PR 不 merge。
> - 記住：**畫面只讀 `Source`，不知道資料從哪來**；接真 daemon 要等第 8 施工關把 client protocol 定案。
> - 下一步：A 段已驗收、T1–T18／G1–G4 已追認（2026-09-26）；第 8 施工關已 merge，**B 段開工前提案 P1–P7 待你確認**（見「B 段開工前提案」），確認後才寫程式。要你明確決定的差異：T1、T2、T12、T13、T18、第 8 施工關 C2、第 7 施工關 P1（codex 打字）。

**先看這條**：B 段（接真 daemon）的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑 B 段開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。A 段只用 `cargo`，不用 `agend`。

## 狀態

**實作中：畫面層完成；B 段提案中（PR #133，branch `feat/gate-11-tui-daemon`）**。

- 畫面層（2026-09-26，已 merge #120）：A 段通過、T1–T18／G1–G4 已追認；畫面、按鍵、斷線畫面已完成並有測試；資料來自腳本假來源與 testkit 假 daemon。
- B 段（2026-09-27，draft PR #133）：接真 daemon 的 P1–P7 待你確認；確認前不寫程式。

## 範圍

- 首頁（「需要你」+ 各 team 區塊）、team 頁（目標／Agents／流水線）、Task Detail、Agent Detail
- attach 單一 agent 的終端（`t`；目前是唯讀快照）
- `/` 快速跳轉、英文／繁中切換（`L`）
- 「需要你」：已讀與已解決分開；選項或自由文字回答（D35）
- daemon 斷線畫面與自動重連
- B 段（第 8 施工關後，見「B 段開工前提案」）：用 `agend-client` 實作 `Source`、`agend app` 子命令、全貌與事件接到畫面、「需要你」的 `retry`、終端即時更新與只有操作者能用的輸入

## B 段開工前提案

**待你確認**（2026-09-27）。確認前不寫程式。每項：問題 · 建議 · 理由 · 替代方案 · 例子。

已定、這裡不重問的：畫面只讀 `Fleet`、只透過 `Source` 動作（T1）；版面、按鍵、回答方式（T5、T12、T13，A 段）；已讀是 TUI 本機狀態、追問回到未讀（T4、T17；G4 已移到第 12 施工關，[第 8 施工關 P6](gate-08-client.md#p6真-daemon-本關做哪些請求g4-移走)）；**重連一律重拿全貌、不重播事件**（[第 8 施工關 P4](gate-08-client.md#p4g1-全貌與事件游標) 已改掉 T6 的「重播事件」）；身分用 `hello` 的 `caller`、從 `AGEND_INSTANCE` 來，權限只在 daemon 擋（第 8 施工關 P2、[第 9 施工關 P1](gate-09-cli.md#p1命令面本關做哪些誰能跑真-daemon-做到哪)）；client 同步 I/O、不建 runtime（D11）；`resolve_attention` 只收操作者、`retry` 的規則（第 8 施工關 P5）；`AGEND_HOME` 一律必須設（[第 9 施工關](gate-09-cli.md) P3）；codex TUI 裡人打的字是同一個 thread 的使用者訊息、不進 `messages`（[第 7 施工關 P1](gate-07-codex.md#p1daemon-怎麼跟-codex-講話)，U17 未查證）。

**要你明確決定的差異**（各項裡標「請明確決定」）：T1（P1）、T2（P1）、第 8 施工關 C2（P1，假 daemon 的終端）、T12、T13、T18（P3）、第 7 施工關 P1（P6，codex 打字）。

開工時的事實：第 8 施工關已 merge（#131）；第 9 施工關只有提案（還沒有 `agend instance`，驗收用 `daemon_probe`）；第 7 施工關在 review（PR #132，還起不了 codex）；第 10 施工關只有提案（真 daemon 沒有請示）。

### P1：真的 `Source` 放哪

- 問題：T2 說 socket client 不放 lib，等第 8 施工關做出 `agend-client`。現在有了，真實作放 lib 還是 `agend` binary？`Source` 介面要改嗎？
- 建議：
  - 放 lib：`agend_tui::source::client::ClientSource`，只呼叫 `agend-client`。lib 裡仍然沒有 socket 程式碼（`UnixStream` 只在 `agend-client`）。`agend-tui` 的 `Cargo.toml` 本來就依賴 `agend-client`（骨架時加的，目前沒用到）。
  - `poll` 絕不阻塞，但 `agend-client` 的 `next_event` 會阻塞。所以每條連線各有一條 thread 阻塞讀、經 channel 給主 thread：
    - 事件連線：`get_fleet` → `subscribe_events(as_of)`，之後只讀事件。
    - 請求連線：`resolve_attention`、`answer_ask`（都有 `request_id`，在主 thread 送、等回覆）。
    - 終端連線：開終端時才開（P5），離開終端就關。一條 thread 阻塞讀（`next_terminal`）；主 thread 經 `Client::sender()` 拿到的**寫入端**（`UnixStream::try_clone` 出來的 writer，只能送、不讀）寫 `subscribe_terminal` 與 `terminal_input`。`Client` 的讀寫都在 `&mut self` 後面，讀的 thread 卡在 `next_terminal` 時主 thread 拿不到它（加 Mutex 會互等），所以寫入要分出來。`subscribe_terminal` 與 **`terminal_input` 都走這條**：`terminal_input` 沒有 `request_id`，它的錯誤是不帶 `request_id` 的 `error`，而 `agend-client` 等回覆時會把不帶 id 的 `error` 當成「正在等的那個請求」的回覆（`connection.rs` 的 `wait_reply`）。放在請求連線上，打字的錯誤就會變成下一個 `retry` 的回覆。終端連線上沒有別的請求在等，所以這條連線上任何不帶 id 的 `error` 都是終端或打字的錯誤，一律顯示在底下並離開輸入模式；再依錯誤碼分兩種（見 P5、P6）：`forbidden`、`not_supported`（打字被拒）→ 畫面照常更新；`no_terminal`（終端結束、重訂失敗、打字時已沒有活的終端，三種都是這個碼）→ 標題 `· 已結束，重試中`、每秒重訂一次。
  - `agend-client` 只加 4 個 API（加法，不改讀取方式）：`Client::answer_ask`、`Client::next_terminal`、`Client::sender()`，以及 `Sender::close()`。`sender()` 回的 `Sender` 有 `subscribe_terminal`、`terminal_input`（只寫一行、不等回覆）與 `close()`。
  - 終端連線的生命週期（讀的 thread 卡在沒有逾時的讀取裡，丟掉 `Sender` 只關掉複製出來的 fd、socket 還開著，所以一定要明確關）：
    - 開（含重連）：主 thread `connect_once` → 拿 `sender()` → 把 `Client` 交給一條**新的**讀 thread。
    - 關（離開終端畫面、重連前）：主 thread 呼叫 `Sender::close()`＝`UnixStream::shutdown(Both)`，讀的 thread 馬上讀到 EOF、自己結束。不留 thread、不留連線（例如 `t` 開 `failed` instance 的最後畫面再 `←`，每次都收乾淨）。
    - `Sender` 寫入失敗（`EPIPE` 等）＝終端連線 EOF，走 P5 的「只重連這條」。
  - `Source` 介面：`connect` 回「`Catalog` ＋目前的需要你清單」；`FleetView` → `Catalog` 的轉換在 `ClientSource` 裡（P3），畫面看不到 `FleetView`；`ScriptedSource` 照舊直接給完整的 `Catalog`。`poll` 只給 `as_of` 之後的事件（不再從頭重播）；加 `resolve(attention_id, action)` 與終端的訂閱、輸入（P4–P6）。
  - 刪掉 `examples/support/daemon_source.rs`：`tui_fake --daemon`、`tui_accept` 經 socket 的段落、tests 改用 `ClientSource` 連 testkit 假 daemon（假 daemon 已有 `get_fleet`、`set_task`、`set_instance`）。腳本假來源 `ScriptedSource` 留著，A 段 demo 的畫面與導覽（`owner_steps.rs`）照舊用它（見 P3 的 T12）。
  - **本段也改一個 daemon 的小地方**：同一條連線重訂終端失敗（例如 holder 5 秒沒回畫面，`handlers.rs` 的 `terminal`）時，daemon 現在會留著舊的即時串流（`server.rs` 只在成功時換掉）。改成：重訂一開始就清掉這條連線舊的串流，失敗就是沒有串流，client 收到 `no_terminal`，照 P5 每秒重試。
  - **本段也要改 testkit 假 daemon**（`fake_daemon.rs` 現在：終端一律回 `fake screen of X`、從不送 `terminal_bytes`、`terminal_input` 回 `not_supported`），不然 `ClientSource` 的測試跑不起來：
    - 每個 instance 自己的畫面（`set_screen(id, text)`）；`push_terminal_bytes(id, bytes)` 送位元組（之後重訂就回新的畫面）。`subscribe_terminal` 對沒登記的 instance 回 `no_terminal`（跟真 daemon 一樣）。
      - **與已追認的第 8 施工關 C2（假 daemon 對任何 instance id 都回一張畫面、`no_terminal` 只在真 daemon 驗）不同，請明確決定**。理由：本段的 `CLP` 新列對假 daemon 與真 daemon 都跑，假 daemon 對不存在的 instance 也回畫面，同一列在兩邊結果不同；TUI 也要測「沒有終端」那條路。代價：用假 daemon 開終端的測試與 demo 要先 `set_instance` 登記 instance：`crates/agend-tui/tests/daemon_source.rs`（改用 `ClientSource` 時一起改）、`crates/agend-tui/examples/tui_accept.rs`（A 段 demo 經 socket 的段落）、`crates/agend-tui/examples/tui_fake.rs --daemon`、`crates/agend-testkit/tests/fake_daemon.rs` 的終端測試、`crates/agend-testkit/src/contract/client.rs` 的 CLP-12（先登記再訂閱）。反悔：假 daemon 改回對任何 id 回畫面、`no_terminal` 那列只對真 daemon 跑（約 15 行）。
    - `terminal_input` 依序檢查：caller 是 agent → `forbidden`；沒有這個 instance → `no_terminal`；backend 是 codex → `not_supported`（P6）；都過了才記下位元組（`terminal_inputs()` 給測試看）。
    - `resolve_attention` 可以延後發 `attention_resolved`（`hold_resolved_events(true)`，之後 `release_resolved_events()`）：現在假 daemon 回 `accepted` 的同時就發事件（`fake_daemon.rs` 開頭的說明），測不出「收到事件才消失」。
    - `CONTRACTS.md` 的 `CLP` 表加對應的列（`terminal_bytes` 之後重訂拿到新畫面、`terminal_input` 的身分與錯誤），對假 daemon 與真 daemon 都跑（第 8 施工關 P9）。
  - **與已追認的 T1「`Fleet` 只由 catalog ＋ client protocol v1 事件重建」不同，請明確決定**：B 段起 `ClientSource` 的 `Fleet` 由全貌（轉成 `Catalog` ＋需要你清單）＋ `as_of` 之後的事件重建；畫面只讀 `Fleet`、只透過 `Source` 動作這兩條不變。
  - **與已追認的 T2「socket client 放 `examples/support`、不放 lib」不同，請明確決定**。T2 的理由（lib 不寫 socket、不替第 8 施工關做決定）在這個做法下仍成立。
- 理由：放 lib 才能在 `agend-tui` 裡對假 daemon 測真實作（畫面＋協定＋client 一起），`agend app` 只剩接線；刪掉手寫的第二份 socket client，demo 走的路就是產品的路（#1493）。
- 替代方案：`ClientSource` 放 `crates/agend`（TUI lib 完全不碰 client；但 TUI 的測試要搬去 `agend`，或留兩份 client）；`agend-client` 加非阻塞的 `try_next_event`（一條連線就夠，但讀到一半的行遇到逾時要自己接回，改到 client 的讀取核心）。
- 例子：`cargo test -p agend-tui` 的 `client_source_*` 對假 daemon：`get_fleet` 回 1 個 team、2 個 instance → 首頁畫出 `┏ general`；假 daemon 發 `attention_resolved` → 下一次 `poll` 後那一項消失。
- [ ] 使用者確認

### P2：`agend app` 誰做、home 怎麼找

- 問題：第 9 施工關的命令表沒有 `agend app`；第 8 施工關寫「TUI 改接 `agend-client` 與 `agend app`（第 11 施工關 B 段）」。誰做？home 怎麼找？在 agent 裡開會怎樣？
- 建議：
  - **本段做** `agend app [--lang en|zh-TW]`：`crates/agend/src/cli.rs` 加一個分支、`--help` 加一行。預設英文（同 `tui_fake`）。
  - 互動迴圈（raw mode、alternate screen、每 250 ms 讀鍵、`tick`、離開時還原終端）從 `examples/tui_fake.rs` 搬進 lib：`agend_tui::run(source, lang)`。`tui_fake` 也改呼叫它，只剩一份。
  - home：照第 9 施工關 P3（必須設、必須是絕對路徑，否則 exit 2）。第 9 施工關先 merge 就用它的 `home::resolve()`。本段先開工的話：`debug.rs` 的 `socket()` 是私有函式、`agend debug` 在沒設 home 時 exit 1，所以把它搬成 `crates/agend` 共用的一個函式、`agend app` 用 exit 2（第 9 施工關 P3 的碼；`agend debug` 維持 exit 1，不在本段改），第 9 施工關 merge 時換成 `home::resolve()`（多了 v1 home 防護、訊息改成它那句）。
  - `caller` 從 `AGEND_INSTANCE` 來（同 CLI）。在 agent 裡開 `agend app` 照樣看得到，但 `retry`、打字會被 daemon 以 `forbidden` 拒絕（P4、P6）；TUI 不自己判斷。
  - stdout 不是終端機（例如被 pipe）：印 `agend app needs a terminal`、exit 2，不進 raw mode。
- 理由：`agend app` 只是「找 socket → 建 `ClientSource` → `run`」的接線，跟 TUI 一起驗最自然；home 規則只有一份，不在 TUI 另寫。
- 替代方案：交給第 9 施工關（要等它 merge，而且它不驗 TUI）；`agend app` 在 agent 裡直接拒絕（多一條 CLI 端規則，D17 說權限只在 daemon）；預設繁中（跟 A 段 demo 不一致）。
- 例子：`env -u AGEND_HOME agend app` → `AGEND_HOME is not set…`、exit 2；`agend app --lang zh-TW` → 首頁。
- [ ] 使用者確認

### P3：全貌與事件怎麼對到畫面

- 問題：G1–G3 由第 8 施工關補上了。哪些改讀協定？協定還缺什麼？
- 建議：`Catalog`（TUI 內部型別）留著，畫面程式不動。`ClientSource` 的 `Catalog` **只由 `FleetView` 與事件填**（轉換在 `source::client`，P1）；`ScriptedSource` 照舊手給完整的 `Catalog`（給 A 段 demo 與畫面測試）。

  | 畫面要的 | A 段（假資料） | B 段起 |
  |---|---|---|
  | team 清單 | catalog | `FleetView.teams`（真 daemon 目前只有 `general`） |
  | task、持有者 | catalog | `tasks`、`assignee`；`task_changed.task` 整筆取代 |
  | agent、backend、狀態 | catalog | `instances`；`instance_changed.instance` 整筆取代；狀態多 `starting`、`failed`（繁中 `啟動中`、`已停止`），`unknown` 顯示 `狀態不明` |
  | 「需要你」清單 | 事件重播 | `FleetView.attention` ＋之後的 `attention_required`；`attention_resolved` 拿掉；請示照舊看 `ask_updated` |
  | 排序（D36） | unblocks 一律 0、event id 當等待時間（G2） | 協定的 `unblocks`、`waiting_since_unix_ms` |
  | 「不處理的話」 | catalog 的固定文字（T16） | `if_ignored` |
  | agent 的「需要你」（T18） | 提問者或 task 持有者 | 提問者 → 項目的 `instance_id` → task 持有者（`instance-failed:g11-2` 算在 `g11-2`） |
  | 項目的 id | 請示 id 或 `event-<id>` | `attention_id` |
  | 已讀 | 本機（T4、T17） | 不變；第 12 施工關改讀 daemon |

  - 協定還沒有的，記成新缺口 **G5**（交給第 10 施工關）：關卡的種類、每個關卡的狀態與負責的 agent、task 的 repo。處理：`StageInfo.kind` 改成選填；有 `stages` 時照 `current_stage` 推「之前完成、目前進行中、之後未開始」，流水線 tab 沒有種類就不分組；沒有 `stages`（第 10 施工關前一律如此）就不畫進度條；repo 不顯示。
  - **與已追認的 T18 不同，請明確決定**：T18 說「需要你」指向提問者、否則 task 持有者，其他狀態照 catalog；B 段加上 `instance_id` 這一層，狀態改照協定（多 `starting`、`failed`，`unknown` 顯示 `狀態不明`）。「需要你」仍由 TUI 從清單算（第 8 施工關 P4），不是協定的狀態。
  - **與已追認的 T12（流水線 tab 依關卡種類分組）不同，請明確決定**：接真 daemon 時，第 10 施工關補 G5 之前流水線 tab 是不分組的清單、Task Detail 沒有 repo、關卡沒有負責的 agent。A 段 demo 用 `ScriptedSource`，分組照舊；只有經 socket 的 demo 段（A1 的 screens、A5）改用 `ClientSource`，那幾段的流水線不分組，A 段步驟 1 的 demo 檢查跟著改。
  - **與已追認的 T13（`t` 在請示列開提問者，非請示項目開 task 持有者）不同，請明確決定**：`failed` instance 的項目沒有 task，照 T13 按 `t` 會什麼都開不了。改成「提問者 → 項目的 `instance_id` → task 持有者」，`instance-failed:g11-2` 按 `t` 開 `g11-2` 的最後畫面（P5）。
- 理由：一個來源（全貌＋事件），不再有「catalog 說的」與「daemon 說的」兩份真相；缺的欄位空著，不在 TUI 推算（G1、T18 的原則）。留著 `Catalog` 讓十幾個畫面函式不用改。
- 替代方案：畫面直接讀 core 的 `FleetView`（少一層轉換，但每個畫面模組都要改，而且 G5 的欄位 core 還沒有）；關卡種類由 TUI 從 stage id 猜（猜錯會分錯組）。
- 例子：真 daemon、`g11-1` 正常、`g11-2` 放棄 → 首頁 `需要你 · 1`（`g11-2`），`┏ general` 標題右邊有「需要你 1」，general 區塊寫 `沒有進行中的目標`（沒有 task）。
- [ ] 使用者確認

### P4：「需要你」的操作：`retry` 與沒有操作的項目

- 問題：非請示的項目現在有 `actions`（第 8 施工關 P5，目前只有 `retry`）。畫面怎麼給？按了之後誰決定它消失？
- 建議：
  - 展開後，`actions` 跟請示的選項一樣是可選取的列（T5 的鍵：`→`／`Enter` 或 `1`–`9`）。`retry` 顯示 `重試`／`Retry`；不認得的操作不顯示。沒有自由文字（`a` 只給請示）。
  - 選了 → `resolve_attention`；底下訊息 `已送出：重試 g11-2`。**只有收到 `attention_resolved` 才消失**（T3 的原則：daemon 決定，不是 TUI）。`retry` 後 daemon 可能把它放回來（第 8 施工關 C6），畫面照事件走。
  - `actions` 是空的（例如跑過的 codex）→ 寫 `沒有可用的操作`，下一行照舊是「不處理的話：…」。這取代 G3 的暫時字樣「client protocol v1 還沒有處理這一項的操作」（第 8 施工關 P5 已預告）。
  - `forbidden` 等錯誤 → 底下訊息，項目留著。
  - 請示照舊走 `answer_ask`；真 daemon 在第 10 施工關前沒有請示，所以「回答請示」這半只在假 daemon 驗（A 段、demo），真 daemon 的部分等第 10 施工關（第 8 施工關 P6、第 9 施工關 P1）。B 段步驟 3 用 `failed` instance 的 `retry`。
- 理由：同一套「選一列 → 送出 → 等 daemon 的事件」，請示與非請示只差送哪個請求；本機先拿掉會在 daemon 放回來時閃一下、甚至說謊。
- 替代方案：按 `retry` 先問一次「確定？」（`retry` 只是重起、可以再放棄，多一步沒好處）；送出後本機立刻拿掉（見理由）。
- 例子：`Enter` 展開 `g11-2` → 列 `1 重試` → 按 `1` → 訊息 `已送出：重試 g11-2`、清單變 `需要你 · 0`；daemon log `g11-2: retry requested by the operator`。
- [ ] 使用者確認

### P5：`t` 的終端怎麼即時更新

- 問題：T7 只顯示一張快照。第 8 施工關的 `subscribe_terminal` 先回一張畫面（holder 算好的純文字，沒有顏色與游標），之後是 PTY 原始位元組（`terminal_bytes`）。TUI 怎麼畫出即時的畫面？
- 建議：
  - **位元組只當「畫面變了」的訊號**。終端連線收到 `terminal_bytes` 就記「有新輸出」；主 thread 每次 tick 檢查：有新輸出、而且離上一次重拿 ≥ 200 ms，就在終端連線再送一次 `subscribe_terminal`（第 8 施工關 C8：新的取代舊的），拿 holder 算好的新畫面，同時清掉記號。重拿之後才到的位元組會再設記號，所以**最後一段輸出（例如停下來的提示符號）一定會在 200 ms 後補畫**，不會卡在舊畫面。一秒最多重拿 5 次；主迴圈的 tick 改成每 100 ms 一次（原本讀鍵 250 ms）。所以從輸出到畫上去最多 **300 ms 加一次來回**（200 ms 節流＋最多一個 100 ms 的 tick＋holder 畫面的來回）。
  - 標題：`g11-1 的終端 · 即時`（取代 `唯讀快照`）。`failed` 的 instance（daemon 只回最後畫面）：`· 最後的畫面（已停止）`；這時**不能按 `i`**（底下提示「這個 agent 已停止，不能輸入」）。之後收到這個 instance 的 `instance_changed` 顯示它又跑起來（例如按了 `retry`），就自動重訂、變回 `· 即時`。終端結束（第 8 施工關 C7 的 `no_terminal … subscribe again`）：畫面留著、標題 `· 已結束，重試中`，每秒重訂一次；重訂失敗（`no_terminal`）也是同一個狀態。只有終端連線 EOF（落後 256 塊或 5 秒寫入逾時被關，第 8 施工關 P8，不一定有 `error`）→ 同樣顯示 `· 已結束，重試中`，只重連這條終端連線、每秒一次；事件連線沒斷就不進斷線畫面。holder 已經不在：照 A 段 `g11-1 沒有終端輸出。`。
  - 不改 agent 的 PTY 大小（resize）：畫面從左上角畫，超出就截掉。
  - T7 本身寫「即時串流與輸入留到第 11 施工關正式接」，這裡是兌現它，不是改它。
- 理由：holder 已經有完整的終端模擬器（`alacritty_terminal`）；TUI 自己再跑一個，起點只有純文字快照、沒有游標與屬性，套上位元組會畫錯。讓 holder 算畫面，TUI 不用多一個重依賴。
- 替代方案：TUI 內嵌 `alacritty_terminal` 套用位元組（有顏色、更即時；但起點畫面不對，要等 agent 整頁重畫才正確，而且多一個大依賴）；不管有沒有輸出，每 500 ms 重拿畫面（更簡單，但閒著也在拿、最多慢半秒）。
- 例子：`g11-1`（每秒印 `counter=N` 的假 agent）開著 `t` → 不按任何鍵，`counter=17` 一秒後變 `counter=18`。
- [ ] 使用者確認

### P6：打字：只有操作者、不會誤觸

- 問題：第 8 施工關把 `terminal_input` 留到這裡（「要先確定只有操作者能打字」）。誰能打？怎麼避免不小心按到的鍵送進 agent？codex 與 claude 一樣嗎？
- 建議：
  - **daemon**：實作 `terminal_input`。先查身分：agent → `forbidden: only the operator can type into an agent's terminal`（跟 `resolve_attention` 一樣先查身分再查 instance）；instance 沒有活的終端 → `no_terminal`；否則經 holder 長連線轉成 holder 的 `OperatorTerminalInput`（第 4 施工關已有）。這個請求沒有 `request_id`，不回成功；錯誤以不帶 `request_id` 的 `error` 回來。TUI 只在終端連線送它（P1），所以它的錯誤不會被當成別的請求的回覆；終端連線上的錯誤照 P1 的規則：一律顯示在底下、回到唯讀，`no_terminal` 另外進「已結束，重試中」。
  - holder 對輸入回的錯誤（`pty_busy`、`agent_exited`）：daemon 的 holder 長連線現在把 holder 的其他回應都丟掉（`runtime/link.rs` 的 `read_until_closed`）。**建議接受「靜靜丟掉」、只記 daemon log**（`g11-1: operator input dropped: pty_busy`）：TUI 看得到畫面，字沒出現就是沒送到；要把錯誤轉回 client 得替每筆輸入配 id，協定要多一個欄位。agent 已經結束的情況，終端連線本來就會收到 `no_terminal … ended`（第 8 施工關 C7）。
  - **TUI 預設唯讀**。在終端畫面按 `i` 才進「輸入模式」：
    - 標題變 `g11-1 的終端 · 輸入中（Ctrl-] 離開）`，外框換顏色。
    - 輸入模式裡**每個鍵都送給 agent**，包括 `q`、`Esc`、`←`、`L`、`Ctrl-C`；只有 `Ctrl-]` 不送、用來離開。
    - 斷線、終端結束、離開這個畫面，都自動回到唯讀；重連後不會自己回到輸入模式。
    - 用 `Ctrl-]` 而不是 `Esc`：`Esc` 是 claude 的「中斷」鍵，要送得進去。
    - crossterm 有些終端把 `Ctrl-]`（0x1D）回報成 `Ctrl-5`，兩個都當「離開」。非美式鍵盤上 `Ctrl-]` 可能要別的按法（例如德式鍵盤），開工時在 macOS Terminal／iTerm2 實測，按不出來就另訂離開鍵。
  - 按鍵轉位元組：字元送 UTF-8，`Enter` 是 `\r`，`Backspace` 是 `0x7f`，方向鍵是 `ESC [ A`–`D`，`Ctrl-字母` 是控制碼。不開 bracketed paste，貼上的文字就是一串按鍵。
  - **backend**：TUI 不分 backend；由 daemon 決定。claude（與之後的 opencode）：寫進 PTY。codex：第 7 施工關 P1 預期「B 段開放後，人在 codex TUI 打字＝同一個 thread 的使用者訊息」，但 U17 未查證；**建議 daemon 對 codex 回 `not_supported`（訊息寫「等 U17 驗證」），直到第 7 施工關驗過 U17**。**與第 7 施工關 P1 的預期（B 段開放後 codex 也能打字）不同，請明確決定**。
- 理由：權限只在 daemon（D17），TUI 與 Telegram 共用；「預設唯讀＋要按 `i`」擋掉誤觸（在首頁習慣按的 `q`、`h`、數字不會跑進 agent）；codex 的打字路徑沒驗過，先關著比送出一則沒人知道去哪的訊息安全。
- 替代方案：終端畫面一開就能打字（v1 的方式，最容易誤觸）；逐行輸入、按 `Enter` 才送（安全，但 claude 這類全螢幕程式要的是原始按鍵，方向鍵、`Esc` 都送不進去）；codex 照第 7 施工關 P1 一起開放（U17 不成立時，打的字可能不進 thread 或讓 daemon 的忙閒判斷錯亂）。
- 例子：操作者在 `g11-1` 按 `i`、打 `hello` → 畫面出現 `hello`（PTY 回顯）；`AGEND_INSTANCE=g11-1 agend app` 裡同樣操作 → 底下 `forbidden: only the operator can type into an agent's terminal`、畫面沒有 `hello`。
- [ ] 使用者確認

### P7：斷線與重連（落實第 8 施工關 P4）

- 問題：接真 daemon 後，斷線、重連、落後、版本不合各怎麼處理？
- 建議：
  - `connect` ＝ `connect_once` ＋ `get_fleet` ＋ `subscribe_events(as_of)`。事件連線 EOF、`event_gap`（落後被關，第 8 施工關 P8）、請求連線斷掉 → 都進 A 段那個斷線畫面（T6 的畫面不變），每 500 ms 重連、`r` 立即重試。
  - 重連後重拿全貌，回到原本的畫面與選取；選取的東西已經不在 → 那個畫面的第一列。開著的終端重新訂閱；輸入模式不恢復（P6）。
  - 版本不合（第 8 施工關 P3）：斷線畫面顯示那段訊息，**不自動重試**（重試也不會好），`r` 仍可手動重試。
  - daemon 卡住但沒死（沒有心跳，第 8 施工關「本關不做」）：請求 10 秒沒回 → 當成斷線。
  - **接受：主 thread 會被卡住**。`connect_once` 與請求連線上的 `resolve_attention`、`answer_ask` 都在主 thread 等回覆，daemon 卡住時畫面最多凍結 10 秒（按鍵等回覆後才處理）。daemon 活著時回覆是毫秒級；要完全不凍結得把請求也搬到背景 thread、回覆變成非同步，本段不做。
- 理由：一條「斷了就重拿全貌」的路，daemon 重啟、TUI 落後、socket 被刪都走它（第 8 施工關 P4、P8）。這項不改已確認的東西，只是寫清楚 B 段怎麼做。
- 替代方案：沿用舊游標訂閱（第 8 施工關 P4 已排除）；版本不合也一直重試（畫面一直跳訊息，沒有用）。
- 例子：TUI 開在 `g11-1` 的終端 → 停 daemon → 斷線畫面、重試次數增加 → 起 daemon → `Reconnected to the daemon.`，回到 `g11-1` 的終端，`counter` 接著跑（同一個 holder）。
- [ ] 使用者確認

### B 段不做（明確列出）

- 把 TUI 視窗大小同步到 agent 的 PTY（resize）、滑鼠、bracketed paste、分割視窗。
- 已讀同步（第 12 施工關）；真 daemon 的請示（第 10 施工關）；codex 打字（U17 之後，P6）。
- 關卡種類、每個關卡的狀態、repo（G5，第 10 施工關）。
- 心跳（第 8 施工關本關不做）。

### 已知風險（開工時處理）

- 請求與事件走不同連線（P1）：`resolve_attention` 的 `accepted` 與 `attention_resolved` 誰先到不一定；TUI 只看事件決定消失，所以沒影響。
- 畫面更新節流 200 ms（P5）：從輸出到畫上去最多 300 ms 加一次來回（最後一段輸出靠記號補畫）；每個開著終端的 TUI 讓 holder 每秒最多多算 5 張畫面。開工時量。
- 輸入模式下的 `Ctrl-C` 會送進 agent（刻意的：操作者要能中斷 agent）；bash 假 agent 會因此結束、被 supervisor 重起。靠「預設唯讀、要按 `i`」防誤觸。
- 第 9 施工關與本段誰先 merge，決定 `agend app` 沒設 home 時的訊息字樣（P2）。
- 第 7 施工關若先 merge 且 U17 已驗證，P6 的 codex 限制照你的決定開放。

## 自動驗收（完成定義）

- [x] `~/.cargo/bin/cargo test -p agend-tui` 單獨通過（畫面層，2026-09-25）
- [x] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨（畫面層，2026-09-25）
- [x] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）（畫面層，2026-09-25）
- [x] `~/.cargo/bin/cargo xtask accept tui` 通過，並印出下方「你親自驗收」用到的 demo（畫面層，2026-09-25）
- [x] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新（畫面層）
- [x] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」（C-r2 CONFIRMED `8b545ab`）

B 段（P1–P7 確認後才開工；上面畫面層的項目在 B 段要重跑一次）：

- [ ] `~/.cargo/bin/cargo test -p agend-tui`、`-p agend-client`、`-p agend-daemon`、`-p agend` 單獨通過，包括：`ClientSource` 對假 daemon（全貌 → 畫面、`attention_resolved` 拿掉項目、`event_gap` 與 EOF 進斷線畫面、重連重拿全貌並回到原畫面）；`retry` 只在收到事件後消失（假 daemon `hold_resolved_events` 延後事件時，`accepted` 之後項目仍在）（P4）；只有終端連線 EOF 時只重連它、不進斷線畫面，開關終端 20 次後沒有多出來的 thread 與連線，停止的終端按 `i` 不進輸入模式（P1、P5）；終端收到位元組後 300 ms 加一次來回內換成新畫面、節流上限、重拿之後才到的最後一段輸出也會補畫（P5）；`terminal_input` 只走終端連線、它的錯誤不會變成 `retry` 的回覆；agent 回 `forbidden`（先查身分）、沒有活的終端回 `no_terminal`、操作者的位元組原樣到 holder、codex 回 `not_supported`（P6）；輸入模式只有 `Ctrl-]` 不送、斷線自動離開（P6）；`agend app` 沒設 home exit 2、非終端 exit 2（P2）
- [ ] `~/.cargo/bin/cargo test -p agend-testkit` 通過：假 daemon 的每個 instance 畫面、`terminal_bytes`、`terminal_input`（P1）；`CLP` 新列對假 daemon 與真 `agend daemon` 都通過，每列有 mutant
- [ ] `~/.cargo/bin/cargo xtask accept tui` 多印一段真 daemon（`agend daemon` 在暫存 home、`App` 經 `agend-client`），並對應下面 B 段步驟 1
- [ ] `cargo xtask check-deps` 最後一行照舊 `… no-std build ok)`（`agend-tui` 仍不依賴 SQLite、`agend-daemon`）
- [ ] 測試不留殘留：`pgrep -fl "agend (holder|daemon)"` 沒有輸出、`/tmp/g11.*` 沒有留下（驗收用 `mktemp -d /tmp/g11.XXXX`，測試也用 `/tmp/g11.` 開頭）
- [ ] fresh-context verifier 重跑 B 段並嘗試推翻

## 你親自驗收

由 agent 帶著走：一次一步，你貼輸出，agent 逐項比對。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。

### A. 畫面層（假資料；步驟本身已追認，T14）

在這個 PR 的 worktree（`~/Documents/Hack/AgEnD-v2-tui`）或 merge 後的 repo 根目錄跑。終端機至少 100×30。

1. 跑 demo。

   ```bash
   ~/.cargo/bin/cargo xtask accept tui 2>&1 | grep -E '^(==|tui demo|gate 11)'
   ```

   應該看到（第一次要編譯約 1 分鐘）：

   ```text
   == gate 11 (tui) == docs/gates/gate-11-tui.md
   == screens (fake daemon over its socket, 100x30)
   == navigate (scripted keys; each line: keys -> breadcrumb | selected row or first line)
   == resolve (read is not resolved; answering removes the item)
   == disconnect (stop the fake daemon while the TUI is open)
   tui demo: screens, navigation, resolve and disconnect checks passed
   gate 11 (tui): checks passed
   ```

   想看畫面本身：拿掉 `| grep ...`，每個畫面以 `   | ` 開頭印出，英文與繁中各一次。

   **這步在驗什麼**：四個畫面、按鍵、回答請示、斷線，全部透過 testkit 假 daemon 的真 socket（client protocol v1）跑，而且每段都有自動檢查。壞了的話，畫面層和協定之間的接縫就沒有證據，之後接真 daemon 會一次冒出很多問題。

   - [x] 通過

2. 開互動版，看首頁。

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-tui --example tui_fake -- --lang zh-TW
   ```

   應該看到：

   | 位置 | 要找的 |
   |---|---|
   | 第 3 行 | `━━ 需要你 · 3 ━━` |
   | 第一項（已選取，有 `›`） | `Regression suite fails 3 of 10 runs…`，右邊 `新  archfix · T-45` |
   | 下面 | `┏ archfix`、`┏ research`、`┏ general` 三個區塊，標題右邊是 agent 狀態數量 |
   | `general` 區塊 | `沒有進行中的目標` |

   選取的那一列從 `›` 到右邊反白，但左邊的 `▌`／`┃` 不反白。

   **這步在驗什麼**：attention-first 的版面（需要你在最上、依 team 分組、repo 不出現在首頁）和 DEMO-01 的選取樣式。壞了的話，你一打開就得自己找哪裡需要你。

   - [x] 通過

3. 已讀不等於已解決；回答後才消失（接著第 2 步的畫面）。

   操作：`Enter`（展開第一項）→ `↓` 三次（移到第二項）→ `h`（回首頁）

   應該看到：還是 `需要你 · 3`，但第一項右邊的 `新` 不見了（已讀）。

   操作：`Enter` → `1`

   應該看到：底下訊息 `已送出對 A-1 的回答：fixed seed 42`，清單變 `需要你 · 2 項待處理`，這一項不見了。

   操作：`F3`（假 agent dev-2 追問）→ `h`

   應該看到：`需要你 · 3`，第一項變成 `Seed 42 hides the flake. Also run 20 times nightly?`，右邊又有 `新`（追問是新的問題，算未讀）；`Enter` 展開後看得到 `你（tui）：fixed seed 42` 這段歷史。

   **這步在驗什麼**：D35 的請示是對話，已讀和已解決分開。壞了的話，只是看一眼就把事情「處理掉」，agent 會卡著等一個永遠不會來的回答。

   - [x] 通過

4. `t` 看 agent 終端、`←`／`→` 上下層、`/` 搜尋、`L` 切語言。

   操作與應該看到：

   | 操作 | 應該看到 |
   |---|---|
   | `h`，`↓` 移到 `● Fix lock-order inversion`，按 `t` | `dev-1 的終端 · 唯讀快照`，內容有 `cargo test --workspace` |
   | `←` | 回首頁，選取還在同一列 |
   | `↑` 移到 `┏ archfix`，按 `→` | team 頁，第一行 `AgEnD › archfix`，`[1 目標]` |
   | `2`，`↓` 兩次到 `qa-1`，按 `t` | 底下訊息 `qa-1 沒有終端輸出。`，畫面不變 |
   | `←` | 回首頁，`┏ archfix` 仍是選取 |
   | `/`，打 `rev`，`Enter` | `reviewer-1 的終端`；`←` 回到按 `/` 的地方 |
   | `L` | 同一個畫面換成英文（`L 中文`）；再按一次換回來 |
   | `q` | 離開，終端機恢復正常 |

   **這步在驗什麼**：DEMO-01 五輪決定的導覽：`←`／`→` 永遠是上一層／下一層、`t` 從任何有 agent 的列直接開終端、`/` 是覆蓋層不是一層、`L` 不改狀態。壞了的話，你會在畫面之間迷路。

   - [x] 通過

5. 故意弄壞：TUI 開著時停掉假 daemon（真的 socket）。

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-tui --example tui_fake -- --daemon
   ```

   操作：`↓` 三次、`→`（進 archfix team 頁）→ `F2`（停掉假 daemon）

   應該看到：`━━ Daemon disconnected`、`Lost the connection to the daemon: the daemon closed the connection`、`Reconnect attempt N failed: cannot connect to …`（N 會增加），沒有舊資料，程式沒有當掉。

   操作：`F2`（啟動新的假 daemon）

   應該看到：一秒內 `Reconnected to the daemon.`，畫面回到 `AgEnD › archfix`。`q` 離開。

   **這步在驗什麼**：daemon 不在時 TUI 說清楚發生什麼事、自己重試，daemon 回來後回到原本的畫面。壞了的話，daemon 重啟一次 TUI 就當掉或顯示過期資料（v1 的設定錯誤都是「靜靜不動」）。

   - [x] 通過

### B. 接真 daemon（步驟待 P1–P7 確認；應該看到的字樣是設計，實作後改成實跑輸出）

用三個終端機分頁：**第一個跑 daemon**，**第二個跑 `agend app`**，第三個只在步驟 5 用。步驟 2 起用同一個暫存 home。

**每個新開的終端機分頁都要先跑這段**。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
unset AGEND_BIN               # 前幾關步驟留下的 export 可能指到已刪除的 worktree
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.0.0`。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。第二、第三個分頁還要貼上步驟 2 印出的那行 `export AGEND_HOME=…`。

1. 跑 demo。

   **這步在驗什麼**：真的 `Source`（`ClientSource`，P1）對假 daemon 與真 `agend daemon` 都畫出正確畫面；`retry`、終端即時更新、打字權限、重連這幾段有自動檢查。壞了的話，後面幾步你看到的問題分不出是 TUI 還是 daemon 的。

   ```bash
   ~/.cargo/bin/cargo xtask accept tui 2>&1 | grep -E '^(==|tui demo|gate 11)'
   ```

   應該看到：A 段第 1 步那幾行，再多出 `== real daemon …`、`== retry …`、`== terminal …`、`== input …` 幾段（確切字樣開工時細化），最後 `gate 11 (tui): checks passed`。

   - [ ] 通過

2. 起 daemon（一個正常、一個一起來就死的假 agent），開 `agend app`。

   **這步在驗什麼**：`agend app` 找得到 daemon（沒設 home 就說清楚），首頁的資料來自真 daemon 的全貌與事件：`g11-2` 放棄後「需要你」**自己**多一項，不用按鍵（P2、P3）。壞了的話，A 段只證明了假資料，真的用起來首頁是空的或不會更新。

   第一個終端：

   ```bash
   export AGEND_HOME="$(mktemp -d /tmp/g11.XXXX)" && echo "export AGEND_HOME=$AGEND_HOME"
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- add g11-1
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- add g11-2 --dies
   agend daemon
   ```

   應該看到：`added g11-1 …`、`added g11-2 …`，然後 `listening on /tmp/g11.…/run/daemon.sock`、`agend daemon ready: instances=2 …`（daemon 留在前景）。

   第二個終端（先跑開頭那段、貼上 `export AGEND_HOME=…`）：

   ```bash
   env -u AGEND_HOME agend app; echo "exit=$?"
   agend app --lang zh-TW
   ```

   應該看到：

   | 時間點 | 要找的 |
   |---|---|
   | 第一行指令 | `AGEND_HOME is not set…`、`exit=2`（沒有進全螢幕） |
   | 剛開 | 首頁、`┏ general` 區塊寫 `沒有進行中的目標` |
   | 約 15 秒內（不要按鍵） | 最上面出現 `━━ 需要你 · 1 ━━`，那一項是 `g11-2`，右邊有 `新` |

   - [ ] 通過

3. 處理「需要你」：看過不會消失，按 `重試` 才消失（接著第 2 步的畫面）。

   **這步在驗什麼**：已讀和已解決分開；按了之後是 daemon 決定它解決（收到 `attention_resolved` 才消失）（P4；第 8 施工關 P6 建議本步改用 `failed` 的 `retry`，因為真 daemon 還沒有請示）。壞了的話，你以為處理了，agent 其實還停著。

   操作：`Enter`（展開 `g11-2`）→ `h`（回首頁）

   應該看到：展開時有 `不處理的話：g11-2 stays stopped` 與一列 `1 重試`；回首頁後還是 `需要你 · 1`，但 `新` 不見了（已讀）。

   操作：`Enter` → `1`

   應該看到：底下 `已送出：重試 g11-2`，清單變 `需要你 · 0`；第一個終端的 daemon 印 `g11-2: retry requested by the operator`、`g11-2: start …`。`--dies` 的 agent 還是會死：約 15 秒後它又回到「需要你」、又有 `新`，這是正常的。

   - [ ] 通過

4. `t` 看即時終端、`i` 打字、`Ctrl-]` 離開；導覽照舊。

   **這步在驗什麼**：`t` 看到的是真 agent 的即時畫面，不是快照（P5）；打字要先按 `i`，平常的鍵不會跑進 agent（P6）；接真 daemon 後 `←`、`/`、`L` 行為沒變。壞了的話，你照過期畫面做決定，或一個 `q` 就打進 agent 裡。

   | 操作 | 應該看到 |
   |---|---|
   | `/`，打 `g11-1`，`Enter` | `g11-1 的終端 · 即時`，內容有 `counter=…`；**不按鍵**，數字每秒加一 |
   | `L` | 同一個畫面換成英文（`Terminal of g11-1 · live` 之類），畫面上沒有多出 `L`；再按一次換回來 |
   | `i` | 標題變 `g11-1 的終端 · 輸入中（Ctrl-] 離開）`、外框換顏色 |
   | 打 `hello`（**不要按 `Ctrl-C`**：會送進 agent、讓它結束再被重起） | 畫面上 `counter=…` 之間出現 `hello`（PTY 回顯） |
   | `Ctrl-]` | 標題回到 `· 即時` |
   | `←` | 回到按 `/` 的地方 |
   | `q` | 離開，終端機恢復正常 |

   - [ ] 通過

5. 故意弄壞：假裝是 agent 來按。

   **這步在驗什麼**：只有操作者能打字、能按 `重試`；擋在 daemon，不是 TUI 自己藏按鈕（P2、P4、P6，D17）。壞了的話，agent 照說明跑錯命令就能在別的 agent 終端打字。

   第二個終端：

   ```bash
   AGEND_INSTANCE=g11-1 agend app --lang zh-TW
   ```

   | 操作 | 應該看到 |
   |---|---|
   | `/`，打 `g11-1`，`Enter`，`i`，打 `x` | 底下 `forbidden: only the operator can type into an agent's terminal`，回到唯讀，畫面沒有 `x` |
   | `h`；如果 `g11-2` 在「需要你」：`Enter` → `1` | 底下 `forbidden: only the operator can resolve needs-you items; …`，項目還在 |
   | `q` | 離開 |

   - [ ] 通過

6. 故意弄壞：TUI 開著時重啟 daemon。

   **這步在驗什麼**：daemon 停掉時 TUI 說清楚、自己重試；回來後**重拿全貌**、回到原本的畫面，終端接著跑（P7；第 8 施工關 P4）。壞了的話，daemon 重啟一次 TUI 就當掉，或顯示過期資料。

   第二個終端：`agend app`（英文，對照 A 段第 5 步的字樣），`/`，打 `g11-1`，`Enter`。

   操作：第一個終端 Ctrl-C。

   應該看到（第二個終端）：`━━ Daemon disconnected`、`Lost the connection to the daemon: the daemon closed the connection`、`Reconnect attempt N failed: …`（N 會增加），沒有舊資料，程式沒有當掉。

   操作：第一個終端 `agend daemon`。

   應該看到：一秒內 `Reconnected to the daemon.`，回到 `Terminal of g11-1 · live`，`counter` 接著原本的數字跑（同一個 holder）；`←` 回首頁，`g11-2` 那一項還在（它在 DB 裡是 `failed`，由全貌帶回來，不是重播）。`q` 離開。

   - [ ] 通過

7. 收尾。

   **這步在驗什麼**：本段的 instance 在下次開機被收掉，什麼都不留（第 6、8 施工關的孤兒巡查）。壞了的話會留下 holder 在背景跑。

   第一個終端 Ctrl-C 停 daemon，然後：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- remove g11-1
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- remove g11-2
   agend daemon
   ```

   應該看到：`removed g11-1`、`removed g11-2`；daemon 印 `orphan g11-1: Shutdown sent`、`orphan g11-2: Shutdown sent`、`agend daemon ready: instances=0 … orphans=2`。

   第二個終端：

   ```bash
   pgrep -fl "agend holder g11-"; echo "pgrep exit=$?"
   ```

   應該看到只有 `pgrep exit=1`。最後第一個終端 Ctrl-C，再 `rm -rf "$AGEND_HOME"`。

   - [ ] 通過

## 待你追認

畫面層提前時由實作者決定、可以反悔的事。每項：決定 · 理由 · 反悔的成本 · 追認結果。G 開頭的是給第 8 施工關的協定缺口（這個 PR 沒有改 core、沒有發明 wire 訊息）。

| # | 決定 | 理由 | 反悔成本 | 追認結果 |
|---|---|---|---|---|
| T1 | 資料來源接縫是 trait `source::Source`（`connect`／`poll`／`terminal`／`answer`）；畫面只讀 `source::Fleet`，`Fleet` 只由 catalog + client protocol v1 事件（`EventData`）重建 | 畫面不知道資料從哪來；腳本假來源、假 daemon、之後的真 daemon 走同一條路 | 換介面：`source.rs` 與兩個實作 | 已追認（2026-09-26） |
| T2 | 兩個實作：`source::scripted::ScriptedSource`（lib 內，給測試與 demo，回答規則同 testkit 假 daemon）；假 daemon 的 socket client 放 `examples/support/daemon_source.rs`（dev 專用，examples 與 tests 用 `#[path]` 共用），不放 lib、不放 `agend-client` | lib 不該有 socket 程式碼（「只透過 agend-client」）；`agend-client` 的連線與重試是第 8 施工關的範圍，現在寫會先替它做決定 | 搬進 `agend-client`：第 8 施工關本來就要做 | 已追認（2026-09-26） |
| T3 | 「需要你」的定義：ask 的最後一筆是提問或追問才算；回答後離開、agent 追問再回來；有結論也離開 | D35 的請示是對話；「已解決」要看 daemon 回的 `ask_updated`，不是 TUI 自己決定 | 改 `Attention::waiting` 一處 | 已追認（2026-09-26） |
| T4 | 已讀是 TUI 本機狀態：展開過的項目去掉粗體和 `new`；重開 TUI 會回到未讀，Telegram 不同步 | protocol v1 沒有已讀狀態（見 G4） | 協定加欄位後改讀 daemon 的值 | 已追認（2026-09-26） |
| T5 | 回答方式：選項是可選取的列，`→`／`Enter` 或 `1`–`9` 選；`a` 開一行自由文字；回答後選取移到下一項 | D35 要能自由回答；DEMO-01 的 `Tab` 切焦點改成直接把選項當列，少一個模式 | 改 `attention.rs` 與 `App::key` | 已追認（2026-09-26） |
| T6 | 斷線時整個內容換成斷線說明（原因、第幾次重連失敗、狀態都在 daemon），不顯示舊資料；每 500 ms 自動重連，`r` 立即重試；重連後重播事件、回到原本畫面與選取 | 舊資料會讓人以為還在更新；自動重連不用你動手 | 改 `ui.rs` 的斷線分支 | 已追認（2026-09-26） |
| T7 | attach 只顯示 `terminal_snapshot` 唯讀快照；即時串流（`terminal_bytes`）與輸入（`terminal_input`）留到第 11 施工關正式接 | 假 daemon 只回一張快照；輸入要身分與權限，等真 daemon | 之後加串流與輸入，畫面結構不變 | 已追認（2026-09-26） |
| T8 | testkit 加 `FakeDaemon::open_ask(thread, recap)`：建立帶 task 與脈絡摘要的請示，可以 `answer_ask` | 假 daemon 的 `ask` 命令建立的請示 `task_id` 永遠是 None、沒有 recap，畫面無法依 team 分組 | 移除一個方法與一個測試 | 已追認（2026-09-26） |
| T9 | `check-deps` 加 agend-tui 規則：不可依賴 SQLite 與 agend-daemon；不擋 async runtime | TUI 是 protocol client（D11）；crossterm 會帶進 `mio`，擋 runtime 會誤擋 | 改 `RULES` 一筆 | 已追認（2026-09-26） |
| T10 | 依賴：`ratatui 0.30`（關掉預設功能，只開 `crossterm` + `std`）與 `unicode-width 0.2`；crossterm 用 ratatui 的 re-export；dev 依賴 `agend-testkit`、`serde_json` | 與 DEMO-01 同版本；關掉預設功能少 87 個 crate | 改 `Cargo.toml` | 已追認（2026-09-26） |
| T11 | 文字：一張表（`i18n::Text` enum），不用 i18n crate；daemon 來的標題、問題、摘要不翻譯；繁中用詞沿用 DEMO-01，但依名詞表把「步驟」改「關卡」、「流程」改「流水線」 | 與名詞表一致；資料翻譯要由 daemon 或 agent 做 | 改 `i18n.rs` | 已追認（2026-09-26） |
| T12 | 與 DEMO-01 的差異：不做滑鼠；team 區塊的「最近變更」列不能選（同一個 task 已有目標列）；流水線 tab 是依關卡種類分組的清單，不是欄位；team 頁切 tab 時選取回到第一列；`/` 用子字串比對；沒有 `Space`（下個模擬事件）與 `r`（重設），`r` 改成斷線時立即重試；終端標題是「唯讀快照」 | KISS：先做鍵盤與畫面；滑鼠與欄位版面等你用過再決定 | 各自在對應模組補回 | 已追認（2026-09-26） |
| T13 | 首頁「需要你」列 `→` 開「需要你」畫面並展開該項；Task Detail 只有目前關卡 `→` 會開它的請示；`t` 在請示列開提問的 agent，非請示項目開 task 持有者 | 同 DEMO-01 的層級；「誰問的」比「誰持有」更接近要看的終端 | 改 `App::open_selected` | 已追認（2026-09-26） |
| T14 | 你親自驗收 A 段的步驟本身（上面 A1–A5，含 `F2`／`F3` 這兩個只在 `tui_fake` 範例有的 demo 鍵） | 你要求先寫定步驟再驗 | 改這一頁 | 已追認（2026-09-26） |
| T15 | 想改但依規則沒改的文件（列給你決定）：名詞表加「資料來源（data source，`source::Source`）」與「目錄（catalog，`source::Catalog`）」；AGENTS.md 的 crate 邊界表加一列「`agend-tui` 不依賴 SQLite、`agend-daemon`」；ROADMAP 與 docs/gates/README.md 的狀態欄 | 這次的規則不准改 ROADMAP、gates/README、GLOSSARY、AGENTS、DECISIONS | 另一個 docs PR | 已追認（2026-09-26） |
| T16 | 「需要你」展開後，在「來自 … · 任務 … · team …」下一行加「不處理的話：…」（DEMO-01 §4B 第 3 點）；`attention_required` 沒有這個欄位，所以放在 `Catalog.if_ignored`（以 task id 為鍵），demo catalog 給三句固定文字 | DEMO 規格要讓人一眼看懂不處理會怎樣；不在 TUI 自己推算 | 第 8 施工關加欄位後改讀協定；刪一個欄位與一行 | 已追認（2026-09-26） |
| T17 | 已讀記在「哪一題」上（`Attention::read_key` = 請示 id + 問題數）：agent 追問後這一項回到未讀（粗體、`新`） | 追問是新的問題，你還沒看過；只記請示 id 會讓追問靜靜回來，容易漏看 | 改 `read_key` 一處（只用請示 id 就回到舊行為） | 已追認（2026-09-26） |
| T18 | 「需要你」數量與 agent 的「需要你」狀態跟著目前的需要你清單重算（`Fleet::agent_state`）：有等待中的項目指向這個 agent（提問者，否則 task 持有者）就是「需要你」；catalog 說「需要你」但已經沒有等待中的項目時，手上有沒做完的 task 算「工作中」，否則「閒置」；其他狀態照 catalog | 回答 A-1 後 archfix 標題還寫 `! 1 needs you` 會讓人以為還有事；DEMO-01 也是從 task 推算數量。工作中／閒置／卡住等其他狀態協定沒給，照舊是 G1 缺口，不在 TUI 推算 | 改 `Fleet::agent_state` 一處；第 8 施工關協定給 agent 狀態後改讀協定的值 | 已追認（2026-09-26） |
| G1 | 缺口：沒有列出 team、task、agent 的請求，也沒有 task 的關卡清單與狀態、agent 的結構化狀態（working／idle／needs you／stuck／unknown）與 backend，也沒有「不處理的話會怎樣」（見 T16）；agent 狀態只有「需要你」由 TUI 依需要你清單重算（見 T18），其餘照 catalog，事件發生後不會更新 → 現在由 `source::Catalog`（TUI 本地型別）在 connect 時給 | 畫面要分組、要畫 `■□` 進度條、要顯示 agent 狀態，只有 `task_changed`／`instance_changed` 的文字摘要不夠 | 第 8 施工關：加 list 請求（或快照事件），`Catalog` 改用 core 型別 | 已追認（2026-09-26） |
| G2 | 缺口：`attention_required` 沒有「解決後能放行多少工作」與「開始等待的時間」→ D36 排序時 unblocks 一律 0、用 event id 代替等待時間（最舊的在前） | 不在 TUI 自己推算 | 第 8 施工關：`AttentionRequiredData` 加兩個欄位（additive） | 已追認（2026-09-26） |
| G3 | 缺口：不是請示的「需要你」（agent 卡住、用量上限）沒有操作，也沒有「已處理」事件 → 畫面寫明「client protocol v1 還沒有處理這一項的操作」，而且一直留在清單 | 不發明 wire 訊息 | 第 8 施工關：加操作（重試、暫停…）與清除事件 | 已追認（2026-09-26） |
| G4 | 缺口：沒有已讀狀態 → 見 T4 | 同上 | 第 8 施工關：加已讀請求與事件，TUI 與 Telegram 共用 | 已追認（2026-09-26） |

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
| 2026-09-26 | A 段通過 | 在 `feat/gate-11-tui-screens`（merge 前）由 agent 帶著走 5 步（截圖比對）。步驟 2 第一次截圖前已按過鍵，重開後對上。步驟 3：展開過的變已讀、回答後消失、追問回來並看得到歷史；F3 按了兩次所以追問出現兩行（每按一次送一次，demo 行為）；追問後首頁的 `新` 沒有另外截圖（使用者選擇跳過）。步驟 4 全對。步驟 5：斷線畫面重試到第 39 次、F2 後 `Reconnected to the daemon.` 回到 `AgEnD › archfix`。B 段等第 8 施工關。 |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-27 第 3 輪 review REFUTED（1 MEDIUM、3 LOW）後修正：`Sender::close()`（`shutdown(Both)` 叫醒讀的 thread）與終端連線的開、關、重連步驟，`agend-client` 改成加 4 個 API；停止的終端不能按 `i`、instance 又跑起來時自動重訂；假 daemon 對沒登記的 instance 回 `no_terminal`、codex 檢查排在記錄位元組之前；重訂失敗時 daemon 清掉舊串流。
- 2026-09-27 第 2 輪 review REFUTED（1 HIGH、2 MEDIUM、3 LOW）後修正：終端連線的寫入改用 `Client::sender()`（try_clone 的寫入端，讀的 thread 阻塞時主 thread 照樣能寫），`agend-client` 改成加 3 個 API；延遲寫成「最多 300 ms 加一次來回」；假 daemon 加 `hold_resolved_events`；終端連線上錯誤碼的畫面規則、只有終端連線 EOF 時只重連它；標出與 T1、T18 不同之處。
- 2026-09-27 fresh review REFUTED（1 HIGH、3 MEDIUM、4 LOW）後修正：`terminal_input` 改走終端連線（不帶 id 的錯誤不再變成 `retry` 的回覆）；節流加「有新輸出」記號補畫最後一段；本段加改 testkit 假 daemon 與 `CLP` 新列；標出與 T12、T13 不同之處、A 段 demo 保留 `ScriptedSource`、轉換放 `ClientSource`；holder 輸入錯誤接受靜靜丟掉並記 log；`Ctrl-]`／`Ctrl-5`；接受主 thread 最多凍結 10 秒；`/tmp/g11.` 路徑與 `agend app` exit 2。
- 2026-09-27 B 段開工前提案 P1–P7 寫定（draft PR，branch `feat/gate-11-tui-daemon`），待使用者確認：`ClientSource` 放 lib（與 T2 不同）、`agend app` 由本段做、`Catalog` 只由全貌與事件填（新缺口 G5 給第 10 施工關）、`retry` 等事件才消失、終端以位元組為訊號重拿 holder 畫面、`i` 進輸入模式且 codex 先不開放（與第 7 施工關 P1 的預期不同）、重連重拿全貌；「你親自驗收」B 段改成 7 步確切指令。
- 2026-09-26 使用者親自驗收 A 段 5 步通過；T1–T18、G1–G4 使用者全部追認（看過畫面後一次追認）。
- 2026-09-26 驗證報告 C-r1 修正：team 標題的 `─` 線不反白；底部說明只列有用的鍵；需要你展開加「不處理的話」（T16）；追問回到未讀（T17）；標題數量與 agent 的「需要你」跟著清單重算（T18）；B1–B5 補「這步在驗什麼」（`feat/gate-11-tui-screens`，PR #120）。
- 2026-09-25 畫面層提前（你同意並行、放寬 D22）：`Source` 接縫 + 腳本假來源 + testkit 假 daemon socket 來源（demo 用）；首頁、需要你、team 頁、Task／Agent Detail、終端快照、`/`、`L`、斷線與重連；`cargo xtask accept tui` demo；check-deps 加 agend-tui 規則；testkit 加 `FakeDaemon::open_ask`（`feat/gate-11-tui-screens`，draft PR）。

## 下一步

```bash
~/.cargo/bin/cargo xtask accept tui
```
