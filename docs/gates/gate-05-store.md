# 第 5 施工關：agend-daemon：store（`store`）

> **TL;DR**
> - daemon 的 SQLite：schema、migration、CAS、保留期限、每日 DB 快照；所有持久狀態只在這裡（D8）。
> - 記住：**重開不遺失要用真的 DB 檔、跨真的 process 驗**（第 2 施工關 A25）；in-memory DB 正是要擋的那類實作。
> - 下一步：draft PR（build first, merge later，使用者放寬 D22）等 fresh-context verifier、你親自驗收與「待你追認」S1–S16；不 merge。

## 狀態

**實作中**（2026-09-26）：P1–P9 已實作於 draft PR（branch `feat/gate-05-store`）；使用者允許先建、之後才 merge（D22 放寬），PR 維持 draft。自動驗收由實作者跑過（見「進度紀錄」），fresh-context verifier 與你親自驗收尚未做。

## 範圍

- `store` 模組：專屬 DB 執行緒、唯一連線（P1）
- `$AGEND_HOME/agend.db`、權限、同時只有一個程序能開（P2）
- 三張表：`tasks`、`workflows`、`task_events`；實作 `Store` trait，外加 `save_workflow`（P3）
- `PipelineState` 怎麼存：本關只定方向，第 10 施工關實作（P4）
- 只往前的 migration、太新的 DB 拒絕開、migration 前先做 DB 快照（P5）
- WAL、`synchronous=FULL`、崩潰測試（P6）
- 契約 STO-1..12 對真 store 跑；STO-4、STO-12 跨真的 process 四次開機（P7）
- 保留期限 `prune(now)` 與校準；第 3 施工關 T10 的 audit 紀錄輪替也納入同一份規則（P8）
- 每日 DB 快照與 7 份輪替（P9）

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子。文件已定、這裡不重問的：SQLite 是唯一持久狀態（D8）；daemon 用一個 multi-thread tokio runtime，SQLite 由專屬執行緒經 channel 存取（ARCHITECTURE；v1 兩天 861 次 scanner-thread slip）；CLI 與 shim 不開 DB；保留期限的數字與快照 7 份（D31）；workflow 有版本、task 固定建立時的版本（D19、D21）；契約 STO-1..12、四次開機、跨 process 驗重開（A25）；`FakeStore` 已選的行為（版本從 1 開始、對不存在的 task 附加事件會失敗）真 store 照做（#1483）。

### P1：SQLite crate、怎麼接到 async

- 問題：用哪個 crate？`Store` trait 是 async，DB 執行緒是同步的，怎麼接？
- 建議：`rusqlite` 開 `bundled`（SQLite 編進 binary，版本由 Cargo.lock 鎖住，macOS 與 Linux 一致，保證有 `STRICT` 與 `VACUUM INTO`）。一條 std thread `agend-db` 持有唯一的 `Connection`；async 方法把 closure 丟進 `tokio::sync::mpsc`（容量 256），DB 執行緒 `blocking_recv`，結果經 `oneshot` 送回；tokio 只開 `sync` feature（testkit 的 `block_on` 也能驅動）。DB 執行緒 panic 之後每個呼叫都回 `store thread stopped`，不重試；daemon 結束、由 launchd／systemd 重新拉起。
- 理由：一條執行緒、一條連線就是單一寫者，不必管連線池與鎖，約 40 行。
- 替代方案：`sqlx`（async、連線池、依賴多）；`tokio-rusqlite`（同樣的事、多一個依賴）；每次 `spawn_blocking`（多條連線、失去單一寫者）。
- 例子：100 個 async task 同時 `load_task`，依序在 `agend-db` 執行，沒有 `database is locked`。
- [x] 使用者確認（2026-09-25）

### P2：DB 檔放哪、誰能開

- 問題：路徑與權限？怎麼保證只有 daemon 在寫？
- 建議：`$AGEND_HOME/agend.db`（home 0700、`agend.db` 建立時就是 0600、`backups/` 0700；store 由呼叫端傳入 home，不讀環境變數）。連線設 `PRAGMA locking_mode=EXCLUSIVE`，打開時立刻讀一次拿鎖；第二個程序一開就失敗：`agend.db is in use by another process (is another agend daemon running?)`。其他程序一律經 daemon 協定；`check-deps` 加規則：`agend-tui` 不能依賴 SQLite 或 `agend-daemon`。手動用 `sqlite3` 查資料時開 DB 快照（P9）。
- 理由：程序結束或被硬殺時 kernel 放掉鎖；兩個 daemon 同時驅動同一批 task 是最難查的錯，一行 pragma 就擋掉。
- 替代方案：一般鎖定模式讓 `sqlite3` 直接開，「只有一個 daemon」改用另外的鎖檔。
- 例子：daemon 跑著時再起一個，它印出 `agend.db is in use…`、exit 1，第一個不受影響。
- [x] 使用者確認（2026-09-25）

### P3：建哪些表、task 怎麼存

- 問題：`store.rs` 列了 11 種資料，這關全建嗎？task 存欄位還是 JSON？
- 建議：只建有使用者的三張 `STRICT` 表：`tasks`（每個欄位一欄＋`version`）、`workflows`（`(id, version)` 主鍵、內容存 D19 的 TOML 原文）、`task_events`（遞增 `seq`、`task_id` 外鍵）。其他表由用到的施工關在自己的 migration 加。`Task` 手寫對應欄位，對應時整個拆開（`let Task { id, title, … } = task;`），core 加欄位就編譯不過，逼人補欄位與 migration；`depends_on` 存 JSON 陣列文字、`status` 存字面值。CAS：`UPDATE tasks SET …, version = version + 1 WHERE id = ? AND version = ?`，改到 0 列回 `Conflict{current_version}`。`save_workflow` 是 store 自己的方法（契約的 `insert_workflow` 與第 9 施工關 `agend workflow apply` 都用它）。
- 理由：還沒使用者的表是猜的 schema；core 型別不加 serde derive，不動 D32、不必重跑第 1 施工關；task 不刪、版本只增，沒有 ABA。
- 替代方案：現在建 11 種表（猜的）；`Task` 存整塊 JSON（要擴 D32、`sqlite3` 難查）。
- 例子：core 替 `Task` 加 `priority`，`cargo build` 在 store 的拆開那一行報錯。
- [x] 使用者確認（2026-09-25）

### P4：流水線進度怎麼存（第 1 施工關 P7 留到本關）

- 問題：`PipelineState` 欄位私有、只能從 `new` 加事件得到；daemon 重啟後怎麼找回？
- 建議：本關只定方向，第 10 施工關實作：「目前狀態」存成 task 列上的一欄、與 task 一起 CAS，不重播事件。第 10 施工關：加 migration；D32 擴到 `PipelineState` 與其欄位型別，用 golden JSON 鎖格式；只准加欄位（比照 D26）。本關的保留期限因此可以把事件當「只給人看的歷史」刪。
- 理由：事件只留 14 天（D31），重播的話跑超過 14 天的 task 重建不回來；core 修了 `step` 的 bug 後，重播舊事件可能得到不同狀態。
- 替代方案：重播事件（未結束 task 的事件不能刪，升級可能改寫歷史）。
- 例子：task 在 review 卡 20 天，第 15 天前的事件已刪；第 20 天重啟，task 仍在 review、第 2 次、核准者都在。
- [x] 使用者確認（2026-09-25）

### P5：migration

- 問題：schema 怎麼升級？能不能降級？舊 binary 碰到新 schema 怎麼辦？怎麼測？
- 建議：只往前。`crates/agend-daemon/src/store/migrations/0001_init.sql`… 用 `include_str!` 編進 binary；版本記在 `PRAGMA user_version`；每個 migration 一個 transaction，失敗整個退回。打開時先讀版本，之後才做會寫入的事：DB 比 binary 新 → 拒絕開，印出 `agend.db schema version N is newer than this agend supports (M); install a newer agend or restore a snapshot from <home>/backups`，檔案一個 byte 都不改；需要升級 → 先做一份 DB 快照（`agend-YYYY-MM-DD-pre-vN.db`，算在 P9 的 7 份裡）再升。降級＝停 daemon、還原那份快照。自己寫約 40 行。測試：全新 DB 升到最新＝golden `schema.sql`；每個舊版一份 fixture（`fixtures/schema-vN.sql` 含樣本）升級後＝golden、樣本讀得回來（新增 migration 的 PR 要附前一版 fixture）；壞 migration（清單是參數，不留測試後門）→ `user_version` 不變；太新的 DB 被拒、檔案 hash 不變。
- 理由：升級失敗就退回、DB 沒被動；已升級則舊 binary 拒開，不會用錯 schema 亂寫；fixture 比對也抓到「有人改了舊 migration」。
- 替代方案：down migration（兩倍 SQL、很少被測）；`rusqlite_migration` crate。
- 例子：升到 v0.3 後裝回 v0.2，daemon 印「schema version 2 is newer…」並結束；還原 `backups/agend-…-pre-v2.db` 後 v0.2 照常跑。
- [x] 使用者確認（2026-09-25）

### P6：耐久設定與崩潰安全

- 問題：commit 回來後資料真的在磁碟上嗎？硬殺或斷電會怎樣？
- 建議：`journal_mode=WAL`、`synchronous=FULL`、`foreign_keys=ON`，加上 P2 的 `locking_mode=EXCLUSIVE`；macOS `fullfsync` 不開。崩潰測試：子程序連續 CAS，每次 commit 回來印 `acked <version>`；父程序收到 50 行後用自己的 `Child::kill()` 硬殺；新程序打開 DB，版本 ≥ 最後 ack、`PRAGMA integrity_check` 是 `ok`。斷電測不到，寫進 TESTING「還沒測的」。
- 理由：pipeline 規則是「先寫 DB、再建 worktree」，ack 過的寫入不能在崩潰後不見；寫入量小，每次 commit 一次 fsync 負擔得起。
- 替代方案：`synchronous=NORMAL`（斷電可能丟最後幾次）；開 `fullfsync`（macOS 每次 commit 多幾 ms）；rollback journal（寫入擋讀取）。
- 例子：崩潰測試印出 `acked=50 found=50 integrity=ok`。
- [x] 使用者確認（2026-09-25）

### P7：契約怎麼跨真的 process 跑

- 問題：契約 suite 在同一個 process 裡，分不出「真的存下來」還是「藏在 process 裡」（A25）。
- 建議：同 process：`SqliteStoreFixture` 的 `Persisted` 是 DB 檔路徑，`boot` 每次開新連線，不用 shared cache、不用 `static`，整套 STO-1..12 對真 store 跑。跨 process：`crates/agend-daemon/tests/store_process.rs` 用 `current_exe()` 重新執行自己，每次開機一個子程序（環境變數決定步驟）：(1) 建 task、workflow、事件、做幾次 CAS；(2) 只打開 DB；(3) 檢查、拿重開前的舊版本 CAS 要回 `Conflict`、再寫一次；(4) 全部檢查，版本比之前都大。父程序等每個子程序結束才開下一個，四個 pid 都不同。反向檢查：同流程「每次開機用新的 DB 路徑」必須失敗。P6 的崩潰測試也用這套。安全：暫存目錄；只對自己的 `Child` 送 kill；不用 `pkill`、不送 signal 給別人的 pid。
- 理由：不必為測試多一個 production 子命令，也不改 testkit（不必重過第 2 施工關）。
- 替代方案：拆 testkit 的 `daemon_lifecycle` 讓契約 case 本身跨 process（要改第 2 施工關）；隱藏子命令 `agend store-probe`。
- 例子：`boot 1 pid=4101 v=4`、`boot 2 pid=4107 (open only)`、`boot 3 pid=4112 stale v=4 -> conflict current=4, v=5`、`boot 4 pid=4118 v=5 ok`。
- [x] 使用者確認（2026-09-25）

### P8：保留期限

- 問題：D31 的期限怎麼落實？誰觸發？怎麼校準？
- 建議：store 提供 `prune(now_unix_ms) -> PruneReport`（時間由 `Clock` 給，一個 transaction，不 VACUUM）。規則寫成程式裡的一張表：`task_events` 依 `occurred_at_unix_ms` 留 14 天；`tasks`、`workflows` 永久。測試：`sqlite_master` 每張表都要出現在規則表（N 天或永久），加表忘了寫就失敗。第 3 施工關 T10 的 `audit/shim.jsonl` 輪替也列進同一份規則（使用者 2026-09-25 決定）。觸發：daemon 開機一次、之後每 24 小時（接線在第 6 施工關）；本關只做方法與 demo。校準：demo 產生 v1 量級合成資料（task 約 8,300 筆），印出 DB 與 7 份快照總大小；DB ≤ 1 GB、7 份快照 ≤ 5 GB 就維持 D31。
- 理由：規則表加測試，讓「每張表都有期限」從註解變成會失敗的檢查（v1 的 home 長到 161G）。
- 替代方案：SQLite trigger 寫入時順便刪（難測）；刪完 VACUUM（擋寫入）；不校準。
- 例子：假時鐘往前撥 15 天，`task_events 120 -> 0`、`tasks 3 -> 3`。
- [x] 使用者確認（2026-09-25，含 audit 輪替）

### P9：每日 DB 快照

- 問題：快照放哪、叫什麼、何時做、怎麼確認能用、怎麼還原？
- 建議：`$AGEND_HOME/backups/agend-YYYY-MM-DD.db`（UTC 日期），今天的不存在才做；開機一次、之後每 24 小時（先 prune 再快照）。做法：`VACUUM INTO backups/.agend-YYYY-MM-DD.db.tmp` → 唯讀打開跑 `PRAGMA quick_check` → rename → 符合檔名格式的只留最新 7 份；只刪符合格式的檔案，上次留下的 `.tmp` 下次先刪。還原手動、寫在 README：停 daemon → 複製快照蓋掉 `agend.db` → 刪 `agend.db-wal`。名詞表加「DB 快照」（已有 holder 畫面快照、shim 快照、binding 快照）。
- 理由：`VACUUM INTO` 做出一致、壓縮的單一檔，`sqlite3` 直接開；先寫暫存檔再 rename，崩潰不會留下半個快照。
- 替代方案：online backup API（程式多）；直接複製檔案（WAL 下可能不一致）；本地時區命名（夏令時間會重複或跳號）。
- 例子：`backups/` 有 `agend-2026-09-19.db` 到 `agend-2026-09-25.db` 共 7 份；你放的 `notes.txt` 還在。
- [x] 使用者確認（2026-09-25）

### 已知風險（開工時處理）

- P4 選快照：第 10 施工關要擴大 D32（`PipelineState` 加 serde derive）並重跑第 1 施工關。
- `EXCLUSIVE` 鎖：daemon 重啟時新 daemon 要等舊的完全結束；開 DB 最多重試 10 秒，第 6 施工關實測。
- macOS 沒開 `fullfsync`：斷電可能丟最後幾次 commit，靠第 10 施工關的 DB ↔ git 對帳。
- `bundled` 需要 C 編譯器，第一次編譯多約 20–30 秒（CI 的 ubuntu、macOS 都有）。
- `VACUUM INTO` 在 DB 執行緒上跑會擋其他請求；demo 印耗時，v1 量級要在 1 秒內。
- `$AGEND_HOME` 預設值尚未決定，v1-ts 用的是 `~/.agend`（交給第 9、13 施工關）。

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-daemon` 單獨通過，包括：STO-1..12 對真 store（P7）；golden schema、每個舊版 fixture 升級、壞 migration 退回、太新的 DB 被拒且檔案 hash 不變（P5）；每張表都有保留規則（P8）；快照 7 份輪替、不刪其他檔案（P9）；第二個程序開 DB 被拒（P2）
- [ ] `~/.cargo/bin/cargo test -p agend-daemon --test store_process` 通過：四次開機 pid 都不同、之間沒有子程序活著；重開前的舊版本 CAS 被擋、重開後版本嚴格變大；「每次開機用新的 DB 路徑」必須失敗；硬殺後 ack 過的寫入都在、integrity ok（P6、P7）
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過），並有新規則：`agend-tui` 不能依賴 SQLite 或 `agend-daemon`。細化：在 `crates/agend-tui/Cargo.toml` 加 `rusqlite = { version = "0.37", features = ["bundled"] }` 後 check-deps 印出 `agend-tui depends on libsqlite3-sys …`、`agend-tui depends on rusqlite …`、`xtask: 2 dependency rule violation(s)` 並失敗；還原後恢復 ok（單元測試 `tui_may_not_reach_sqlite_or_the_daemon` 也釘這條）
- [ ] `~/.cargo/bin/cargo xtask accept store` 通過，並印出下方用到的 demo（全部在暫存目錄、用假時鐘）
- [ ] `agend-daemon` 的 `README.md`／`TESTING.md` 已更新（TESTING 的「in-memory SQLite」改成暫存目錄裡的真 DB 檔；寫上還原步驟）；名詞表加 DB 快照、schema 版本、`backups/`
- [ ] fresh-context verifier 重跑並嘗試推翻，結果寫進「進度紀錄」（verifier 的 kill 只對自己起的子程序）

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 跑 demo。

   **這步在驗什麼**：demo 在暫存目錄建 DB，下面每一段都跑完。錯了代表 store 最基本的「建得起來、寫得進去」不成立。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo xtask accept store
   ```

   應該看到：依序出現 `== migrate`、`== restart`、`== crash`、`== retention`、`== snapshot`、`== too-new`、`== second-open` 七段，最後一行 `gate 5 (store): checks passed`。draft PR 還沒 merge 時，改在 PR 的 worktree（`~/Documents/Hack/AgEnD-v2-gate05`）跑。

   - [ ] 通過

2. 建 DB 與 migration。

   **這步在驗什麼**：全新 DB 升到最新 schema，檔案只有你能讀（P2、P5）。錯了的話其他使用者可能讀到 task 內容，或 schema 版本不對。

   操作：同一次輸出，找 `== migrate`。應該看到：

   ```text
   schema 0 -> 1
   rows: tasks 3, workflows 2, task_events 120
   home drwx------
   agend.db -rw-------
   ```

   - [ ] 通過

3. 四次開機，每次都是新的 process。

   **這步在驗什麼**：資料真的寫在磁碟上，不是藏在程序裡；開機 2 什麼都沒做，資料也沒被清掉（P7、STO-12）。錯了的話 daemon 每次重啟就遺失 task。

   操作：找 `== restart`。應該看到（pid 每次不同，但 4 行彼此都不同）：

   ```text
   boot 1 pid=<A> v=4 stale=3
   boot 2 pid=<B> (open only)
   boot 3 pid=<C> stale v=3 -> conflict current=4, task unchanged; v=5
   boot 4 pid=<D> v=5 ok (task, workflow, 4 events; next cas v=6)
   4 boots, 4 different pids, each child exited before the next started
   negative check (new DB path each boot): boot 3 failed: child pid=<E> exit status: 101 (task T-1 is missing)
   ```

   最後一行是反向檢查：每次開機換一個新的 DB 路徑，開機 3 就找不到 task——證明這套檢查分得出「真的存下來」和「藏在程序裡」。

   - [ ] 通過

4. 故意弄壞：重開前拿到版本的人，在重開後才寫入。

   **這步在驗什麼**：CAS 擋得住過期的寫入者，重啟之後也擋得住（STO-4）。錯了的話舊寫入會蓋掉新的 task 狀態。

   操作：同一段輸出找 `stale`。應該看到：`stale v=3 -> conflict current=4, task unchanged`。開機 1 最後一次寫入前拿到的版本是 3（`stale=3`），重開後拿它寫入被擋，回報目前版本 4；之後用 4 寫入才成功（`v=5`）。原本草稿寫的 `stale v=4 -> conflict current=4` 自相矛盾（拿目前版本寫入應該成功），見「待你追認」S6。

   - [ ] 通過

5. 故意弄壞：寫到一半硬殺。

   **這步在驗什麼**：commit 回來的寫入在程序被硬殺後都還在，DB 沒壞（P6）。錯了的話 daemon 當掉後資料遺失或 DB 打不開。

   操作：找 `== crash`。應該看到：

   ```text
   child pid=<N> acked 50 writes; killed own child pid=<N> (signal: 9 (SIGKILL))
   acked=<A> found=<F> integrity=ok
   ```

   `A` 是子程序死前印出的最後一個 ack（通常 50），`F` 是重開後 DB 裡的寫入數；必須 `F ≥ A`（通常相等，最多多 1：硬殺時正在 commit 的那一次）。

   - [ ] 通過

6. 保留期限。

   **這步在驗什麼**：過期事件被刪，task 與 workflow 永遠留著（P8、D31）。錯了的話 DB 無限長大，或 task 被刪。

   操作：找 `== retention`。應該看到：

   ```text
   fake clock +15d: tasks 3 -> 3, workflows 2 -> 2, task_events 120 -> 0
   calibration: v1 scale, tasks 8347, workflows 1, task_events 166940 (filled in <t> s)
   calibration: agend.db 57.2 MB (limit 1 GB), 7 snapshots 395.8 MB (limit 5 GB), slowest VACUUM INTO + quick_check <t> s
   calibration: within limits: D31 retention periods and 7 snapshots stay
   ```

   校準：v1 的 8,347 個 task，每個 20 個事件且全部還在 14 天內（上限估計）。實測（2026-09-26，macOS debug build）DB 57.2 MB、7 份快照 395.8 MB、單次快照 0.18–0.19 秒，遠低於門檻，D31 的期限與 7 份維持。

   - [ ] 通過

7. DB 快照與輪替。

   **這步在驗什麼**：每天一份可用的快照，只留 7 份，不刪別的檔案（P9）。錯了的話要還原時沒有能用的備份，或刪掉你的檔案。

   操作：找 `== snapshot`。應該看到：

   ```text
   <demo 目錄>/home/backups/agend-2026-10-06.db <N> bytes quick_check ok (-rw-------, <t> s)
   fake clock 9 days: backups: 7 files (agend-2026-10-08.db .. agend-2026-10-14.db), notes.txt kept
   backups is drwx------
   ```

   - [ ] 通過

8. 故意弄壞：舊 binary 打開新 schema 的 DB。

   **這步在驗什麼**：版本不合就拒絕開，DB 一個 byte 都沒動（P5）。錯了的話裝回舊版後會用錯的 schema 亂寫。

   操作：找 `== too-new`（demo 把一份快照副本的 `user_version` 改成比最新版多 1）。應該看到：

   ```text
   copy of a snapshot with user_version 2
   open: agend.db schema version 2 is newer than this agend supports (1); install a newer agend or restore a snapshot from <demo 目錄>/too-new/backups
   sha256 before <H>
   sha256 after  <H>
   unchanged
   ```

   兩行 `sha256` 的 `<H>` 要一模一樣。

   - [ ] 通過

9. 故意弄壞：兩個程序開同一個 DB。

   **這步在驗什麼**：同時只有一個 daemon 能用這個 DB（P2）。錯了的話兩個 daemon 同時驅動同一個 task。

   操作：找 `== second-open`。應該看到：

   ```text
   first store (pid=<P>) holds <demo 目錄>/home/agend.db
   second-open pid=<Q> error=agend.db is in use by another process (is another agend daemon running?)
   first store still writes: T-1 v=1 -> v=2
   ```

   - [ ] 通過

10. 你自己動手：不經過 agend，直接用 `sqlite3` 打開快照（開工時細化）。

    **這步在驗什麼**：快照是一般的 SQLite 檔，agend 壞掉時你也能自己打開、查資料、還原。錯了的話備份只有 agend 自己讀得懂。

    操作：

    ```bash
    cd ~/Documents/Hack/AgEnD-v2
    AGEND_STORE_DEMO_KEEP=1 ~/.cargo/bin/cargo run --quiet -p agend-daemon --example store_demo | tail -2
    ```

    最後兩行是 `kept <demo 目錄> (delete it when done)` 與 `export SNAP=<快照路徑>`。把 `export SNAP=…` 那一行整行貼上執行，再跑：

    ```bash
    sqlite3 -readonly "$SNAP" 'select count(*) from tasks;'
    ```

    應該看到：`3`。最後刪掉 `kept` 那一行印出的 demo 目錄（`rm -rf <demo 目錄>`，只刪那一個）。

    - [ ] 通過

## 待你追認

owner 睡著時由實作者決定、可以反悔的事（頁面沒寫到的設計選擇）。每項各一個 commit，標 `[待你追認]`。每項：決定 · 理由 · 反悔的成本 · 追認結果。

| # | 決定 | 理由 | 反悔成本 | 追認結果 |
|---|---|---|---|---|
| S1 | `save_workflow(&Workflow)` 存 `toml::to_string` 的結果（D19 格式，但不是使用者原始檔的文字，註解會掉）；同一個 (id, version) 再存一次回 `Exists`，不覆蓋 | D21：task 固定版本，版本內容不能變；存序列化結果保證讀得回（第 1 施工關 workflow golden 已鎖格式） | 第 9 施工關要保留原文：加 `save_workflow_toml(text)`，解析驗證後存原文；表不變 | 待追認 |
| S2 | 0700／0600 只在 store 建立 home、`agend.db`、`backups/` 時設定；已存在的目錄或檔案權限不改 | 頁面寫「建立時就是 0600」；改使用者自己建的目錄權限太侵入 | 改成每次開啟都 chmod：`open_connection` 加兩行＋一個測試 | 待追認 |
| S3 | 測試要求 `fixtures/schema-v1.sql` 到 `schema-v{LATEST}.sql` 全部存在，包括目前最新版（頁面只說「新增 migration 的 PR 要附前一版」） | 目前只有 v1，沒有「舊版」可測；把最新版也凍結，才能在今天就抓到「有人改了 0001_init.sql」 | 改回只要求舊版：迴圈上限改成 `LATEST_VERSION - 1` | 待追認 |
| S4 | 只有 `0 < 版本 < 最新` 才做升級前快照；全新空 DB（版本 0）直接建 schema | 空 DB 沒有資料可保護；否則每次全新安裝都在 `backups/` 留一份空快照 | 改成也做：條件改成 `found < supported` | 待追認 |
| S5 | 開 DB 時 `busy_timeout(0)`：被鎖就立刻回 `InUse`，不等待 | 第二個 daemon 要立刻知道；頁面風險表寫的「最多重試 10 秒」屬第 6 施工關的重啟交接，那時在呼叫端重試 | 第 6 施工關在 `SqliteStore::open` 外面包重試迴圈，store 不必改 | 待追認 |
| S6 | 四次開機裡的過期寫入者拿的是開機 1 最後一次寫入前的版本 3：輸出 `stale v=3 -> conflict current=4, task unchanged; v=5`；步驟 4 的「應該看到」照改 | 草稿的 `stale v=4 -> conflict current=4` 自相矛盾：拿目前版本 4 寫入依 CAS 應該成功 | 改成別的過期版本：`boot1` 的 `stale` 變數 | 待追認 |
| S7 | 崩潰段印 `acked=<子程序印出的最後一個 ack> found=<重開後的寫入數>`；測試要求 `found ≥ acked`、且最多多 1（硬殺時正在 commit 的那次） | 父程序讀到第 50 個 ack 才送 kill，子程序可能多寫幾次；頁面的 `acked=50 found=50` 只是常見情況 | 改成固定 50：子程序每次 ack 後等父程序回覆（多一條管線） | 待追認 |
| S8 | `RETENTION` 列出 `audit/shim.jsonl` 每日輪替留 14 天（gate 6 P8），測試確認這列存在；`prune` 不處理檔案規則，輪替本身是第 6 施工關的 TODO，不寫 stub | AGENTS：不寫假實作；寫 audit 的是 shim、每日觸發在第 6 施工關，放那裡才測得到 | 第 6 施工關實作時可以把檔案規則搬到它自己的模組，規則表測試跟著改 | 待追認 |
| S9 | 校準用 v1 的 8,347 個 task（V1-LESSONS），每個 20 個事件且全部在 14 天內（上限估計）；先經 store 寫一個 task 與 20 個事件（真 producer），再用 SQL 複製到 v1 量級 | v1 沒有事件數；經 API 逐筆寫 16 萬次 fsync 要幾分鐘，SQL 複製保持列的形狀與 production 一致。實測 DB 57.2 MB、7 份快照 395.8 MB、單次快照 0.19 秒（debug build） | 改事件數或改成逐筆寫：`store_demo.rs` 的常數 | 待追認 |
| S10 | `SqliteStore::open_with(home, now, migrations)` 與 `MIGRATIONS` 是公開 API；`open` 就是 `open_with(…, MIGRATIONS)` | 頁面要「清單是參數，不留測試後門」：同一條 production 路徑、只是清單當參數，整合測試才能跑壞 migration 與升級前快照 | 改成 `pub(crate)`：相關測試搬進 crate 內的單元測試 | 待追認 |
| S11 | `store` 模組與它的測試以 `cfg(unix)` 編譯（檔案權限用 unix mode） | CI 只有 ubuntu、macOS；testkit 的 socket 部分也是 unix-only | 要支援 Windows：權限改用 ACL 或略過，拿掉 `cfg(unix)` | 待追認 |

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-26 P1–P9 實作（draft PR，branch `feat/gate-05-store`）：`SqliteStore`、3 張 STRICT 表、migration 0001 + golden + v1 fixture、`prune`、每日 DB 快照、`store_demo`、`accept store`、check-deps 規則 `agend-tui`；實作者自跑自動驗收（fmt、clippy、workspace 測試含短 TMPDIR、`--test store_process`、check-deps `no-std build ok`、`accept store`、`accept testkit`）。fresh-context verifier 尚未跑。
- 2026-09-25 開工前提案 P1–P9 寫定，使用者逐題確認（P8 追加 audit 輪替）；狀態改為提案中。

## 下一步

```bash
cat docs/gates/gate-05-store.md
~/.cargo/bin/cargo xtask accept store
```
