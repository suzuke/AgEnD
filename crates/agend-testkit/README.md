# agend-testkit

> **TL;DR**
> - 共用測試基礎設施（只能當 dev-dependency）：7 個 trait 的假實作、契約測試（含 client protocol 的 CLP）、假 daemon（預設 1.3，注入 producer 後 1.4）、3 個假 agent 程式、真 backend 的錄製器。
> - 記住：**假實作要跑和真實作同一套契約測試，假 agent 要和真 CLI 的錄製檔形狀一致**，才不會漂移（v1 #1483）。
> - 下一步：`~/.cargo/bin/cargo xtask accept testkit`；契約規則看 [CONTRACTS.md](CONTRACTS.md)，錄製與一致性檢查看 [RECORDER.md](RECORDER.md)。

## 第 10 施工關（已驗收，2026-10-02）

新增 `fake-worker`：讀真 CLI inbox、在綁定 worktree commit、回報 done／review；可指定 `--fail-checks-once`、`--changes-once`、`--leave-wip`、`--hold`。只有 agent 決策是假，daemon、holder、SQLite、git、shim、checks 與 forge 都是真的。STO-13 與 SplitAdvance mutant 驗證原子推進；pipeline 共享 FakeStore／SQLite 契約驗 CAS 與 attention_reason 清除同交易，錯誤／衝突保留原註記與 acknowledgement，generic advance_task 保留原語意。


假 worker 的 workspace 放 `.leave-wip` 會讓下一次 work 留下 untracked 檔案；移除檔案恢復正常。這只控制測試用的 agent 行為。

Gate 10 的 `FakePipelineExecutor` 實作 core executor port，組合 FakeStore／FakeForge／FakeRunner 並記錄 bindings、投影與副作用；`clean_worktree` 透過 FakeRunner 的 index／status 回覆判斷，供 daemon 完整 queue 測試使用。FakeDriver／FakeClock clone 共享同一個測試狀態。

## 第 11 施工關 C 段（已驗收並合併 #145）

第 11C 當時兩者提供 client 1.4；第 12A 真 daemon 提升為 1.5，安裝 terminal producer 的 FakeDaemon 保持 1.4；六項 C 契約同跑 fake／真程序，producer 是 holder parser。控制 worker、generation／停止、fd 清理與完整 fake U17 都有回歸。真 Codex 證據另記於首次 U17，不以 fake 代替。 [版本政策](../../docs/gates/gate-11c-codex-input.md)。

fake Codex 的 `-c agend_fake_manual_tui=true` 明確啟用 raw PTY frontend：bracketed paste／Enter 經 remote app-server 送人工 turn，不帶 daemon clientId；預設 frontend 保持原行為。paste 結束會經真 PTY 輸出 draft 標記，完整 App 用畫面確認 draft 已收到再重啟 daemon。這是 U17 fixture，不代表真 Codex CLI 已驗證。fake-only `agendFake/replayUserOnNextResume` 預設關閉；明確指定 thread 後，只重播已存入 thread history 的原人工 user item，供 resume 回覆前的永久歸屬回歸使用，`agendFake/resumeReplayCount` 可核實際次數。[完整 fake 證據](../../docs/gates/gate-11c-u17-validation.md)。

## 負責

| 項目 | 模組 | 內容 |
|---|---|---|
| 假實作 | `fakes` | `FakeDriver`、`FakeForge`、`FakeStore`、`FakeRuntime`、`FakeNotifier`、`FakeClock`、`FakeRunner` |
| 契約測試 | `contract` | 每個 trait 一個 suite：`contract::<trait>::run(實作名, 建 fixture 的函式)` 回傳 `Report`；規則編號見 [CONTRACTS.md](CONTRACTS.md) |
| 假 daemon | `fake_daemon` | 行程內的 client protocol server（unix socket + JSON Lines）與 `ProbeClient` |
| client protocol 契約 | `contract::{client, terminal}` | CLP-1..28：同一套 case 對假 daemon 與真 `agend daemon`；`proxy` 是 mutant 用的改行 proxy（第 8 施工關 P9） |
| 假 agent | `fake_agent` + `src/bin/` | `fake-codex-app-server`、`fake-opencode-serve`、`fake-claude`；`fake-codex`（`codex` CLI 的替身，給第 7 施工關的 `sh` 包裝用：`app-server` 與假 TUI `resume`） |
| 執行 future | `executor` | `block_on`：不用 async runtime 就能跑 trait 的 future |
| 暫存目錄 | `tempdir` | `TempDir`：唯一目錄，drop 時刪除 |
| 錄製器 | `recorder` + `src/bin/agend-record.rs` | 用假 agent 模擬的傳輸驅動真 CLI 跑 5 個情境（codex 另有第 7 施工關的 3 個：`turns_list`、`queue_idle`、`resume_empty`，錄了之前一致性檢查跳過），錄成 `transcripts/<backend>/<scenario>.jsonl`；同一段情境程式也驅動假 agent，`tests/conformance.rs` 按形狀比對（[RECORDER.md](RECORDER.md)） |
| 暫存 git repo | `git_fixture` | `GitFixture`：canonical repo、bare team origin、linked worktree 與 branch，全部在一個新的暫存目錄裡；見下方「git fixture」 |

13C 安裝 canary 另有 `fake-claude-cli`，只模擬原生 ACK／PostToolUse／Stop 的決策流程與版本回報，透過正式 helper 投遞事件；與歷史 `fake-claude` 分開。它不使用模型或共享認證，測試入口在 `agend/tests/backend_canary.rs`。

## 不負責

- production 邏輯；呼叫真 backend 或模型
- binding 快照 fixture（第 3 施工關需要時加）

## 假實作：共同規則

| 規則 | 怎麼用 |
|---|---|
| 可預測 | id、cursor、SHA、pid 都來自計數器；不讀時鐘、不 sleep |
| 可檢查 | `calls()` 依序列出每一次 trait 呼叫（失敗的也算） |
| 可編排 | `fail_next("<trait 方法名>", "訊息")` 讓下一次呼叫回 `FakeError`；打錯方法名會 panic |
| 契約之外的行為 | 寫在各假實作的 doc comment（例如停止不存在的 holder 會失敗） |

各假實作的編排方法：

| 假實作 | 編排 |
|---|---|
| `FakeDriver` | `with_instance`／`add_instance`；預設每次送達自動產生 busy → confirmed → turn completed → idle 事件，同一個訊息 id 再送只回回條、不再產生 turn；`set_auto_turn(false)` 後用 `push_event`；`next_receipt`；`backend()` 取出 `FakeBackend`（假的 agent 那一側，不是 driver；`push_event` 讓 agent 自己產生事件），`FakeDriver::connect(&backend)`（daemon 重啟：同一個 backend 上的新 driver，事件與 cursor 都在） |
| `FakeForge` | `push(branch)` 加一個 commit（新 branch 從目前的 base 開出）；`merges()`；`base_head()`（base branch 的 head：最後一個 merge commit，還沒 merge 時是 `BASE_ROOT`）；`base_contains(commit)`（commit 是 base head 或它的祖先）；`set_base(commit)`（像 force push 把 base 移到某個 commit）；change id 是 `Some("change-<n>")` |
| `FakeStore` | `insert_workflow`、`events(task)`；`file()` 取出 `FakeStoreFile`（假的 DB 檔，不是 store），`FakeStore::open(&file)`（daemon 重啟：同一份資料上的新 handle） |
| `FakeRuntime` | `crash(id)`（holder 自己死掉）、`adopt(handle)`（外來的 holder）、`running()`；`holders()` 取出 `FakeHolders`（假的 holder 程序，不是 runtime），`FakeRuntime::on(&holders)`（daemon 重啟：同一張 holder 表上的新 runtime） |
| `FakeNotifier` | `delivered()` |
| `FakeClock` | `advance(ms)`、`set(ms)`（往回設會 panic）、`peek()`（不算一次讀取）、`reads()` |
| `FakeRunner` | `on(command, ScriptedCommand)`；`takes_ms` 大於 timeout 就回 timed out；沒編排的指令 exit 127 |

## 契約測試：怎麼接真實作

1. 為真實作寫 fixture，實作 `contract::<trait>::<Trait>Fixture`（例如 `ForgeFixture::commit_to` 在暫存 repo 真的 commit，`ForgeFixture::base_head` 從 trait 外面讀 base branch 的 head）。
2. 在該 crate 的測試裡：`agend_testkit::contract::forge::run("local", MyFixture::new).assert_passed();`
3. 需要 tokio 的真實作：在 fixture 裡進入 runtime（future 仍由 `block_on` 驅動）。

規則的唯一清單是 [CONTRACTS.md](CONTRACTS.md)：每條一個編號，每個 case 標著它驗的編號（失敗訊息 `FAIL forge.<case> (FRG-6): …`）。

| suite | 規則 | 重點 |
|---|---|---|
| Driver | `DRV-1`–`9` | 閒置送達 Sent／Confirmed；最後 `TurnCompleted`；同一個訊息 id 只一個 turn，重啟後再送也一樣；從任何 cursor 補回之後的**全部**事件、不含舊的、讀了不消耗，每次 daemon 重啟後的新 driver 也一樣（也從較舊的 cursor），含 daemon 不在時的事件 |
| Forge | `FRG-1`–`10` | 沒有 change id 是 `None`（local）；整串 head 相等才 merge（空字串、短 SHA 不算）；比的是 merge 當下的 head；不符回真正的 head 且什麼都不變（從 `base_head()` 看）；merge 落在目前的 base 上、不弄丟之前的 merge（從 `base_contains()` 看） |
| Store | `STO-1`–`12` | CAS 是相等（舊版本、未來版本都衝突）；衝突回報實際版本；版本嚴格遞增，跨重新開啟也是；事件依序、依 task 分開；每次重新開啟後全部還在 |
| Runtime | `RTM-1`–`9` | recover 回的 handle 與 start 的相同；停止要真的停（`is_running()` 從 trait 外看）；每次 daemon 重啟後 holder 都還在，新 runtime 找得回、停得掉（D3） |
| Notifier | `NTF-1`–`4` | 欄位原樣、3,000 字多位元組 body 不截斷、不修剪空白、順序不變 |
| Clock | `CLK-1`–`4` | unix 毫秒、UTC（對照 `utc_now_unix_ms()`）、不倒退、不凍結 |
| Runner | `RUN-1`–`9` | 輸出逐位元組、256 KiB 不卡；逾時 2 秒內回報，`sh` 與它啟動的子程序都停掉（標記檔判斷）；在指定目錄跑 |
| ClientProtocol | `CLP-1`–`28` | 不是 trait，是 server（第 9 施工關加權限兩個方向、`instance_add`／`remove`、`daemon_restart` 的形狀、`task_cancel` 不改狀態、`send` 同 id 只收一次與 `inbox --after`）：`hello` 在前、版本協商、全貌之後的事件連號、舊／未來游標 `event_gap`、不帶游標重播、未知請求不斷線、兩個 client 同序、慢 client 被關、重啟後 id 變大、拒絕的請求不改狀態、只有操作者能 `resolve_attention`、終端先畫面；fixture 是 `ClientProtocolFixture`（`FakeDaemonFixture`；真 daemon 的在 `agend-daemon/tests/common/client_process.rs`） |

每條規則至少有一個故意弄壞的實作（mutant），列在 CONTRACTS.md 那一列；`tests/contract_teeth/` 跑全部 mutant，並檢查規則表、case、mutant 三者互相對得上（見 [TESTING.md](TESTING.md)）。接真實作時 fixture 多實作的方法：`ForgeFixture::base_head`、`ForgeFixture::base_contains`（例如 `git merge-base --is-ancestor`）、`RuntimeFixture::is_running`（只拿持久狀態）、`ClockFixture::utc_now_unix_ms`；`DriverFixture::turn_timeout` 可選（預設 10 秒）。

daemon 重啟：`RuntimeFixture`、`DriverFixture`、`StoreFixture` 各有一個 `Persisted` 型別（持久狀態：真實作是 run 目錄與 holder 程序、backend、SQLite 檔；不是 trait 物件，也不讓任何 trait 物件活著）、`persisted(&self)` 與 `boot(&Persisted) -> Self`（從持久狀態開機一個新 fixture）。`DriverFixture::emit_while_down(&Persisted)`：daemon 不在時讓 agent 自己跑完一輪（不經過 driver）。契約 case 拿到的 fixture 是 by value，重啟類 case 先把它轉成持久狀態再 drop。

重啟類 case 都走同一個 daemon 生命週期（`contract::daemon_lifecycle`：做事 → 閒置 → 做事 → 檢查 4 次開機，每次先復原，兩次開機之間沒有實例活著、backend 照常動），細節與 fixture 要求（走真的持久層，不用全域 `static` 或記憶體 DB；第 5、6 施工關要跨真的 process 重啟跑）見 [CONTRACTS.md](CONTRACTS.md)。

## git fixture

`GitFixture` 的目錄布局、指令與環境衛生規則見 [GIT-FIXTURE.md](GIT-FIXTURE.md)。原 `README.md#git-fixture` 入口保留。

## 假 daemon

預設 1.3；安裝 `TerminalProducer` 後提供 1.4 的真 parser frame 與控制路徑。完整協商、事件、命令與訂閱行為見 [FAKE-DAEMON.md](FAKE-DAEMON.md)。

## 假 agent 程式

假 codex app-server（`fake-codex-app-server`、`fake-codex app-server`）跟真的 codex 一樣一邊寫一邊讀：寫不出去的資料留在 WebSocket 的緩衝區，thread 繼續讀（第 9 施工關；原本寫的時候不讀，大訊息會跟 daemon 互相卡住）。`fake-codex app-server … --disable duplex-io` 保留舊行為，當「不讀的 peer」測 daemon 用。

所有假 agent（`fake-codex` 除外，見表）：回覆固定為 `fake reply: <prompt>`；一個 turn 花 `--turn-ms`（預設 100）毫秒；**stdin 結束就以 0 結束**；prompt 有一行以 `run: <指令>` 開頭時要求授權（不會真的執行）；設了 `AGEND_FAKE_STATE_DIR` 才把 thread／session 存在那裡，重啟後可接續（沒設就不寫任何檔）。

| 程式 | 真的指令 | 涵蓋 | 沒涵蓋 |
|---|---|---|---|
| `fake-codex-app-server` | `codex app-server --listen unix://<path>`（0.156.1） | WebSocket（unix socket）上的 JSON-RPC：`initialize`、`thread/start`／`resume`（重啟後也可；找不到 → `thread not found`；`excludeTurns`）、`turn/start` → item 與 delta、`thread/tokenUsage/updated`、`thread/status/changed`、`turn/completed`；`turn/steer`（同一輪另一則回覆）、`turn/interrupt`、`thread/queue/add`（自動出列）、`item/commandExecution/requestApproval`；socket 一律在短路徑、要求的路徑是 symlink（陷阱 1），舊的 symlink 先刪。第 7 施工關加、**未查證**（沒有錄製檔）：`clientUserMessageId` 回成 `clientId`、`thread/turns/list`、`thread/queue/list`、`thread/queue/start`、閒置時 `queue/add` 立刻開始、`agendFake/exit`（只有假的：程序立刻結束） | `thread/items/list`、reasoning item、其他授權種類、sandbox、MCP／帳號通知 |
| `fake-codex` | `codex [-c k=v]… app-server …`、`codex [-c k=v]… resume <thread> --remote unix://…` | `app-server`：同上，但 stdin 結束**不**停（包裝在背景起它，stdin 是 `/dev/null`），thread 存在 `<listen 路徑>.fake-state/`（agent 環境白名單沒有 `AGEND_FAKE_STATE_DIR`）；`resume`：假 TUI，印 `agent args: resume <id> --remote <url>` 與 `agent config: <-c 值>`，一行 `q` 結束，不連 app-server | 真的 TUI 畫面 |
| `fake-opencode-serve` | `opencode serve --pure --hostname 127.0.0.1 --port <p>`（1.18.31） | `POST /session`、`prompt_async`（忙碌時排隊）、`message`、`abort`（`session.error` + `MessageAbortedError`）、`GET /session/:id`、`…/message`、`/session/status`、`/permission` 與 `POST …/permissions/:id`、`/event` SSE（step／text part、delta、session 簿記事件；無 replay） | reasoning part、plugin／catalog 事件、heartbeat、V2 permission、config、model |
| `fake-claude` | `claude`（互動模式，2.1.282） | `.claude/settings.json` hooks：`SessionStart`（startup／resume）、`UserPromptSubmit`、`Stop`（`decision: block` 多跑一輪，`stop_hook_active`）、`PreToolUse` + `PermissionRequest`（`run: `）、`SessionEnd`（`/exit`）；stdin 的 `Esc` 中斷（不觸發 Stop）；`.mcp.json` channel server：`initialize`、`tools/list`、`notifications/claude/channel` 包成 `<channel source=...>`；**忙碌時的 channel 訊息刻意不處理**（真的會排隊並照做；D16、gate-02 A6）；transcript 寫在專案內 `.claude/fake-transcripts/` | TUI 畫面、啟動對話框、Bash 以外的工具、`PostToolUse`、`Notification`、hook exit 2、CLAUDE.md 來源說明的效果 |

各模組開頭的表格是準確的清單。欄位與事件順序由一致性檢查（`tests/conformance.rs`）對真 CLI 的錄製檔按形狀比對；CLI 升版時用 `cargo xtask record` 重錄（[RECORDER.md](RECORDER.md)）。

## 錄製器：新增一個 backend

1. 在 `src/recorder/` 加一個模組，實作 `recorder::Backend`（`name`、`program`、`fake`、`scenarios`、`run`：啟動、傳輸、情境步驟；同一段 `run` 要能驅動真的與假的）。
2. 加進 `recorder::BACKENDS`；需要時在 `recorder::shape` 的 `IGNORED`／`UNORDERED`／`TIMED`／`COLLAPSED` 加規則並寫原因。
3. `cargo xtask record <name> --sandbox <寫入沙箱腳本>` 錄製、確認遮蔽、commit。`tests/conformance.rs` 不用改：它逐一檢查 `BACKENDS` 裡的每個 backend。

## 依賴規則

| 依賴 | 為什麼 |
|---|---|
| `agend-core` | 被測的型別與 trait |
| `serde_json` | client protocol、codex JSON-RPC、opencode JSON、claude hook payload 都是 JSON |
| `tungstenite`（`default-features = false`，只開 `handshake`） | codex app-server 的傳輸是 WebSocket；用現成、同步（不帶 async runtime）的實作，避免自己寫的 frame 解析和真的 client 不相容 |

- 任何 crate 都不能把它當一般依賴（`cargo xtask check-deps`）。
- HTTP 與 SSE 用 std 自己寫（`fake_agent::http`，約 150 行），不加 HTTP crate。
- 只支援 unix（`fake_daemon`、`fake_agent` 以 `cfg(unix)` 編譯）；CI 只有 ubuntu 與 macOS。

## 入口

- `agend_testkit::fakes::*`、`agend_testkit::contract::{<trait>::run, run_all_fakes}`
- `agend_testkit::fake_daemon::{FakeDaemon, ProbeClient}`
- `agend_testkit::contract::client::{run, run_rules, ClientProtocolFixture, FakeDaemonFixture, fake_with_ids_from_one, slow_clients, proxy}`
- `agend_testkit::fake_agent::{locate, codex::Probe, http::{call, EventStream}}`
- `agend_testkit::recorder::{BACKENDS, Backend, Scenario, run_fake, read_transcript, shape::compare}`；`cargo xtask record`、`agend-record`
- 其他 crate 的測試要用假 agent 程式：先 `cargo build -p agend-testkit --bins`，再用 `fake_agent::locate("fake-codex-app-server")`；行程內的假 app-server：`fake_agent::codex::Server::bind`（第 7 施工關的 driver 測試）；`codex` CLI 替身：`fake_agent::codex_cli::main`（`agend` crate 的 `fake_codex` example 包一層，`cargo test -p agend` 會一起編）

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-testkit
~/.cargo/bin/cargo xtask accept testkit
```

## 第 12A bridge producer

`fake_agent::claude::{hook_payload, initialize_request}` 抽出既有 fake producer，讓 native helper consumer 使用相同形狀。`ack_request` 產生本次新增 agend_ack 契約。FakeClaude 仍沿用既有錄製檔，不自動呼叫新 ACK；完整新版本真 CLI conformance 尚未通過，不以 native bridge 代替。

FakePipelineExecutor 記錄 forge 選擇、base refresh 與 merge recovery 的 kind，供 pipeline 接線測試核對；不是 GitHub API 的行為替代證據。

12C 嚴格 base 政策限定 FRG-10 的第二條過期 sibling 必須回已識別的 policy refusal；驗 main 完全不變、先前 merge／head 保留、拒絕 head 未進 main 且 branch head 不變。預設 local／fake 仍跑原本兩次成功的 ancestry 斷言與 OverwritesBase mutant；strict 額外拒絕先改 base 才報錯、錯誤種類不符與意外成功的 mutants。FRG-5 成功前提含 server policy 允許；不略過任何案例。
