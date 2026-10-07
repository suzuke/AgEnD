# agend-daemon 測試

> **TL;DR**
> - store 的測試都用暫存目錄裡的真 DB 檔（不用 in-memory SQLite）；重開與硬殺跨真的 process 驗（第 5 施工關 P6、P7）。
> - daemon ↔ holder（第 6 施工關）：純邏輯在本 crate 單元測試；要真 `agend` binary 的測試在 `crates/agend/tests/`（`holder_runtime.rs`、`daemon_process.rs`），各段與 `daemon_probe demo` 共用 `tests/common/daemon_process.rs`。
> - 記住：每個領域模組都要能對 testkit 的假實作單獨測；每個 adapter 要跑契約測試。
> - codex（第 7 施工關）：driver 對行程內的假 app-server ＋真 DB 在 `tests/codex_driver.rs`（DRV-1..9、三級忙碌、冪等、崩潰對帳、授權、四次開機跨 process），各段與 `codex_demo` 共用 `tests/common/codex_driver.rs`；真 daemon ＋真 holder ＋`sh` 包裝＋`fake_codex` 在 `crates/agend/tests/codex_process.rs`，共用 `tests/common/codex_process.rs`。
> - client protocol（第 8 施工關）：`fleet` 的游標規則是單元測試；真 daemon 的 CLP 契約與 socket／`retry`／終端／重啟在 `crates/agend/tests/client_protocol.rs`，各段與 `client_demo` 共用 `tests/common/client_process.rs`。
> - CLI 的 daemon 端（第 9 施工關）：權限、`send`／`inbox`、`instance add|remove`、`daemon restart` 的預檢與 `exec`、繼承 holder 的收屍，都對真 `agend` binary 測：`crates/agend/tests/cli.rs`（CLI-n 表、重啟、預檢、里程碑）與 `client_protocol.rs`（CLP-13..17）。
> - 下一步：`cargo test -p agend-daemon`；看 demo：`cargo xtask accept store`、`cargo xtask accept client`、`cargo xtask accept cli`。

正式 P5 啟動按鍵與 P6 初始 idle 的實作、schema v9 與 native 驗證邊界見 [啟動處理](../../docs/gates/gate-12a-startup-runtime.md)。
private startup capture 明確登記 manual 模式，保留原初始尺寸、停用 daemon 自動鍵；SQLite 重啟仍保留。capture 的 native 回歸與 raw PTY 測試不受正式 P5 介入。
`agend/tests/tui_outer_pty.rs` 另保留自動 P5 與人工 TUI 共存的真 outer PTY 回歸，核最後 modes、focus、history 與 20 次 App 關閉；terminal hub 在共用 sample 到期後再確認尾段，不用 manual 模式取代此項。
自動 P5 的尾段時效在 80×23 與 100×24 的實際 agent viewport 各驗 12 筆，保留每筆 300ms 預算。啟動辨識只查一份完整 24 列 frame；未知尺寸不建立初始 idle。未知且 session／notice 未變時最多一秒重查，輸出／link 變動仍立即取樣；已知選單與 Ready 維持原檢查。首筆 dirty 等 50ms 共用 sample 過期，持續輸出的後續 notice 不延長等待。

## 第 10 施工關驗證

`cargo test -p agend-daemon --test pipeline_adapters` 跑真 Runner 的 RUN-1..9、LocalForge 的 FRG-1..10、metadata／cache／外部寫入／FIFO marker 反向測試、冷 cache、TCP 可連／daemon socket 不可連、父程序 SIGKILL 的子程序清理與真 cargo／npm 編譯測試；`tests/store.rs` 跑 STO-13、schema v7 與既有有資料的 migrations。 `pipeline_store_ports` 的 fake／SQLite 共享契約驗 attention 清除同 CAS transaction、ack 保留與 generic advance 語意；真 SQLite 拒絕 attention UPDATE 時，version／event／receipt／note 全部 rollback，移除故障後重試成功。

## 第 11 施工關 C 段（已驗收並合併 #145）

`runtime::client::tests` 以真型別 serializer 產生邊界行，驗 8 MiB 包含換行、超限整份拒絕；`runtime::terminal::tests` 驗 1.0 holder 不送新請求、legacy write lock 阻擋時仍可逾時與停止。`agend/tests/terminal_runtime.rs` 走真 binary／holder／PTY：並行 viewport 配對、實際尺寸與輸入 ack、交接後舊 input 拒絕、取消 native blocked input 後舊憑證失效／重連不重送、grant 回覆已到但未接收的取消競態、取消唯讀查詢不打斷控制、尺寸及 holder 保留、超限輸入整份拒絕與重起 generation。

8 個 terminal_hub native cases 驗 caller、socket-scoped view／attach、控制交接、EOF／停止、尺寸、dirty 尾段、歷史、20 次開關與正式 client。完整 TUI、fake C 契約與 U17 已有原生回歸；使用者要求剩餘行為自動驗證；最新 head verifier／CI、清理與 merge 確認見 [驗收收尾](../../docs/gates/gate-11c-closeout.md)。 [版本政策](../../docs/gates/gate-11c-codex-input.md)。

`agend/tests/terminal_capability.rs` 對真 binary 與 FakeDaemon 跑同樣的 client 1.4 請求：以明確 1.3 hello 協商，明示能力不足，控制請求先拒 agent、錯誤保留 request id，拒絕後 get_fleet 仍正常。真 daemon 選 1.4 的控制路徑另跑 native cases；這組案例刻意協商舊能力 1.3；完整 C 共享契約已在 `agend/tests/full_terminal_contract.rs` 對 fake／native 同跑 CLP-23–28。

`pipeline::attention::tests` 注入真 Fleet 快照與 Retry 的受控交錯：原程式重建已移除項目、覆蓋新失敗，兩個反例 exit 101；條件更新後皆通過，另核正常 enrichment／no-op／錯 id。真 client protocol CLP-11 另跑；[證據](../../docs/gates/gate-11c-regression-validation.md)。

Codex history 的四個匹配 cases 保留 lost-reply 對帳，新增 never-attempted／foreign clientId 拒絕；native U17 foundation 在 agend 的 `codex_u17`。[範圍](../../docs/gates/gate-11c-u17-validation.md)。

Codex history 五個 cases 核嚴格 clientId 與舊 lost-reply 相容性；codex_u17 12 個 native tests 核 App／daemon、caller、已驗版本開放、未驗版本拒絕、缺失／過期 holder 版本記錄、DB 拒寫與 attempted crash-window 跨拒絕重啟。driver unit 另驗歸屬讀取失敗在 resume 前拒絕；store unit 驗歸屬持久、冪等及永久保留。v1–v6 fixtures 升級比對 golden。 [版本政策](../../docs/gates/gate-11c-codex-input.md)。

## 第 12A store 基礎

`cargo test -p agend-daemon --test claude_store` 跑真 SQLite 並行預約、ACK tuple／重複與延遲、channel／Stop route、14／30 天邊界、instance 移除後保留、快照及四個真程序開機（首個在 commit 後硬殺）與新 home 負面對照。這是 store-only，未代替完整 DRV-6／9。`store::claude::tests` 注入交易故障，驗預約／寫出／ACK／人工放棄全部 rollback，且一般 receipt 不能替 Claude push 確認收件。`store` schema fixtures v1–v7 升級並核 v7 投遞／事件樣本。

## 怎麼跑

```bash
cargo test -p agend-daemon
cargo test -p agend-daemon --test store_process      # 只跑跨 process 的
cargo test -p agend --test holder_runtime --test daemon_process   # 第 6 施工關：真 agend、真 holder
cargo test -p agend --test client_protocol                      # 第 8 施工關：真 daemon 的 client protocol
cargo test -p agend-daemon --test codex_driver                  # 第 7 施工關：codex driver 對假 app-server
cargo test -p agend --test codex_process                        # 第 7 施工關：真 daemon、真 holder、sh 包裝、fake_codex
cargo test -p agend --test cli                                  # 第 9 施工關：CLI、重啟與預檢、里程碑
AGEND_BLESS_GOLDEN=1 cargo test -p agend-daemon --test store   # 故意改 schema 後重寫 golden
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `driver::codex::tests` | 長 listen 路徑的 symlink 會被解析到真正的 socket；不存在的路徑回錯誤，不猜 |
| `driver::codex::launch::tests`（第 7 施工關 P2、P4） | trust `-c` 值是 TOML、路徑的引號與反斜線有跳脫；包裝只拿位置參數（路徑不進腳本文字）；用 `echo` 當 codex 實跑包裝：app-server 那行的 `-c` 在子命令前、instance 的 args 在 `--listen` 後，TUI 那行從 `$GO` 讀 thread 與 socket；`$GO` 只寫一次、經暫存檔 rename、不留暫存檔；`prepare` 刪舊 socket 與 `$GO`，不刪 `codex-daemon-<uid>` 以外的目標；工作目錄不存在是錯誤；`.zprofile` 內容一樣就不重寫、啟動 PATH 變了就重寫、單引號有跳脫；**真的 `/bin/zsh -lc`**（macOS 會跑 `path_helper`）加我們的 `ZDOTDIR`：PATH 正好是 shim 目錄、然後啟動時的順序（像 Homebrew 的目錄仍在 `/usr/bin` 前面），`command -v git pkill killall` 都是 shim（沒有 `/bin/zsh` 的機器印 SKIPPED）；`launch_path` 去掉 `$AGEND_HOME/bin` |
| `driver::codex::sweep::tests`（P2） | argv 只比完整元素（`xg7-1.codex.sock`、`thread-10`、`agend-codex` 都不算）；pgid ≤ 1 或 > `i32::MAX` 什麼都不看、不送；讀得到自己子程序的每個 argv 元素；測試自己起的不相干 group（沒有標記）不送訊號、還活著，有標記的 group 兩個程序都被 SIGKILL、之後 `Gone` |
| `driver::codex::send::tests`（P6） | 兩個競態（用照稿回答的連線，假 app-server 沒辦法照時機重現）：`queue/add` 回覆時已閒置 → `queue/start` 一次、`-32600` 算成功，還在忙就不呼叫；steer 撞上剛結束的 turn（`-32600`）→ 一次 `turn/start`，其他錯誤不重試；閒置時三級都是 `turn/start`；忙碌但還不知道 turn id：`Queue` 仍是 `thread/queue/add`，steer／interrupt 用 `turn/start` |
| `driver::codex::history::tests`（P5、P7） | 沒有 `clientId` 時比 turn ＋內容，同一個 turn 兩則相同內容對到兩列、`failed` 的不對、崩潰窗口的 `queued` 只比內容；cursor 是 slot，晚一點才對上的訊息只多一個事件、不改別的 cursor；cursor 的 turn 不見了就從頭 |
| `driver::codex::rpc::tests` | 回應、通知、server→client 請求怎麼分 |
| `delivery::tests` | `From:`／`Task:` 標頭後接完整 body（不截斷） |
| `tests/codex_driver.rs`（第 7 施工關，行程內的假 app-server ＋真 DB） | DRV-1..9 對 `CodexDriver` 全過；三級（`== busy`：閒置 `turn/start`；忙碌時 queue 在 A 之後的新 turn、steer 在 A 裡、interrupt 讓 A `interrupted`，四則都 `confirmed`；被中斷前還沒有 user message 的那則停在 `sent`）；冪等（同 id 再送不多 turn、daemon 重啟後也一樣；同 id 不同內容 `invalid_request`；兩個同 id 同時送只存一次）；崩潰窗口（`queued` 但 codex 收到了：歷史裡的 → `sent`、`confirmed`，佇列裡的 → `sent`，都不再送）；回覆沒回來（`attempted_at`，turn 還在跑、user message 還沒出現）→ 等閒置、在歷史找到、`confirmed`，只有一則 user message、一個 turn（拿掉等待時這條測試失敗：2 個 turn）；狀態不明的那一則在等時，後面的中斷照送、讓跑著的 turn 變 `interrupted`，後面的 `Queue` 訊息跟著等、之後照順序（A 在 B 之前）；人在 TUI 中斷、codex 佇列裡還有訊息：閒置後 driver 送一次 `thread/queue/start`，排隊的開始、`confirmed`（daemon 跑著與重啟後各一次，K16）；關掉的 link 不會連到同一個路徑上的下一個 app-server（沒關的會）；授權回 `decline`；四次開機跨 process（本 binary 重跑：開機 1 送 m-1、m-q，開機 2 閒置，停機時 m-q 自己跑完，開機 3 補回、再送不多 turn，停機時 TUI 自己一輪，開機 4 檢查；也從較舊的 cursor 補回）；反向：每次開機新的 `AGEND_HOME` 在開機 2 失敗；沒有 driver 的 instance 停在 `queued`、`failed` 的 instance → `failed`、還沒連上時 `queued`、連上後送出；thread 找不到：沒送過就換新的，送過就 `connect` 失敗 |
| `tests/store.rs`（第 7 施工關） | migration `0004`：只有 `codex`、沒 thread、`running`／`failed` 而且 `session_started = 1` 的列 → `failed`＋`legacy_no_thread = 1`（`new`、`session_started = 0`、有 thread、別的 backend 不動）；`claim`：同內容 `Existing`、不同內容 `Different`、50 個同時送同一個新 id 只插入一次；`seq` 在 30 天 prune 留下空號後，`VACUUM INTO` 快照還原成 `agend.db` 仍是同樣的號碼、新的接著編；golden 有 `seq … AUTOINCREMENT` 與 `UNIQUE`；30 天 prune 清空表之後新訊息的 `seq` 仍然更大；`schema-v4.sql` 升到 golden |
| `store::tests`（單元） | DB 執行緒 panic 後每個呼叫都回 `store thread stopped`；panic 前 commit 的資料重開還在（P1）；`hard_link` 被 `cfg(test)` 開關弄失敗時新 DB 改用 rename 放上去、已存在的 `agend.db` 不被蓋、失敗訊息寫出步驟（S20） |
| `tests/store.rs` 契約 | `SqliteStoreFixture` 跑整套 STO-1..12（`Persisted` = DB 檔路徑，`boot` 開新連線）；反向：每次開機開新 DB 的 fixture 必須在 STO-4、STO-12 失敗 |
| `tests/store.rs`（第 8 施工關） | migration `0003`：v2 的 `running`、`failed` 的 codex／opencode → `session_started=1`，`new` 與 `failed` 的 claude 留 0；寫 `running` 同時設 1、之後不會變回 0；`CHECK` 擋 2；`tasks()` 依 id 列出 |
| `tests/store.rs` 其他 | 每個 schema 版本（v1、v2、v3）的 fixture 升級到 golden；100 個呼叫者同時讀（P1）；0700／0600、沒有 `-shm`、同一 process 第二次開被拒（P2）；版本從 1 開始、未知 task 附加事件失敗（同 `FakeStore`，#1483）；golden `schema.sql`、每個 schema 版本的 fixture 升級後＝golden 且樣本讀得回、壞 migration 退回、升級前快照、太新的 DB 被拒且檔案 hash 與目錄內容不變（P5）；每張表都有保留規則、`prune` 只刪 14 天前的事件（P8）；0 bytes／比檔頭短／版本 0／版本為負數／缺表的 `agend.db` 被拒且不動、dangling symlink 被拒；留下的 `.agend.db.new` 重建成 0600 的 v1、是 symlink 就拒絕（S17、S19、S21、S22）；UTC 日期、空 DB 不做每日快照也不擠掉舊的（S23）、只有 instance 的 DB 照做（第 6 施工關 verifier r1 F2）、快照 7 份輪替不動其他檔案、刪掉殘留 `.tmp`、快照可唯讀開、還原步驟可用（P9） |
| `boot::tests`（單元） | `plan_boot` 表格測試：頁面 P2 的例子、全空、`new` 第一次全新啟動、`new` 但 holder 在（接回）、`failed` 不動也不算孤兒、只有孤兒 |
| `supervisor::tests`（單元） | 10 分鐘內第 4 次死 → 放棄；超過 10 分鐘的重起不算；claude 第一次 `--session-id`、之後只 `--resume`；沒有 session id（opencode、缺 id 的 claude）不能 resume，只能 `failed`（P6）；codex 不管有沒有 thread、要不要 resume 都起 `/bin/sh` 包裝（第 7 施工關） |
| `runtime::env::tests`（單元） | agent 環境只有白名單：`TELEGRAM_BOT_TOKEN`、`ANTHROPIC_API_KEY`、`AGEND_SHIM_BYPASS`、daemon 的 `TERM` 不給；`PATH` 以 `$AGEND_HOME/bin` 開頭；daemon 沒有 `PATH` 也有預設（P3）；codex 多一個 `ZDOTDIR=$AGEND_HOME/zsh`，其他完全相同（第 7 施工關 P4） |
| `runtime::shims::tests`（單元） | git／gh／kill／killall／pkill 的缺失或指錯 symlink 重建，`bin/` 裡別的檔案不動（P3） |
| `runtime::files::tests`（單元） | 只有 flock 被持有的鎖檔算 holder 在跑；鎖住但沒有活 pid 的檔案絕不回 0、1 或死掉的 pid，掃描時跳過它、其他照常（verifier r1 F3） |
| `housekeeping::tests`（單元） | 假時鐘：daemon log 留 7 天、其他檔案不動；audit 每天輪替、留 14 份（13 份輪替＋今天的）；holder log 只在 holder 不在且 7 天沒動時刪（P8） |
| `fleet::tests`（單元） | 第一個事件＝基準 + 1；還沒有事件時只有基準接得上；「最舊 − 1」到最新接得上，其他 `event_gap`；時鐘往回調：比新 daemon 最新的還大 → `event_gap`，落在新範圍內會被接受（已知風險，所以 1.1 client 重連一律重拿全貌）；沒變的 instance 不發事件；「需要你」加一次、解決一次（第 8 施工關 P4、P5） |
| `supervisor::tests::a_failed_instance_is_a_needs_you_item_with_retry_unless_it_cannot_resume` | P5 的表：claude 一律有 `retry`；opencode push 有持久 session 或尚未建立過 session 才有；codex（第 7 施工關）除了 `legacy_no_thread` 都有；沒有的 `actions` 空、「不處理的話」寫 `delete and re-add the instance (gate 9)` |
| `crates/agend/tests/cli.rs`（第 9 施工關，真 daemon） | `handlers::agent`／`handlers::operator`、`supervisor` 的 add／remove、`preflight`、`reaper`、`daemon` 的 `exec`：見 [agend TESTING](../agend/TESTING.md)（CLI-n 表、重啟 pid 與 holder 不變、預檢失敗 DB 位元組不變、一次一個重啟、繼承的 holder 不留殘屍而自己的 holder 與預檢子程序的 exit status 沒被搶、里程碑） |
| `crates/agend/tests/client_protocol.rs`（CLP-13..17） | 權限兩個方向、`instance_add`／`remove`、`daemon_restart` 的形狀、`task_cancel` 不改狀態、`send` 同 id 只收一次與 `inbox --after`，對真 daemon（同一套也對假 daemon，見 testkit） |
| `store::instances::tests`、`log::tests`（單元） | instance id 只能 `[a-z0-9-]{1,24}`；session id 是 UUID v4；log 檔名與時間戳是 UTC |
| `tests/store_process.rs` | 測試 binary 重新執行自己：四次開機四個 pid、開機 2 只打開、開機 3 重開前的舊版本 CAS 回 `Conflict` 且 task 不變、版本嚴格變大；反向：每次開機用新 DB 路徑必須在開機 3 失敗；硬殺（只對自己的 `Child`）後 ack 過的寫入都在、`integrity_check` ok；第二個程序開 DB 被拒、第一個照常寫入（P2、P6、P7） |

子程序一律：暫存目錄當 home 與 cwd、清掉 `AGEND_*` 環境變數、每個等待都有 60 秒期限（逾時只 kill 自己的子程序）。

## 用到的假實作

- `agend_testkit::tempdir::TempDir`（暫存目錄）
- `agend_testkit::fakes::FakeClock`（保留期限、快照日期）
- `agend_testkit::contract::store`（STO-1..12）

## 還沒測的

- [ ] 斷電：`synchronous=FULL` 但 macOS 沒開 `fullfsync`，斷電可能丟最後幾次 commit；測不到，靠第 10 施工關的 DB ↔ git 對帳（P6）
- [x] 新 daemon 等舊 daemon 放鎖（開 DB 重試）：第 6 施工關 `second_daemon`（實測交接等了約 420 ms）
- [x] `prune`／快照的開機觸發、`audit/shim.jsonl` 輪替：第 6 施工關（每小時那一次只有單元測試，沒有跑一小時的測試）
- [ ] daemon 在啟動或重起 holder 的中途收到 Ctrl-C：事件依序處理，要等那一步做完才結束，可能超過 P1 的 5 秒（見第 6 施工關「待你追認」）；沒有測
- [x] 真正的舊版升級（v1 → v2）：第 6 施工關加 `0002_instances`，`schema-v1.sql` 升到 golden v2（含升級前快照）
- [x] agent runtime 與真 holder（第 6 施工關：`crates/agend/tests/holder_runtime.rs`、`daemon_process.rs`）
- [ ] 真的 claude 收到 `--resume` 後接回對話（本關只用 bash 假 agent 證明 daemon 送了哪些參數；真 backend 在第 12 施工關）
- [x] codex driver 對假 app-server、delivery 的冪等與重連不遺失不重複（第 7 施工關：`tests/codex_driver.rs`）
- [ ] codex driver 對**真** codex：只有使用者選做的 `codex_live`（`AGEND_REAL_CODEX=1`，第 7 施工關步驟 8），CI 不跑
- [ ] 送過一次的訊息，它的 turn 被中斷、user message 永遠沒出現：閒置後會再送一次；真 codex 不認得 `thread/queue/list` 時還在佇列裡的會再送（第 7 施工關 K9）
- [x] protocol server（第 8 施工關：`crates/agend/tests/client_protocol.rs`）
- [ ] 開機很慢（instance 很多、每個 holder 起不來要等 5 秒）時 CLI 的 10 秒會先放棄：本關量到開機 0.1 秒（1–7 個 instance），沒有測大量 instance
- [ ] daemon 在預檢進行中收到 Ctrl-C：預檢子程序自己跑完，`/tmp/agend-pf-*` 會留下（第 9 施工關，沒有測）
- [ ] `exec` 失敗（預檢之後 binary 被刪）：只印錯誤、exit 1，沒有測（第 9 施工關已知風險）
- [x] 第 10 施工關：FakeStore／FakeForge／FakeRunner／FakeDriver／FakeClock 的整條 queue 測試驗 merge、checks 返工、stale、store 失敗與逐 task boot 隔離；transition 驗 CAS conflict 不派 action；`pipeline_store_ports.rs` 對 fake 與 SQLite 同跑結果／receipt 原子提交與 rollback；真 git／Runner／LocalForge 契約、checks 的 metadata／cache／socket 防護、cargo/npm smoke；跨程序的重啟、merge intent/trailer 對帳、持久化請示與人工 attention（`pipeline_adapters.rs`、`agend/tests/pipeline.rs`、`pipeline_tui.rs`）
- [ ] claude／opencode driver、forge github、notifier（第 12 施工關）

`pipeline::tests::failed_human_approval_commit_keeps_attention_and_publishes_no_resolution` 對完整 queue／FakeStore 驗核准 CAS 失敗時保留原 attention，沒有 resolution 或 merge；真程序／CLI 回歸見 agend 的 pipeline_attention_events。

## 下一步

```bash
cargo test -p agend-daemon
cargo xtask accept daemon-holder
```

Failed 派工回歸：五種 fake queue 只用一個 dev，boot 派工失敗後仍可派下一個 task；真程序 `pipeline_review` 以 bindings 暫存路徑故障，恢復後不用 restart 即釋放 terminal binding、worktree 與 assignee。Linux CI 的 canonical repo 寫入回歸涵蓋 `/tmp` 遮蔽後的明確唯讀掛載。

`pipeline_handoff` 使用真 daemon、git、SQLite 與 checks：前段 Branch 作者提交、交接排隊、重啟、後段作者接手，最後 merge 的 tree 必須含兩位作者的檔案，不能只留 archive。

`pipeline_context` 驗 approved Result 的人工 recap／接手訊息與重啟、repo team 的 headless role review，以及 merge recovery 的 main ref 損壞不阻止整機啟動。`daemon::stop_flag` 以獨立子程序在 Tokio runtime 關閉後送真 SIGINT，確保 exec 前仍能看到停止；CLI 原回歸保留 10 秒條件，診斷延遲不留在產品碼。

`pipeline_archive` 用三種各 6 MiB 的 binary（未 merge commit、tracked 修改、untracked）驗 archive 的 `git apply` round trip 與 bytes 相等；另驗 archive I/O 失敗保留原 WIP、下一次 wake 重試，以及 foreign checks worktree／tmp 保留、命名空間內孤兒仍清理。

`pipeline_archive_history` 以真 merge 衝突建立只在 merge commit 出現的解法，再加未 commit 修改；取消後 `git apply` 必須還原最終 bytes。CLP-14 仍要求 instance-add 成功後立即 get_fleet 就可見，supervisor 先投影 Starting 再回覆成功。

`pipeline_archive_index` 經真 shim stage 6 MiB binary、刪除工作目錄檔案；取消後從 archive 分出 index／worktree patch，還原相同 AD 狀態與全部 bytes。另驗 unresolved index 保存失敗時仍保留各 stage 與原 worktree，不能發布假完整 archive。

`pipeline_archive_flags` 經真 shim 設定 `skip-worktree`／`assume-unchanged`，驗取消後 patch 還原實際 bytes（含 split index）；另驗私有 index 檢查後保存失敗時，原 index bytes 與 WIP 都保留；canonical main 的隱藏修改會阻擋 merge，author 的隱藏修改在需要 rebase 時保留原 head 與 bytes。

`pipeline_archive_diff` 經真 shim 設定 external diff／textconv，原 diff 為空仍須保留 staged bytes，取消後以 patch 還原 unstaged 與 untracked bytes；archive 與 patch-id 都停用顯示轉換；另驗 ignored local notes 還原、content filter 保存失敗時保留原始 WIP、patch-id 不受顯示設定影響，以及真 normalization fixture 的 raw bytes 修改不能通過 clean 檢查。

`pipeline_archive_racy` 使用真 Git 產生 cached stat，再以 minimal stat／忽略 ctime／相同長度與 mtime 產生 WIP；確認 fresh-mtime 的 index 副本會隱藏修改，但取消封存仍還原實際 bytes。新 index 由 stage entries 重建，不帶 stat cache 或隱藏旗標；旗標回歸另含 core.ignoreStat 設定。

`pipeline_workflow` 證明抽象 core 可表示 count=2 human approval，但本 runtime 的 workflow check／apply／task create 均提早拒絕，拒絕後不能留下 task；內建 count=1 workflow 仍有效。

`pipeline_archive_eol` 經真 daemon／shim 設定 core.autocrlf=input／true 與 text／text=auto／eol／legacy crlf attributes；取消時須回報 Failed，保留原 worktree、CRLF 的全部 bytes 與原 index，不發布會遺失 CR bytes 的 patch。

`pipeline_archive_nested` 用真 Git 建 nested repo／staged gitlink，取消須保留內部資料、Git metadata 與原 index；一般未追蹤子目錄、空檔、symlink 則以真 git apply 還原。`pipeline_archive_index` 只檢查已發布的 .patch，避免每次 wake 的暫存 staging 造成競態。

`pipeline_archive_display` 以真 daemon／shim 設 color.ui／color.diff=always 與 shared diff.noprefix，取消後須用預設 git apply 還原 commit／index／worktree 與原資料內 ESC bytes；patch-id 也不受顏色／prefix 影響。`pipeline_archive_nested` 另移除 inner HEAD、留下只有 Git objects／index 保存的 staged binary，Git 看不到其 metadata 時仍須保留全部資料。

## 第 12B OpenCode

第 12B 的傳輸／session／歷史核對以 `cargo test -p agend-daemon driver::opencode --lib` 驗證。原生假 producer 覆蓋 create／resume／busy／abort／history；1.18.34 真 `noReply` 捕獲覆蓋指定訊息 ID、中文／換行與防止誤認。這些案例不啟動模型，尚不代表正式 Driver／holder 恢復或完整 12B 驗收。

## 第 12A Claude

Claude bridge／Driver／native startup capture 的案例、限制與重驗指令見 [Claude 測試](CLAUDE-TESTING.md)。

2026-10-06 [D41](../../docs/decisions/d41.md) 允許 Ready 建議內容可變，其他完整畫面／版本／路徑／尺寸不變。
`claude_startup::startup_variable_ready_suggestions_replay_actual_v5_and_both_widths` 經真 daemon／holder／PTY 重播 v5 兩份捕獲及 140 欄變體，核五秒初始 idle 與 Ready 不加鍵；
`startup_variable_ready_rejects_unknown_footer_and_split_hint_without_idle_or_more_keys` 拒絕未知 footer／分行建議。
既有無 SessionStart、人工控制、結果不明與四次開機回歸維持；這些測試不啟動真 Claude、不送模型訊息。

OpenCode worker 原生 producer 案例含 140 筆結果不明前綴、獨立新投遞批次與一次確認事件；另以 `lost_native_mutation_replies_are_not_replayed_after_store_reopen` 注入 producer 已套用 POST 後直接斷線：prompt 由 history 確認、abort 結果不明不補送 prompt，跨三次 SQLite 重開均不重送。supervisor 測試另覆蓋保存 session 的 wrapper resume、缺 session 拒絕與不支援的參數拒絕。

12B permission 原生 producer 測試核對完整請求、session／request identity 與拒絕後失效；SQLite reopen 後決策 attempt 不可重取。schema v10 及 v1–v10 fixture 升級保存既有資料；permission 歷史保留至明確移除 instance，透過 FK cascade 刪除。handler／正式 socket／holder 的權限回覆與重啟由 `opencode_bridge` 覆蓋；真模型權限證據是 REST API 捕獲，沒有宣稱真模型 TUI 權限端到端驗收。

`handlers::opencode_attention` 回歸驗證 agent 不能 AnswerAsk、operator free text 不會取得 permission attempt。API 原生 producer 案例使用同一 `send_decision` 路徑驗拒絕、重複回覆不送與待處理紀錄消失；正式 socket／holder 端到端案例已納入 `opencode_bridge`。

12B schema v11 保存 REST 回合事件去重；`driver::opencode` 驗正常 terminal assistant／abort、外來 assistant part、刪除 driver_events 後歷史不重新發布同一回合。schema v12 另保存 attempt 歸屬與未終結保留例外，store fixtures 核各版升級／golden／retention。完整 DRV 10/10 已通過；429 只有原生 producer 契約覆蓋，未蒐集真服務限額回覆。

`driver::opencode::contract_tests` 直接使用未改動的 DRV 10 個案例，production Driver／Worker、原生 REST producer 與 SQLite。boot 結束會停 worker／關 DB，重新 boot 回填後服務游標；停機時直接經原生 API 讓 backend 完成回合。這是 Driver 契約層，真 daemon／holder 的兩條恢復路徑另由 `agend --test opencode_bridge` 驗。

OpenCode `real_11834_model_capture_confirms_delivery_and_one_terminal_turn` 使用正式 daemon 的真模型 history，檢查完整訊息確認、terminal 完成、未完成排除與外來 part 拒絕；一般測試不呼叫模型。`runtime::env::tests` 同時核 OpenCode／Codex 的 `ZDOTDIR` 與 env 白名單，沿用真 login zsh 的 shim-first 測試。

`cargo test -p agend --test claude_bridge three_backend_delivery` 以同一正式 daemon 啟動三種 backend 的 native producers，檢查六方向 sender／Unicode／newline 與 Confirmed；Claude 透過正式 channel helper 明確 ACK，再發 native Stop。需先 build agend-testkit bins，不呼叫模型。

`opencode_bridge::unknown_delivery_survives_restart_and_only_operator_can_abandon_it` 透過正式 socket／daemon 驗證結果不明提示跨重啟、agent Forbidden、operator Abandon、晚到確認不復活及 native history 未被重送；沒有真模型呼叫。

OpenCode `oversized_total_history_does_not_block_old_receipts_or_new_delivery` 建立超過 16 MiB 的 native REST history，先核全量讀取失敗，再核分頁／縮小頁數、舊 attempt 定點確認、新訊息送達與早期完成去重。單筆本身超限仍明確拒絕。

`chunked_oversized_json_reports_the_same_limit_as_content_length` 核對無 Content-Length 的 chunked 超限回覆也可觸發分頁縮小，避免長歷史卡住。

權限 native API 測試注入 session／permission GET 503：原版本會耗用尚未送出的 POST attempt，修正版保留 operator 答覆機會；另注入已套用 permission POST 後丟回覆，重開資料庫必須保持 unknown 且不能再 POST。

12D `cargo test -p agend-daemon --lib notifier::` 驗 config allowlist、拒 inline secret、private file／symlink 邊界，以及 native HTTP 的真 getMe fixture／redirect／malformed response／429 安全錯誤。同組測試亦跑全套 NTF、長 Unicode 通知、失去或損壞第二段收據後 DB 重開不重送。`cargo test -p agend-daemon --lib store::telegram` 驗持久 CAS；`cargo test -p agend-daemon --test store` 驗 schema migration／golden。尚非手機操作完成認證。

12D outbound worker 測試以真 Fleet／SQLite／native HTTP 驗通知、停機收據與等待時間改變不重送；store observer 驗 DB 重開保持 delivery id、重複 source rollback、內容更新／解除／再開建立新通知。

12D inbound 的 `notifier::inbound` 測試以已捕獲的真 Message 收據和正式 keyboard producer 驗身分／allowlist／選項／修改原因、未確認或已消耗通知拒絕、不同 update ID 與重開不重播。Callback envelope 目前是 schema adversary，不是真手機 callback 證據。`pipeline_store_ports` 核注意事項版本持久化；`pipeline::tests::mobile_guard` 核 CLI／TUI 清除再開同原因後舊版本不得操作。`store` 核 instance failure episode 跨 boot 穩定、再次失敗更新，以及 schema 15 的舊版升級。

`notifier::poll_tests` 使用 native HTTP 與 recorded Message schema，實際執行 SQLite claim、production pipeline acknowledgment，檢查未授權拒絕、一次作用與 stale callback 回覆。另一案例在 RetryConfirmed 等待中丟棄 receiver／event，驗 cancellation 不移除事項、不回 Accepted；它是 queue 邊界測試，並未啟動真 daemon 程序。

共用已讀：`cargo test -p agend --test shared_read` 以兩個真 TUI client／daemon 程序驗同步與重啟保留；daemon `mobile_read_` 驗 native HTTP 按鈕不消耗原動作，`a_followup_remains_unread` 驗舊 read key 拒絕，store `read_receipts_` 驗 DB 重開。TUI `a_disconnected_source_` 驗斷線不能宣稱保存成功。上述不含真 Telegram 手機操作。

`cargo test -p agend-daemon --lib notifier::` 包含 native HTTP 的 needs-you／雙 team topics、任務狀態更新、DB 重開不重送摘要，以及輔助 outbox pending 恢復、unknown／foreign bot 不送出。這些不宣稱真 Telegram forum 驗收。

`notifier::poll_tests::native_mobile_choice_and_free_reply_reach_the_asking_agent_once` 以真 pipeline 建立問答及追問、原生 HTTP producer 收據與不同 message ID，驗選項／多行自由文字經 Telegram source 保存且各入 inbox 一次；重複輪詢／舊通知不回答新追問。沒有啟動 backend 模型。

`native_mobile_approval_and_changes_require_current_receipt_and_explicit_reason` 以 core state machine 產生已交付的 research 結果，在正式 serialized pipeline 等 human approval，經本機 HTTP 收據／callback 核 approve 完成；request_changes 先提示、不執行，空白拒絕、多行理由完整保存後回 work。完成後新 update ID 重用舊按鈕不改 pipeline。此例 bind_head=false，不代替 Git head／merge 驗收。

Telegram unknown 測試核 active claim 不提早發布、HTTP 收據遺失／毀損標 unknown、agent／Retry 拒絕、operator Abandon 不造收據、不重送，DB 重開保留處置與原文。`agend --test telegram_unknown` 另以正式 daemon 三次開機、無 token／設定核啟動恢復、操作員 socket 權限及持久處置。

12D `cargo test -p agend-daemon --lib daemon::telegram_tests::active_shutdown` 用兩個獨立子程序執行正式 daemon `serve`，本機 HTTP producer 扣住第一段回覆。SIGINT 後 socket 已移除但程序仍等收據；放行後 SQLite 保存第一段，重啟只送第二段並完成。僅測試編譯可注入 loopback API，正式 origin 不變；ignored `child` 是父測試啟動的子程序入口。這驗 auxiliary outbox active shutdown，不涵蓋 Retry 排隊關機。
