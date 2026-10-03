# 假 daemon

> **TL;DR**
> - FakeDaemon 是測試用 Unix socket server；預設保留 1.3，安裝真 producer 後提供 1.4。
> - C 路徑每 instance 有界 queue、每 socket view／attach，操作完成才回 ack；producer 自己負責真 parser 與實際操作。
> - 下一步：用 `set_terminal_producer` 注入實作；跑 `cargo test -p agend-testkit --test full_terminal`。

## 假 daemon

- `FakeDaemon::start()`：`<tmp>/agend-test-fd-*/daemon.sock`；`FakeDaemon::start_at(path)`：指定路徑（在同一個路徑「重啟」）。drop 時停止接受連線、關閉所有已開的連線（client 讀到 EOF）、刪除 socket。
- 第一行必須是 `hello`；其他請求或無效 JSON 都回 `hello_required` 並關閉；hello 之後的無效 JSON 回 `invalid_request`，連線不關；major 不合回 `version_mismatch` 並關閉；`hello` 帶 `caller` 就是 agent 的連線。錯誤碼一律用 core 的 `client::error_code`。
- 全貌：`get_fleet` 回 `set_instance`、`set_task`、`add_attention`、請示組成的全貌（team 至少有 `general`），`as_of_event_id` 是最新的事件 id；`fleet()` 給測試看同一份。
- 事件：id 從「啟動時間 unix ms × 1000」+ 1 開始（`event_id_start()`），留最近 1024 筆；游標規則與真 daemon 相同（不帶游標重播全部、「最舊 − 1」到最新接得上、其他 `event_gap`）；落後超過 1024 筆 → `event_gap` 後關連線；寫入 5 秒沒進度 → 關連線。一個請求造成的事件在它的回應之後才送出（跟真 daemon 一樣）。
- `resolve_attention`：agent → `forbidden`（先於 id）；沒有這個 id 或操作不在 `actions` → `unknown_attention`；成功 → 項目消失、`attention_resolved`、`accepted`。`hold_resolved_events(true)` 時事件等 `release_resolved_events()` 才發（第 11 施工關 B 段 P4：測「收到事件才消失」）。
- 終端（第 11 施工關 B 段 P1）：`subscribe_terminal` 只對全貌裡的 instance 回畫面（`set_screen`；沒設是 `fake screen of <id>`），之後每次 `push_terminal_bytes(id, bytes)` 送 `terminal_bytes` 並把文字接到畫面後面（下次訂閱看得到）；不存在的 id → `no_terminal`（改掉第 8 施工關 C2）。同一條連線再訂一次取代舊的，失敗也一樣。落後 256 塊就關連線；`drop_terminal_subscribers()` 直接這樣關（測只重連終端）。
- 打字（P6）：`terminal_input` 依序 agent → `forbidden`、不存在或 `failed` 的 instance → `no_terminal`、codex → `not_supported`、轉成 holder 請求行超過 1 MiB（含換行）→ `invalid_request`；base64 解不開 → 不回應、不記（跟 daemon 一樣，它不解 base64）、codex → `not_supported`，都不帶 request id；其他記下（`terminal_inputs()`），不回應。訊息常數 `TYPE_OPERATOR_ONLY`、`CODEX_INPUT` 跟真 daemon 一字不差。
- `open_connections()`：正在服務的連線數（TUI 測試查沒有留下連線）。
- 事件身分：`assign(task, ResultIdentity)` 設定目前要的結果；`done`／`result`／`review_*` 沒帶或不符 → `stale_result`、什麼都不變；接受後這個 attempt 就用掉了。
- 第 9 施工關：`hello` 回 `daemon_version`（`agend <版本> (fake daemon)`）、`daemon_pid`（測試程序）、`boot_id`（事件 id 起點）。`command` 只收 agent、`operator` 只收操作者（`forbidden` 的訊息跟真 daemon 一字不差）。`status`：`set_status` 設了就用它，否則照真 daemon 的格式（`g9-a (claude): no task` ＋ `next: …`；呼叫者不在 instance 裡 → `unknown_instance`），有 `assign` 時帶 task 與 `identity`。`send`／`inbox` 跟真 daemon 同規則（`message_id` 要 UUID v4、同 id 同內容再送照樣 `accepted` 只存一則、不同內容 `invalid_request`；收件者不存在 `unknown_instance`；`inbox` 最近 20 則或 `--after` 之後全部，沒有那一則 → `unknown_message`；`message_ids_to(name)` 給測試看）。`instance_add`／`instance_remove` 改全貌（`instance_exists`、`unknown_instance`、名字規則；`working_directory` 預設 `<home>/workspace/<name>`，home 是 `…/run/daemon.sock` 往上兩層）。`daemon_restart`：跑 `<binary> --version`，要印 `agend …` 才算過（否則 `preflight_failed`，訊息格式同真 daemon）；過了回 `restarting`、關掉所有連線、換新的 `boot_id`（事件清空）。`task_cancel` 對不存在的 task 回 `invalid_request`。
- 其他：`task_create`、`ask`、`answer_ask`、`block`／`unblock`／`remind`（真 daemon 已由 pipeline queue 處理，假 daemon 保留 client 契約所需的簡化狀態）。
- `open_ask(thread, recap)`：像綁定 task 的 agent 跑 `agend ask` 那樣建立請示（帶 task 與脈絡摘要），可以 `answer_ask`，也列在全貌的「需要你」裡（`attention_id` = ask id）；`ask` 命令建立的請示沒有 task（TUI 的 demo 與測試用）。
- `ProbeClient::hello(path, caller)`、`recv_within(timeout)`：契約的驅動端（逾時不丟掉讀到一半的行）。
## C 段

`set_terminal_producer(instance, producer)` 安裝 core 的同步 `TerminalProducer` 並提供 1.4；這個 port 提供 frame、完成後 control ack 與 legacy input。normal testkit 不依賴 holder 或建立第二套 parser；測試中的 Screen 只透過 dev-dependency 引入。未安裝 producer 的舊 fake 保持 1.3，`set_supported_versions` 只改協商，不能建立畫面。

每 instance 64 個有序工作；reader 不等 I/O，8 個有序 replies 加一份合併 frame。view／attach 綁 socket，每次 Acquire 新 token；最後 Acquire 控制，前 owner 唯讀。控制、viewport、EOF、generation、failed／removed instance 都核對；EOF 保留尺寸，不恢復舊 grant。producer 在實際執行點重驗 owner／generation，沒有在途重送。超限 client frame 整份拒絕並關閉，請求 1 MiB 整次拒絕。

`ProbeClient::writer_clone` 供 native 大寫入 fixture 併行讀寫；JSON Lines 由 core serializer 產生。完整共用規則見 [CLIENT-CONTRACTS.md](CLIENT-CONTRACTS.md)。FakeDaemon 沒有 live driver 與 durable thread 歸屬核准，因此 Codex Acquire／Input 及 legacy 輸入仍 not_supported；不是 production 版本開放狀態。正常 daemon 已依 [版本政策](../../docs/gates/gate-11c-codex-input.md) 開放 0.159.3，U17 suites 使用真 daemon／holder 與 fake Codex producer，另與首次真模型證據分開核對。

## 下一步

```bash
cargo test -p agend-testkit --test full_terminal --test full_terminal_pressure --test full_terminal_fds
```
