# agend-testkit 測試

> **TL;DR**
> - 測假實作本身、7 個契約 suite（對假實作全過；[CONTRACTS.md](CONTRACTS.md) 的 57 條規則各有 mutant，每個 mutant 都被它那條規則的 case 抓到）、假 daemon、3 個假 agent 程式（以真的子程序跑）。
> - 記住：假 agent 的測試啟動 `src/bin/` 的真 binary，走真的 socket／HTTP；不在行程內呼叫。
> - 下一步：`~/.cargo/bin/cargo test -p agend-testkit`。

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
| `tests/fake_daemon.rs` | hello 必須在前（無效 JSON 也回 `hello_required` 並關閉）、hello 之後的無效 JSON 不斷線、drop 時已開的連線讀到 EOF（2 秒內）、major 不合的錯誤訊息、狀態與未知請求、事件身分（沒帶、舊 attempt、別的關卡、重播都 `stale_result`）、backlog 再即時事件、請示回答 |
| `tests/fake_codex.rs` | turn 完成事件帶回 threadId／turnId；steer（錯的 turn id 被拒）、queue 自動出列成新 turn、interrupt；approval 等待決定；只有 resume 過的 thread 才推事件；長路徑 symlink 指到 temp dir 裡的短 socket（不留目錄） |
| `tests/fake_opencode.rs` | SSE 事件順序、同步 prompt 回覆、忙碌排隊、abort 標 `MessageAbortedError`、REST 補歷史、status |
| `tests/fake_claude.rs` | Stop hook block 多一輪（`stop_hook_active` false → true）、Esc 中斷不觸發 Stop、hook payload、channel 包裝、未知 channel server 的錯誤、transcript 在專案目錄內 |

## 花時間的地方

- Runner 契約的兩個「停掉了嗎」case 用真的時間：`sleep 3; touch timed-out-command-finished` 與 `sh -c 'sleep 3; touch timed-out-child-finished'; true` 以 200 ms 逾時跑，各等到約 4.5 秒確認標記檔沒出現（只讀檔，不送 signal 探測）。假實作也一樣等，所以 Runner suite 每跑一次約 9 秒。
- `contract_teeth` 約 15 秒：mutant 平行跑，最慢的是 Runner mutant（約 9–15 秒）與真 `sh` runner 的對照測試。

## 輸入從哪來（#1493）

- 假 daemon 與測試都用 `agend_core::protocol::client` 型別 + `serde_json` 編碼；沒有手寫 client protocol JSON。
- 契約 suite 用 core 的建構子（`Task::new`、`Workflow::builtin_code`、`model::work_branch`）產生輸入。
- backend 協定（codex、opencode、claude）是外部格式，沒有 Rust producer：測試寫的 JSON 取自 `docs/research/spike-*.md` 記錄的形狀。

## 用到的假實作

- 不適用（這裡就是假實作的家）

## 還沒測的

- [ ] 契約 suite 對真實作（各施工關接上：Store 第 5、agent runtime 第 6、Driver 第 7／12、Runner 與 Forge local 第 10、Notifier 與 Forge github 第 12）
- [ ] 假 agent 的欄位對真 backend schema 逐一比對（第 7、12 施工關）
- [ ] binding 快照 fixture（第 3 施工關需要時）

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-testkit
```
