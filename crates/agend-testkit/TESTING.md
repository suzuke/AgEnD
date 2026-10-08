# agend-testkit 測試

> **TL;DR**
> - 測假實作本身、7 個契約 suite（對假實作全過；[CONTRACTS.md](CONTRACTS.md) 的 57 條規則各有 mutant，每個 mutant 都被它那條規則的 case 抓到）、假 daemon、3 個假 agent 程式（以真的子程序跑）、假 agent 對真 CLI 錄製檔的一致性檢查。
> - 記住：假 agent 的測試啟動 `src/bin/` 的真 binary，走真的 socket／HTTP；不在行程內呼叫。
> - 下一步：`~/.cargo/bin/cargo test -p agend-testkit`。

## 第 10 施工關驗證

`recorder_recovery.rs` 對真 fake producer 重複 ResumeEmpty 50 次，關閉連線前必須讀到該 thread 的 `thread/started`。Pipeline 的共享 fake／SQLite 契約另外驗 dispatch receipt、task CAS 與 attention_reason 清除同交易：error／conflict 不改註記，success 保留 acknowledgement，generic advance_task 不清註記。真 SQLite attention UPDATE 故障須 rollback version／event／receipt，移除故障後可重試一次。
STO-13 同時跑 FakeStore 與 SQLite；SplitAdvance mutant 故意在失敗時先寫 task，必須被契約推翻。`fake-worker` 由 `agend` 的 pipeline 程序測試與 `pipeline_probe demo` 啟動。

## 第 11 施工關 C 段（已驗收並合併 #145）

- `full_terminal`：共用 CLP-23–28 與 producer generation／failed instance；輸入由真 holder Screen 的 parser producer 提供。相同六條在 `agend/tests/full_terminal_contract.rs` 對真 daemon／holder／PTY 執行。
- `full_terminal_pressure`：producer 操作阻塞時同 socket GetFleet 可回覆、新 grant 等原寫入；capture 中替換訂閱，完成後不能發舊 view。
- `full_terminal_fds`：獨立程序 20 次檢視／取得控制／關閉，最後 socket fd 回 baseline；不代替 native daemon／TUI 的資源驗證。
- `contract_teeth/terminal.rs`：六個 C wire mutant 都須被拒絕；規則表與 cases／mutants 從 `CONTRACTS.md` 及 `CLIENT-CONTRACTS.md` 機械核對。

以上不認證 TUI／鍵鼠／貼上、300 ms 時效或 Codex U17。

opt-in fake manual frontend 的原生驗證在 `agend/tests/codex_u17.rs`：真 PTY 輸入經實際 fake app-server，busy／user item／queue 由 producer 產生。既有 fake Codex 與錄製檔 conformance 仍照跑；本批另有共用完整 App／client／daemon 子程序情境，核 draft／重啟與 receipt；不代表真 Codex smoke。[範圍](../../docs/gates/gate-11c-u17-validation.md)。

新增 `resume_notification_requires_persistent_attribution_before_the_rpc_reply`：真 holder／wrapper／raw fake frontend 先產生並保存 clientId=null 的人工同文 turn，再以預設關閉的 fake-only `agendFake/replayUserOnNextResume` 在下一次 resume response 前重播原 item/completed。測試核 producer item／turn 身分、實際 replay count=1 與一次性關閉；重接到拒絕版本仍不得取人工 receipt。沒有合成人工 item／frame，不呼叫真模型。

CLP-26 的 EOF release 判定以獨立 PTY consumer 紀錄為準；`GetFleet` 回覆不是背景 legacy input 的完成 fence。原 macOS CI 的 `AFTER-EOF` 失敗 log 保留，收尾以修正後固定 head 重驗。

CLP-25 控制請求寫入時必須同時讀取同 socket 的 parser frame；testkit 的每份完整 JSON 行都使用有 socket timeout 的 writer thread，原 reader 持續處理關聯回覆，不重送。native `small_socket_buffers_reject_large_input_and_keep_native_consumer_live` 把自有 socket 收送 buffer 降至 4 KiB，核 pending 真 frame、超限拒絕、後續同連線輸入與 PTY consumer 未收到被拒位元組。原同步 fixture 在相同條件 BrokenPipe；只對 >64KiB 並行的版本也被全新 verifier 以合法 24KiB input 推翻，原負例保留。`small_socket_buffers_accept_valid_input_below_the_previous_duplex_threshold` 另核編碼後 <64KiB 的合法分行輸入，真 PTY consumer 精確收到 24KiB 與後續 sentinel。不能用固定請求大小推斷它會否超過 socket buffer。

`fake_daemon::incompatible_major_gets_a_clear_error_and_close` 核對目前 Hello 宣告的完整版本清單（1.3–1.8）、錯誤內容與拒絕後 EOF；新增協定版本時須同步更新此精確斷言，舊版相容案例仍保留。

## 怎麼跑

```bash
~/.cargo/bin/cargo test -p agend-testkit
~/.cargo/bin/cargo xtask accept testkit     # 另外跑 demo（啟動三個假 agent、假 daemon、契約摘要）
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `fakes::*::tests` | 每個假實作的編排：失敗排隊、回應順序、逾時、crash／adopt、重複 submit、`FakeForge` 的 ancestry（`base_contains`、`set_base`）、自動 turn 可關；`FakeRuntime::on`／`FakeDriver::connect`／`FakeStore::open` 在舊物件 drop 之後仍拿到同一個 holder 表、backend、資料，但 `calls()` 各自從空開始；同一個訊息 id 只一個 turn |
| `contract::tests` | 報表格式：每條失敗都寫出 `<trait>.<case>` 與原因；case panic 也算失敗 |
| `executor::tests` | `block_on` 會在 wake 後再 poll |
| `git_fixture::tests` | 真的 git：canonical、origin、worktree、branch 建得出來，worktree 上的 commit 不動 `main`，drop 後目錄刪掉；`command` 拒絕暫存目錄外的目錄（`/`、`<tmp>`、`root/..`、相對路徑）與 `../x` 檔名；`command` 只在 fixture 自己建的 repo（canonical、origin、linked worktree 與它們的子目錄）裡跑，`root()`、`root/worktrees`、`root` 下其他目錄都拒絕，`commit` 不收 bare origin；`add_worktree` 拒絕名字是 `.git`（不分大小寫）的 worktree，什麼都不建；`dir` 在 `.git` 裡（`canonical/.git`、`canonical/.git/worktrees/w`）時 `command`、`git`、`commit` 都拒絕，`commit` 寫 `commondir`／`config` 被拒絕後檔案沒變，之後在 canonical 與 worktree 跑 git 仍落在 `root()` 裡、外層 config 沒變（舊程式碼會先寫檔再失敗，之後的指令寫到外層 repo）；fixture 放在外層 repo 裡時，在 `root()` 與 `root/worktrees` 寫 config 都被拒絕、外層 config 沒變，允許的目錄 `rev-parse` 都落在 `root()` 裡；暫存目錄或 label 含 `:`（git 拿 `:` 切 `GIT_CEILING_DIRECTORIES`，含了就沒有 ceiling）時 `new` 回 `InvalidInput`、什麼都不建、外層 config 沒變（舊程式碼在這兩種情況會寫到外層 repo）；`commit` 拒絕路徑上的 symlink、任何一段是 `.git`（不分大小寫；linked worktree 的 `.git` 檔沒被改）、已存在且 hard link 數 > 1 的檔案，外面什麼都沒寫；`git` 拒絕 `-C`／`--git-dir`／`--work-tree`／`--namespace`／`-c`；繼承的 `GIT_*`／`AGEND_*` 全部移除、全域與系統設定關掉；FRG-10 的觀察在真 git 上成立（正常 merge 保留之前的 merge，`update-ref` 把 main 設成 branch head 就丟掉） |
| `tests/contract_fakes.rs` | 7 個契約 suite 對假實作全部通過；`run_all_fakes` 每個 trait 剛好一次 |
| `tests/contract_teeth/`（`main.rs`） | 覆蓋測試：讀 CONTRACTS.md 的規則表，每條規則至少一個 case、至少一個 mutant；表上列的 mutant 名與註冊的完全一致；case 與 mutant 都不能指向表上沒有的編號；每個前綴 1..n 連號。mutant 測試：82 個 mutant 平行各跑整個 suite，每個都要讓至少一個標著它那條規則的 case 失敗（`--nocapture` 印出每個 mutant 被哪些規則抓到） |
| `tests/contract_teeth/<trait>.rs` | 各 trait 的 mutant：包住假實作、換掉一個方法（例如 backfill 掉第一個事件、CAS 接受未來版本、prefix 比對 head、stop 只是藏起來、body 截在 64 bytes、時鐘凍結或差 8 小時）。daemon 重啟類：`DaemonScoped`（verifier r3：drop 時殺掉自己啟動的 holder、只 recover 自己的）、`OnlyOwnLifetimeEvents`、`InMemoryOnly`；verifier r4 的四個（狀態只在 Rust 物件裡，撐過一次重啟、撐不過第二次）：`Handoff`（recover 消耗 holder 登記）、`CounterStore`（版本計數器沒存）、`ObjDedup`（去重在 driver 物件裡）、`Gap`（丟掉 daemon 不在時的事件）；verifier r5 的七個：閒置開機才看得出來的 `RewriteOnChange`（打開時清空 holder 登記）、`TruncOnOpen`（打開時清空 DB 檔），`LastIdDedup`（只記最後一個 id）、`ReadAck`（重啟後從較舊的 cursor 補不回），case 自己的 fixture 一直活著才通過的 `SharedMem`（記憶體 DB）、`LiveJournal`（只保留有 driver 活著時的事件）、`LastOneOut`（最後一個 runtime 走時 holder 跟著死）；第 6 輪加的 `FrozenRegistry`、`FrozenDatabase`（重啟後的 daemon 寫的沒存下來，只有開機 4 看得出來）。`forge_without_change_ids_passes`：change id 一律回 `None` 的 forge（local 的行為）通過整個 Forge suite。`OverwritesBase`（FRG-10，gate 2 A23）：merge 時把 base 直接設成 branch 的 head，丟掉 branch 開出之後才 merge 的變更 |
| `tests/contract_teeth/real_runner.rs` | 一個真的 `sh -c` runner（子程序放進自己的新 process group，逾時只對那個 group 送 SIGKILL）：全部旋鈕正確時通過整個 Runner 契約；只殺 `sh`、結束後才讀管線、在別的目錄跑，是 RUN-8、RUN-4、RUN-9 的 mutant |
| `tests/fake_knobs.rs` | 每個假實作（Forge、Driver、Store、Runtime、Runner、Notifier）的每個方法：`fail_next` 只讓下一次失敗、`calls()` 依序記下每次呼叫（含失敗的）；`FakeForge::merges`／`base_head` 只記成功的 merge |
| `tests/fake_daemon.rs` | hello 必須在前（無效 JSON 也回 `hello_required` 並關閉）、hello 之後的無效 JSON 不斷線、drop 時已開的連線讀到 EOF（2 秒內）、major 不合的錯誤訊息、狀態與未知請求、事件身分（沒帶、舊 attempt、別的關卡、重播都 `stale_result`）、backlog 再即時事件（id 從基準 + 1 開始）、回應先於它造成的事件、請示回答、`open_ask` 帶 task 與脈絡摘要而且可以回答；第 11 施工關 B 段：每個 instance 自己的畫面、位元組接到畫面、換訂閱後舊的不再送、連線關掉訂閱就消失，`terminal_input` 依身分、instance、backend 的順序拒絕、接受的原樣記下且不回應，`hold_resolved_events` 讓 `attention_resolved` 等到放行；第 9 施工關：`hello` 帶 daemon 的版本、pid、`boot_id`，操作者的 agent 命令與 agent 的 `operator` 請求都 `forbidden`（訊息逐字）；`daemon_restart` 到 `/usr/bin/false` → `preflight_failed`，沒帶 binary → `restarting`、連線關掉、`boot_id` 變大 |
| `tests/fake_daemon_fds.rs` | 關掉的連線（有事件訂閱也一樣）馬上放掉 fd，不等下一個事件（100 次訂閱後關掉，fd 數不變） |
| `tests/contract_fakes.rs::client_protocol_*` | 假 daemon 通過 CLP-1..22；反向檢查：事件 id 改回從 1 開始（proxy 重新編號）時 CLP-4 必須失敗（第 8 施工關 P9） |
| `tests/contract_teeth/client.rs` | CLP 的 mutant（第 11 施工關 B 段加 `FreezesScreen`、`ScreenForAnyInstance`、`AnyoneMayType`，CLP-10 的改成 `AcceptsRefused`）（假 daemon 前面加改行的 proxy），每個只跑自己那條規則的 case，都必須失敗 |
| `tests/fake_codex.rs` | 第 7 施工關：`clientUserMessageId` 回成 `clientId`、`thread/turns/list` 分頁且進行中的 turn 在最後、`thread/queue/list`、忙碌時 `thread/queue/start` 回 `-32600 … active or pending turn`、`thread/resume` 找不到回 `thread not found`、`excludeTurns` 不帶 turn、舊的 listen symlink 被換掉、fake-codex --version 固定印未獲開放的 codex-cli 0.158.0，`fake-codex` 的 `app-server` 在 stdin 結束後照跑（`agendFake/exit` 才結束，thread 存在 socket 旁）與假 TUI 印參數、`q` 結束。原本的：turn 完成事件帶回 threadId／turnId 與回覆；steer（錯的 turn id 被拒；在同一輪另成一則回覆）、queue 自動出列成新 turn、interrupt；approval 等待決定；只有 resume 過的 thread 才推事件；長路徑與短路徑都是 symlink 指到 temp dir 裡的短 socket（不留目錄） |
| `tests/fake_opencode.rs` | SSE 事件順序（照 `transcripts/opencode/one_turn.jsonl`）、同步 prompt 回覆、忙碌排隊、abort 標 `MessageAbortedError`、REST 補歷史、status |
| `tests/fake_claude.rs` | Stop hook block 多一輪（`stop_hook_active` false → true）、Esc 中斷不觸發 Stop、hook payload、channel 包裝、未知 channel server 的錯誤、transcript 在專案目錄內 |
| `tests/conformance.rs` | 一致性檢查：用錄製器的同一套情境（`one_turn`、`interrupt`、`approval`、`busy`、`resume`）驅動 `recorder::BACKENDS` 的每個假 agent（新增 backend 不用加測試），和 `transcripts/<backend>/` 的真 CLI 錄製檔按形狀比對（比對規則只在 `recorder::shape`，見 [RECORDER.md](RECORDER.md)）；突變檢查：錄製檔裡的 `turn/completed` 多一則要比出差異、串流 delta 多一則不算；每個 backend 支援的情境剛好各有一個錄製檔（第 7 施工關的 codex 情境 `turns_list`、`queue_idle`、`resume_empty` 在使用者錄之前可以沒有，印 `not recorded yet … skipped`）；錄製檔通過 secret scan（含本機 denylist）；`gate_7_codex_scenarios_run_against_the_fake`：這三個情境對假 app-server 跑得完，而且送出了它們要問的方法。`AGEND_CONFORMANCE_DUMP=<dir>` 另外把假 agent 的流量寫成錄製檔以便對照 |
| `recorder::{redact,shape}::tests` | 遮蔽保留型別、id 換成穩定的 placeholder、時刻／`/tmp` uid／`platformOs`／`authMode`／`sk-` key 換掉、MCP 狀態通知只留一則空白的；secret scan 抓得到漏網的 UUID／家目錄／email／id／短 `sk-proj-…`／時刻／uid／denylist 字詞（且不印出字詞）；形狀比對忽略值與 id、但抓得到型別、欄位、分類值、順序與生命週期事件則數的差異，串流 delta 則數不算 |

## 花時間的地方

- Runner 契約的兩個「停掉了嗎」case 用真的時間：`sleep 3; touch timed-out-command-finished` 與 `sh -c 'sleep 3; touch timed-out-child-finished'; true` 以 200 ms 逾時跑，各等到約 4.5 秒確認標記檔沒出現（只讀檔，不送 signal 探測）。假實作也一樣等，所以 Runner suite 每跑一次約 9 秒。
- `contract_teeth` 約 15 秒：mutant 平行跑，最慢的是 Runner mutant（約 9–15 秒）與真 `sh` runner 的對照測試。
- `conformance` 約 21 秒：3 個 backend 平行，每個情境啟動假 agent（`--turn-ms 900`）走完整個情境；claude 與 codex 的 `resume` 各啟動兩次。

## 輸入從哪來（#1493）

- 假 daemon 與測試都用 `agend_core::protocol::client` 型別 + `serde_json` 編碼；沒有手寫 client protocol JSON。
- 契約 suite 用 core 的建構子（`Task::new`、`Workflow::builtin_code`、`model::work_branch`）產生輸入。
- backend 協定（codex、opencode、claude）是外部格式，沒有 Rust producer：真的 producer 是真 CLI，它的輸出錄在 `transcripts/`；假 agent 的形狀由一致性檢查對錄製檔比對，錄製器與假 agent 測試共用同一套情境程式（`recorder`）。

## 用到的假實作

- 不適用（這裡就是假實作的家）

## 還沒測的

- [ ] 契約 suite 對真實作（各施工關接上：Store 第 5、agent runtime 第 6、Driver 第 7／12、Runner 與 Forge local 第 10、Notifier 與 Forge github 第 12）
- [x] 假 agent 的欄位與事件順序對真 CLI 比對：`tests/conformance.rs` 對 `transcripts/`（2026-09-25 錄製；CLI 升版時重錄，見 [RECORDER.md](RECORDER.md)）
- [ ] binding 快照 fixture（第 3 施工關需要時）

C 段 CLP 拒絕案例同跑真 parser-backed fake 與真 daemon：agent caller 的 Acquire／Resize／Input／Release 全部 forbidden；之後核尺寸不變、原 owner 輸入仍可實收、拒絕 bytes 沒有進 consumer。

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-testkit
```

## 第 12A bridge producer

抽出的 hook_payload／initialize_request 仍由 FakeClaude 與原 conformance 使用；新的 ack_request 用 shared core ClaudeReceipt 產生 MCP 工具請求。真正 consumer 的跨 process 回歸在 `agend/tests/claude_bridge.rs`，對真 helper／daemon 跑；未新增真 CLI 錄製或模型回合。

13C 的獨立 `fake-claude-cli` 固定回報 2.1.284，重播已保存的 Ready 畫面；由正式 MCP channel helper 接收訊息、呼叫 agend_ack 並等待對應回覆，再送原生 PostToolUse／Stop hook。僅模型決策是假，不寫 daemon DB、不代替 hook helper；不涵蓋真 CLI 認證、啟動選單或忙碌排隊。歷史 `fake-claude` 的未確認 channel 行為不變，沿用原 conformance。

`fake-opencode-cli` 為 12B holder 整合提供 version／serve／attach，API 沿用錄製對照的原生 producer，health 固定 1.18.34，資料只寫自有 AGEND_HOME/opencode 內的 XDG_DATA_HOME。舊 fake-opencode-serve 與 1.18.31 conformance 固定不變；此 CLI fixture 不驗真 TUI 畫面、不啟動模型。

12C whole-queue 測試從 executor effects 核 `forge:github`／`prepare-main:github`／`find-merge:github`，搭配真 API／Git 測試使用，不能獨立證明遠端 GitHub 合併。

Forge 契約的不同 branch 使用不同 task ID，Submission.task_id 由 work_branch 的正式 parser 取回；保留所有 head／merge／多 PR 斷言。local 與 GitHub 原生 adapter 均跑同套，`contract_teeth` 繼續驗每個故障 mutant 會被抓出。

12C 嚴格 base 政策限定 FRG-10 的第二條過期 sibling 必須回已識別的 policy refusal；驗 main 完全不變、先前 merge／head 保留、拒絕 head 未進 main 且 branch head 不變。預設 local／fake 仍跑原本兩次成功的 ancestry 斷言與 OverwritesBase mutant；strict 額外拒絕先改 base 才報錯、錯誤種類不符與意外成功的 mutants。FRG-5 成功前提含 server policy 允許；不略過任何案例。
