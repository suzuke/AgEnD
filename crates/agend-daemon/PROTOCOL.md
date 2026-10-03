# Daemon protocol server

> **TL;DR**
> - 真 daemon 現在協商 client 1.4；一般 client 的 NEEDED 仍為 1.3，以下第 8／11B 規則保留。
> - 新終端畫面與控制在 [TERMINAL.md](TERMINAL.md)；FakeDaemon 安裝真 TerminalProducer 後提供 1.4，未安裝的舊 fixture 保持 1.3。
> - 下一步：protocol 回歸用 `cargo test -p agend --test client_protocol --test terminal_capability`。

## client protocol server（第 8 施工關）

下表「終端」與「打字」保留第 11 B 段原規則，用來定位舊 wire 與限制。現行 C 路徑由 [TerminalHub](TERMINAL.md) 管理 view／控制、resize 與有序輸入；Codex 已依 [版本政策](../../docs/gates/gate-11c-codex-input.md) 只開放 0.159.3，不再一律拒絕。legacy input 也須經相同核准，有 C owner 時拒絕無 attach 的輸入。

| 項目 | 內容 |
|---|---|
| socket | `$AGEND_HOME/run/daemon.sock`：`run/` 0700、socket 0600；路徑超過 100 bytes 開機就拒絕（`socket path too long: … (… bytes, max 100 bytes); use a shorter AGEND_HOME`，exit 1，不碰 `agend.db`）；拿到 DB 鎖後刪掉舊的 socket 檔 |
| 何時出現 | 開機計畫做完才 bind，接著印 `listening on …` 與 `agend daemon ready: …`；連得上的 client 一定看到完整的 instance 清單（不需要 `.ready`） |
| 停止 | Ctrl-C／SIGTERM：停止接受、刪 socket 檔、關所有 client 連線，再照第 6 施工關結束 |
| 身分 | `hello` 的 `caller`（CLI 在 agent 裡填 `AGEND_INSTANCE`）：有填＝agent，沒填＝操作者；不做 cookie |
| 請求 | `hello`、`get_fleet`、`subscribe_events`、`subscribe_terminal`、`resolve_attention`（只收操作者，先查身分再找 id；pipeline 人工核准先提交 CAS，再解除項目並發布真 action；instance 的 `retry` 交給 supervisor）；`terminal_input`（第 11 施工關 B 段，見下）；`answer_ask` → `unknown_ask`；agent 命令與 `operator` 見「CLI 的 daemon 端」；未知請求 → `unknown_request`、連線不斷 |
| 事件 id | 第一個＝開機時間（unix ms）× 1000 + 1；只放記憶體最近 1024 筆；游標規則見 `agend_core::protocol::client` |
| 慢 client | 落後超過 1024 筆 → `event_gap` 後關連線；寫入 5 秒沒進度 → 關連線；都記一行 log（`client #N (…): …`） |
| instance 狀態 | `starting`（啟動中、等重起）、`unknown`（在跑；忙碌／閒置要 driver）、`failed` |
| 需要你 | `failed` 的 instance → `instance-failed:<id>`（等待時間＝這個 daemon 第一次看到它 `failed`）；`retry`：先 `Shutdown` 留著的 holder，session 建立過就 `running` + `--resume`（claude），沒建立過就 `new`（claude `--session-id`、codex／opencode 全新啟動）；codex／opencode 建立過 session 的沒有操作 |
| 終端 | 在跑的 instance：先回 holder 當下畫面、再轉送之後的 `terminal_bytes`（經 daemon 的長連線，client 不直接連 holder）；`failed` 且 holder 還在：短連一次、只回最後畫面；其他 → `no_terminal`。同一條連線再訂一次：先清掉舊的串流，失敗就沒有串流（第 11 施工關 B 段 P1） |
| 打字 | `terminal_input`（第 11 施工關 B 段 P6）依序：agent → `forbidden: only the operator can type into an agent's terminal`；沒有活的終端（不存在、`failed`、沒有長連線）→ `no_terminal`；當時 codex → `not_supported`（等 U17 驗證；現行版本核准見上方）；其他經長連線轉成 holder 的 `OperatorTerminalInput`，不回應。錯誤都不帶 `request_id`。holder 拒絕（`pty_busy`、`agent_exited`）只記 log：`<id>: operator input dropped: <code>`；寫給 holder 的請求：每條 link 一把寫入鎖包住一整行（不拿 links 表的鎖）、5 秒沒進展就放棄並關掉那條連線（`link::WRITE_WITHIN`，link 會重連）；轉成 holder 請求行超過 1 MiB（`protocol::holder::MAX_REQUEST_LINE`）的輸入先回 `invalid_request` |


## CLI 的 daemon 端（第 9 施工關）

| 項目 | 內容 |
|---|---|
| 協定 | client protocol 1.2：`operator` 請求（`instance_add`、`instance_remove`、`daemon_restart`、`task_cancel`）與結果 `instance_added`、`restarting`；`send` 加 `level`、`message_id`；`status` 加 `identity`；`hello` 回 `daemon_version`、`daemon_pid`、`boot_id`（＝事件 id 起點，每次開機都變）；全貌的 instance 加 `working_directory` |
| 權限 | `command`：操作者送 → `forbidden: agend done is an agent command; it runs inside an agent, where AGEND_INSTANCE is set`；`operator`：agent 送 → `forbidden: only the operator can add instances; ask the operator`（依請求換字） |
| `status` | 呼叫者自己的 instance：`g9-1 (claude): no task` ＋ `next: …`；DB 沒有這個 instance → `unknown_instance` |
| `send` | 收件者要存在（否則 `unknown_instance`）；`message_id` 要 UUID v4（否則 `invalid_request`），沒帶就由 daemon 產生；交給 `CodexDriver::deliver`（`messages` 表：同 id 同內容 → 照樣 `accepted`、不再送；同 id 不同內容 → `invalid_request: message id … is already used by another message`）；claude／opencode 收件者停在 `queued`；寫入 DB（codex 再加上交給長連線）之後才回 `accepted` |
| 大小上限 | `send` 的 body 最多 1 MiB（`MAX_MESSAGE_BYTES`，超過 `invalid_request`，什麼都不存）；協定一行最多 8 MiB（`MAX_LINE_BYTES`，超過回 `invalid_request` 並關連線，不整行讀進記憶體）（第 9 施工關 L17） |
| `inbox` | 寄給呼叫者的，依 `seq`：不帶 `--after` 最近 20 則；帶的話回那一則之後的全部；那一則不存在、過期或不是寄給呼叫者 → `unknown_message` |
| Pipeline agent 命令 | `done`／`result`／`review` 必須帶目前 stage/attempt，由 task holder 或指定 reviewer 回報；`ask` 對話永久保存，`block`／`unblock`／`remind`／`task_create` 由 pipeline queue 處理 |
| `instance_add` | supervisor 做：名字 `[a-z0-9-]{1,24}`、backend、DB 沒有同名、holder 鎖沒被持有（否則 `instance_exists`）；`--dir` 預設 `$AGEND_HOME/workspace/<name>`（0700，daemon 建），給的目錄要存在；program 預設 backend 名；claude 產生 session id；寫 `new` → 先投影 fleet `Starting` → 回覆 → 馬上啟動；成功回覆後立即 get_fleet 就可見 |
| `instance_remove` | 不再監看、關長連線、`Shutdown` holder（最多 5 秒；停不了照樣刪，下次開機巡查收掉）、codex 照清掃規則、刪列、從全貌拿掉（`instance_changed` 不帶 instance；它的「需要你」項目以 `attention_resolved`（`unknown`）離開）；workspace 不刪 |
| `daemon_restart` | 一次一個（另一個進行中 → `invalid_request: a restart is already in progress`）；`mkdtemp` `/tmp/agend-pf-XXXXXX`、`VACUUM INTO` 當下的 DB 複本、跑 `<binary> daemon preflight <dir>`（60 秒）：要 exit 0 而且印出 `agend …`、`db copy: …`、`holder: hello ok, spawn ok, shutdown ok`；失敗 → `preflight_failed: <binary> daemon preflight exited with status 1[: 最後一行 stderr]; the daemon keeps running agend 0.0.0`；暫存 home 一律刪掉（daemon 在預檢中停止也一樣：Drop guard kill 並收屍子程序、`Shutdown` 暫存 home 裡的 holder），60 秒逾時不等子程序的 pipe；預檢結束時 binary 必須還是同一個檔案（device、inode、大小、mtime），`agend.db` 不動。通過 → 回 `restarting { preflight }`、照 Ctrl-C 的順序收尾（不送 `Shutdown`）、`exec <binary> daemon`（pid、終端、環境不變） |
| 繼承的 holder | 新 image 開機時（還沒起任何 holder 前）對每個鎖檔裡的 pid `waitpid(pid, WNOHANG)` 一次；是自己的子程序而且還活著的每秒再查，收到或 `ECHILD` 就不再查；log `reaped inherited holder pid <pid> (exit 0)`；不用 `waitpid(-1)` |
| `task_cancel` | 可帶 reason；CAS 推進 Cancelled、保存 WIP 並釋放 binding；archive 直接串流至檔案並同步後發布，不受程序診斷輸出上限影響；保存失敗保留原 worktree／binding 並回報 Failed；merge in-flight 拒絕取消 |

