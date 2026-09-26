# agend-daemon 測試

> **TL;DR**
> - store 的測試都用暫存目錄裡的真 DB 檔（不用 in-memory SQLite）；重開與硬殺跨真的 process 驗（第 5 施工關 P6、P7）。
> - daemon ↔ holder（第 6 施工關）：純邏輯在本 crate 單元測試；要真 `agend` binary 的測試在 `crates/agend/tests/`（`holder_runtime.rs`、`daemon_process.rs`），各段與 `daemon_probe demo` 共用 `tests/common/daemon_process.rs`。
> - 記住：每個領域模組都要能對 testkit 的假實作單獨測；每個 adapter 要跑契約測試。
> - client protocol（第 8 施工關）：`fleet` 的游標規則是單元測試；真 daemon 的 CLP 契約與 socket／`retry`／終端／重啟在 `crates/agend/tests/client_protocol.rs`，各段與 `client_demo` 共用 `tests/common/client_process.rs`。
> - 下一步：`cargo test -p agend-daemon`；看 demo：`cargo xtask accept store`、`cargo xtask accept client`。

## 怎麼跑

```bash
cargo test -p agend-daemon
cargo test -p agend-daemon --test store_process      # 只跑跨 process 的
cargo test -p agend --test holder_runtime --test daemon_process   # 第 6 施工關：真 agend、真 holder
cargo test -p agend --test client_protocol                      # 第 8 施工關：真 daemon 的 client protocol
AGEND_BLESS_GOLDEN=1 cargo test -p agend-daemon --test store   # 故意改 schema 後重寫 golden
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `driver::codex::tests` | 長 listen 路徑的 symlink 會被解析到真正的 socket；不存在的路徑回錯誤，不猜 |
| `store::tests`（單元） | DB 執行緒 panic 後每個呼叫都回 `store thread stopped`；panic 前 commit 的資料重開還在（P1）；`hard_link` 被 `cfg(test)` 開關弄失敗時新 DB 改用 rename 放上去、已存在的 `agend.db` 不被蓋、失敗訊息寫出步驟（S20） |
| `tests/store.rs` 契約 | `SqliteStoreFixture` 跑整套 STO-1..12（`Persisted` = DB 檔路徑，`boot` 開新連線）；反向：每次開機開新 DB 的 fixture 必須在 STO-4、STO-12 失敗 |
| `tests/store.rs`（第 8 施工關） | migration `0003`：v2 的 `running`、`failed` 的 codex／opencode → `session_started=1`，`new` 與 `failed` 的 claude 留 0；寫 `running` 同時設 1、之後不會變回 0；`CHECK` 擋 2；`tasks()` 依 id 列出 |
| `tests/store.rs` 其他 | 每個 schema 版本（v1、v2、v3）的 fixture 升級到 golden；100 個呼叫者同時讀（P1）；0700／0600、沒有 `-shm`、同一 process 第二次開被拒（P2）；版本從 1 開始、未知 task 附加事件失敗（同 `FakeStore`，#1483）；golden `schema.sql`、每個 schema 版本的 fixture 升級後＝golden 且樣本讀得回、壞 migration 退回、升級前快照、太新的 DB 被拒且檔案 hash 與目錄內容不變（P5）；每張表都有保留規則、`prune` 只刪 14 天前的事件（P8）；0 bytes／比檔頭短／版本 0／版本為負數／缺表的 `agend.db` 被拒且不動、dangling symlink 被拒；留下的 `.agend.db.new` 重建成 0600 的 v1、是 symlink 就拒絕（S17、S19、S21、S22）；UTC 日期、空 DB 不做每日快照也不擠掉舊的（S23）、只有 instance 的 DB 照做（第 6 施工關 verifier r1 F2）、快照 7 份輪替不動其他檔案、刪掉殘留 `.tmp`、快照可唯讀開、還原步驟可用（P9） |
| `boot::tests`（單元） | `plan_boot` 表格測試：頁面 P2 的例子、全空、`new` 第一次全新啟動、`new` 但 holder 在（接回）、`failed` 不動也不算孤兒、只有孤兒 |
| `supervisor::tests`（單元） | 10 分鐘內第 4 次死 → 放棄；超過 10 分鐘的重起不算；claude 第一次 `--session-id`、之後只 `--resume`；沒有 session id（codex、opencode、缺 id 的 claude）不能 resume，只能 `failed`（P6） |
| `runtime::env::tests`（單元） | agent 環境只有白名單：`TELEGRAM_BOT_TOKEN`、`ANTHROPIC_API_KEY`、`AGEND_SHIM_BYPASS`、daemon 的 `TERM` 不給；`PATH` 以 `$AGEND_HOME/bin` 開頭；daemon 沒有 `PATH` 也有預設（P3） |
| `runtime::shims::tests`（單元） | 缺的或指錯的 shim symlink 重建，`bin/` 裡別的檔案不動（P3） |
| `runtime::files::tests`（單元） | 只有 flock 被持有的鎖檔算 holder 在跑；鎖住但沒有活 pid 的檔案絕不回 0、1 或死掉的 pid，掃描時跳過它、其他照常（verifier r1 F3） |
| `housekeeping::tests`（單元） | 假時鐘：daemon log 留 7 天、其他檔案不動；audit 每天輪替、留 14 份（13 份輪替＋今天的）；holder log 只在 holder 不在且 7 天沒動時刪（P8） |
| `fleet::tests`（單元） | 第一個事件＝基準 + 1；還沒有事件時只有基準接得上；「最舊 − 1」到最新接得上，其他 `event_gap`；時鐘往回調：比新 daemon 最新的還大 → `event_gap`，落在新範圍內會被接受（已知風險，所以 1.1 client 重連一律重拿全貌）；沒變的 instance 不發事件；「需要你」加一次、解決一次（第 8 施工關 P4、P5） |
| `supervisor::tests::a_failed_instance_is_a_needs_you_item_with_retry_unless_it_cannot_resume` | P5 的表：claude 一律有 `retry`；codex／opencode 只有 session 沒建立過才有，否則 `actions` 空、「不處理的話」寫 `delete and re-add the instance (gate 9)` |
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
- [ ] codex driver 對假 app-server、delivery 的冪等與重連不遺失不重複（第 7 施工關）
- [x] protocol server（第 8 施工關：`crates/agend/tests/client_protocol.rs`）
- [ ] 開機很慢（instance 很多、每個 holder 起不來要等 5 秒）時 CLI 的 10 秒會先放棄：本關量到開機 0.1 秒（1–7 個 instance），沒有測大量 instance
- [ ] pipeline、git、runner、forge local、supervisor、reconcile（第 10 施工關）
- [ ] claude／opencode driver、forge github、notifier（第 12 施工關）

## 下一步

```bash
cargo test -p agend-daemon
cargo xtask accept daemon-holder
```
