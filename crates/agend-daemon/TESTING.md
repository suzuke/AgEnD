# agend-daemon 測試

> **TL;DR**
> - store 的測試都用暫存目錄裡的真 DB 檔（不用 in-memory SQLite）；重開與硬殺跨真的 process 驗（第 5 施工關 P6、P7）。
> - 記住：每個領域模組都要能對 testkit 的假實作單獨測；每個 adapter 要跑契約測試。
> - 下一步：`cargo test -p agend-daemon`；看 demo：`cargo xtask accept store`。

## 怎麼跑

```bash
cargo test -p agend-daemon
cargo test -p agend-daemon --test store_process      # 只跑跨 process 的
AGEND_BLESS_GOLDEN=1 cargo test -p agend-daemon --test store   # 故意改 schema 後重寫 golden
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `driver::codex::tests` | 長 listen 路徑的 symlink 會被解析到真正的 socket；不存在的路徑回錯誤，不猜 |
| `store::tests`（單元） | DB 執行緒 panic 後每個呼叫都回 `store thread stopped`；panic 前 commit 的資料重開還在（P1）；`hard_link` 被 `cfg(test)` 開關弄失敗時新 DB 改用 rename 放上去、已存在的 `agend.db` 不被蓋、失敗訊息寫出步驟（S20） |
| `tests/store.rs` 契約 | `SqliteStoreFixture` 跑整套 STO-1..12（`Persisted` = DB 檔路徑，`boot` 開新連線）；反向：每次開機開新 DB 的 fixture 必須在 STO-4、STO-12 失敗 |
| `tests/store.rs` 其他 | 100 個呼叫者同時讀（P1）；0700／0600、沒有 `-shm`、同一 process 第二次開被拒（P2）；版本從 1 開始、未知 task 附加事件失敗（同 `FakeStore`，#1483）；golden `schema.sql`、每個 schema 版本的 fixture 升級後＝golden 且樣本讀得回、壞 migration 退回、升級前快照、太新的 DB 被拒且檔案 hash 與目錄內容不變（P5）；每張表都有保留規則、`prune` 只刪 14 天前的事件（P8）；0 bytes／比檔頭短／版本 0／版本為負數／缺表的 `agend.db` 被拒且不動、dangling symlink 被拒；留下的 `.agend.db.new` 重建成 0600 的 v1、是 symlink 就拒絕（S17、S19、S21、S22）；UTC 日期、空 DB 不做每日快照也不擠掉舊的（S23）、快照 7 份輪替不動其他檔案、刪掉殘留 `.tmp`、快照可唯讀開、還原步驟可用（P9） |
| `tests/store_process.rs` | 測試 binary 重新執行自己：四次開機四個 pid、開機 2 只打開、開機 3 重開前的舊版本 CAS 回 `Conflict` 且 task 不變、版本嚴格變大；反向：每次開機用新 DB 路徑必須在開機 3 失敗；硬殺（只對自己的 `Child`）後 ack 過的寫入都在、`integrity_check` ok；第二個程序開 DB 被拒、第一個照常寫入（P2、P6、P7） |

子程序一律：暫存目錄當 home 與 cwd、清掉 `AGEND_*` 環境變數、每個等待都有 60 秒期限（逾時只 kill 自己的子程序）。

## 用到的假實作

- `agend_testkit::tempdir::TempDir`（暫存目錄）
- `agend_testkit::fakes::FakeClock`（保留期限、快照日期）
- `agend_testkit::contract::store`（STO-1..12）

## 還沒測的

- [ ] 斷電：`synchronous=FULL` 但 macOS 沒開 `fullfsync`，斷電可能丟最後幾次 commit；測不到，靠第 10 施工關的 DB ↔ git 對帳（P6）
- [ ] 新 daemon 等舊 daemon 放鎖（開 DB 重試）：第 6 施工關
- [ ] `prune`／快照的開機與每 24 小時觸發、`audit/shim.jsonl` 輪替：第 6 施工關
- [ ] 真正的舊版升級（v1 → v2）：第一個新 migration 的 PR 附 `schema-v2.sql` 時才有
- [ ] agent runtime 與真 holder（第 6 施工關）
- [ ] codex driver 對假 app-server、delivery 的冪等與重連不遺失不重複（第 7 施工關）
- [ ] protocol server（第 8 施工關）
- [ ] pipeline、git、runner、forge local、supervisor、reconcile（第 10 施工關）
- [ ] claude／opencode driver、forge github、notifier（第 12 施工關）

## 下一步

```bash
cargo test -p agend-daemon
cargo xtask accept store
```
