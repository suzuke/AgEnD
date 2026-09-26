# 契約規則表

> **TL;DR**
> - 7 個 trait 的契約規則，每條一個編號（`DRV-1`…`RUN-9`，共 57 條，5 條標「新增，已追認」）；契約 suite 的每個 case 標著它驗的規則編號。
> - 記住：**每條規則至少一個 case、至少一個故意弄壞的實作（mutant）**；`tests/contract_teeth/` 的覆蓋測試讀這張表，缺一個就失敗。
> - 下一步：加規則 = 這裡加一列（含 mutant 名）+ 標了編號的 case + 註冊 mutant。

每列：規則（一行、可測）· 來源 · mutant（`tests/contract_teeth/<trait>.rs` 裡的名字）。來源寫「**推得**自 X」表示 X 沒逐字寫這條，但這條是 X 的必然結果（冒號後寫怎麼推）。標 **（新增，已追認）** 的規則文件沒有寫，是實作者判斷需要而加的，使用者 2026-09-25 追認，見 [gate 2「待你追認」](../../docs/gates/gate-02-testkit.md#待你追認)。「不釘」列出刻意不驗的行為與理由。

「daemon 重啟」在契約裡的意思：daemon 那一側的物件全部丟掉（drop），只剩持久狀態，再從持久狀態建一個新的。三個 fixture（Runtime、Driver、Store）各有一個 `Persisted` 型別與兩個方法：`persisted()` 取出持久狀態，`boot(&persisted)` 從持久狀態開機一個新的 fixture。`Persisted` 本身不是 trait 物件，也不能讓任何 trait 物件活著；Runtime 的 `is_running` 與 Driver 的 `emit_while_down` 只拿 `Persisted`，所以 daemon 不在時 backend 的動作（holder 繼續跑、agent 自己跑完一輪）不經過任何 daemon 那一側的物件。

重啟類規則（DRV-6、DRV-9、STO-4、STO-12、RTM-8、RTM-9）都用同一個 daemon 生命週期（`contract::daemon_lifecycle`）驗。case 拿到的 fixture 先轉成持久狀態、drop 掉，之後一共 4 次開機，每次開機都先做開機復原（Runtime `recover_holders`；Driver 從上一個 daemon 最後看到的 cursor 補回事件；Store 就是打開），兩次開機之間沒有任何實例活著：

| 開機 | 種類 | 做什麼 |
|---|---|---|
| 1 | 做事 | 開機復原，再做事（寫入、啟動 holder、送訊息） |
| 2 | 閒置 | 只做開機復原，不做事就結束 |
| 3 | 做事 | 開機復原、檢查前面留下的，再做事 |
| 4 | 檢查 | 開機復原，檢查前面留下的全部（只做檢查需要的寫入，例如 CAS 看下一個版本） |

每次開機之後、下一次開機之前，daemon 不在時 backend 照常動（holder 繼續跑、從 `Persisted` 確認還在跑；agent 自己跑完一輪）。

每一段擋一類實作（verifier r4、r5）：

- 只重啟一次不夠：只在 Rust 物件裡的狀態、第一次讀就被消耗的狀態，撐得過一次，撐不過第二次（`CounterStore`、`ObjDedup`、`Handoff`）。
- 閒置開機：打開時把持久狀態讀走清空、只在有變動時才寫回的實作，每次開機都做事時看不出來（`RewriteOnChange`、`TruncOnOpen`）。
- 開機 4 的檢查：重啟後的 daemon 寫的東西沒存下來（`FrozenRegistry`、`FrozenDatabase`）。
- 先 drop case 自己的 fixture：只要還有一個實例活著就不丟資料的實作（SQLite shared-cache 的記憶體 DB、只保留有 driver 活著時的事件、最後一個 runtime 走時 holder 跟著死）在 fixture 一直活著時看不出來（`SharedMem`、`LiveJournal`、`LastOneOut`）。
- DRV-9 在重啟前送兩個以上不同的 id，重啟後先重送舊的（`LastIdDedup`）；DRV-6 在重啟後也從較舊的 cursor 補回（daemon 在存下最新 cursor 前崩潰），要拿到那之後的全部事件（`ReadAck`）。

**fixture 要求**：真實作的 `Persisted`／`boot`／`emit_while_down`／`is_running` 要走真的持久層：同一個 DB 檔、同一個 run 目錄與 holder socket、真的 fake agent 程序，不可以用 process 內的全域 `static`、shared-cache 記憶體 DB，或 `Persisted` 裡藏一個活著的實例把狀態帶過去。契約在同一個 process 裡跑，分不出「真的存下來」與「藏在 process 裡」，所以：

**重啟／持久化類的契約 case，要對真實作、跨真的 process 重啟跑（分開的 process、真的檔案／DB）**：Store 在[第 5 施工關](../../docs/gates/gate-05-store.md)、Runtime（holder）在[第 6 施工關](../../docs/gates/gate-06-daemon-holder.md)。本表的 mutant 都是同一個 process 裡的模型；例如 `LastOneOut`（holder 是 daemon 程序的子程序、daemon 結束就跟著死）在這裡用共享的 `Weak` 模擬，真的 process 版本在第 6 施工關驗。

## Driver（`DRV`，9 條）

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| DRV-1 | 送到閒置的 instance，回條是 `Sent` 或 `Confirmed`，不是 `Queued`／`Failed` | **推得**自 [delivery 忙碌策略](../../docs/architecture/delivery.md#忙碌策略三級)：排隊是忙碌時的等級，閒置時直接送 | `QueuedReceipt` |
| DRV-2 | 送到不存在的 instance 是錯誤，不會改送別的 instance | 推得：`deliver(instance_id, …)` | `DeliversUnknownToDefault` |
| DRV-3 | 送達之後，事件裡最後會出現 `TurnCompleted` | **推得**自 [GLOSSARY driver](../../docs/GLOSSARY.md)：driver 收狀態事件，一輪結束是其中之一 | `HidesTurnCompleted` |
| DRV-4 | 每個事件的 cursor 都不同 | 推得：能從任何 cursor 接續 | `ConstantCursor` |
| DRV-5 | `events(after)` 不含該 cursor 本身，也不含更舊的事件 | `events(after_cursor)` 簽章；重連補回（[ARCHITECTURE](../../docs/ARCHITECTURE.md) 程序模型 2） | `IgnoresCursor`、`IncludesTheCursorEvent` |
| DRV-6 | `events(after)` 回傳該 cursor 之後的全部事件：一個不少、不截斷、順序不變；每次 daemon 重啟後，新的 driver 拿上一個 driver 最後看到的 cursor 或更舊的 cursor 也一樣，而且包含 daemon 不在時 agent 產生的事件 | [ARCHITECTURE](../../docs/ARCHITECTURE.md#程序模型) 程序模型規則 2：daemon 重啟後「重連 backend 補回斷線期間的事件」 | `SkipsFirstAfterCursor`、`OnlyLastTwoAfterCursor`、`OnlyOwnLifetimeEvents`、`Gap`、`ReadAck`、`LiveJournal` |
| DRV-7 | 同一個 cursor 重讀，結果只會變長；讀取不消耗事件 | 推得：能從任何 cursor 接續 | `ConsumesOnRead` |
| DRV-8 | 讀不存在的 instance 的事件是錯誤，不是空清單 | 推得：`events(instance_id, …)` | `UnknownInstanceHasNoEvents` |
| DRV-9 | 同一個訊息 id 再送一次不多一個 turn（`turn_timeout` 內不出現新的 `TurnCompleted`），daemon 重啟後再送也一樣（不只最後一個 id：之前送過的每個 id 都一樣）；再送時 `deliver` 不是錯誤 | [delivery](../../docs/architecture/delivery.md#送達模型)：以 id 冪等，只有一套去重；[GLOSSARY 訊息](../../docs/GLOSSARY.md)；[V1-LESSONS #1](../../docs/V1-LESSONS.md)；[REWRITE-PLAN](../../docs/research/REWRITE-PLAN.md) 假 driver「重連後不遺失不重複」。「再送不是錯誤」是**推得**自 delivery 的「以 id 冪等」：冪等的操作重做一次結果相同，回錯誤會讓崩潰後重送的一方以為沒送到 | `RedeliversSameId`、`ObjDedup`、`LastIdDedup` |

不釘：送達是否一定被確認（有些路徑無法確認，[delivery](../../docs/architecture/delivery.md)）；忙碌時的行為（第 7 施工關對真 backend 定）；未知 cursor；不同 instance 的事件是否分開（fixture 只給一個 instance）。

## Forge（`FRG`，10 條）

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| FRG-1 | `head` 回報 branch 目前最新的 commit | [pipeline：head](../../docs/architecture/pipeline.md#merge-與-main-前進) | `CachesFirstHead` |
| FRG-2 | 不存在的 branch，`head` 是錯誤 | 推得：沒有 `refs/heads/<branch>` | `UnknownBranchHeadIsBase` |
| FRG-3 | `submit` 回報送出當下的 head；沒有 change id 時 `id` 是 `None`（forge local），有的話不是空字串；兩個不同 branch 的 change 都有 id 時，id 不同 | [GLOSSARY change id](../../docs/GLOSSARY.md)、[pipeline](../../docs/architecture/pipeline.md#6-種關卡)：local 沒有 change id；`SubmittedChange.id: Option<String>` 與 `PipelineEvent::Submitted { change_id: Option<_> }` 一致（gate 2 A22）；`agend-core` `pipeline/state.rs` 的可完成證明也這樣假設 | `SameIdForEveryChange`、`EmptyIdInsteadOfNone` |
| FRG-4 | 不存在的 branch，`submit` 是錯誤 | **推得**自 [pipeline](../../docs/architecture/pipeline.md#merge-與-main-前進)：head 是 `refs/heads/<branch>` 指向的 commit，submit 送出的是 branch 的 head；branch 不存在就沒有東西可送（同 FRG-2） | `SubmitsUnknownBranch` |
| FRG-5 | `expected_head` 等於 merge 當下的 head 就 merge：回 `Merged{merge_commit}`，base 移到那個 commit（submit 之後又 commit 也一樣） | [pipeline](../../docs/architecture/pipeline.md#merge-與-main-前進)：帶核准的 head merge | `MergedWithoutMerging`、`ComparesSubmittedHead` |
| FRG-6 | head 不符回 `HeadChanged{actual_head}`，`actual_head` 是當下真正的 head | **推得**自事件身分（[GLOSSARY](../../docs/GLOSSARY.md)）：daemon 靠回報的 head 判斷結果是不是目前的 | `EchoesExpectedHead` |
| FRG-7 | head 不符時什麼都不變：base 不動、branch 不動；之後用目前的 head 仍可 merge | **推得**自同上與 [pipeline](../../docs/architecture/pipeline.md#merge-與-main-前進)：merge 只有 daemon 做、只做一次 | `MergesStaleHeads`、`MergesThenRefuses` |
| FRG-8 | `expected_head` 要整串相等：空字串、短 SHA、少一個字都算不符 | 推得：核准綁的是完整 head SHA | `PrefixMatchesHead` |
| FRG-9 | 不存在的 branch，merge 是錯誤 | **推得**自 [pipeline](../../docs/architecture/pipeline.md#merge-與-main-前進)：merge 比對的是 `refs/heads/<branch>` 目前的 head；branch 不存在就沒有 head 可比對，回 `HeadChanged` 會讓 daemon 以為只是 head 變了而重跑關卡（事件身分，[pipeline](../../docs/architecture/pipeline.md#6-種關卡)） | `UnknownBranchMergeIsHeadChanged` |
| FRG-10 | merge 落在目前的 base 上、不弄丟之前的 merge：從同一個 base 開出的兩個 branch 依序 merge，之後 base 仍含第一個 merge commit 與兩個被 merge 的 head（用 `ForgeFixture::base_contains` 從 trait 外面看 ancestry） | **推得**自 [pipeline](../../docs/architecture/pipeline.md#merge-與-main-前進)：local 以 `merge-tree` 產生結果、CAS `update-ref` 比對 main 仍是預期 SHA，所以新的 main 建在 merge 當下的 main 上；把 main 直接設成 branch 的 head 會丟掉 branch 開出之後才 merge 的變更（gate 2 A23） | `OverwritesBase` |

不釘（local 與 GitHub 不同）：同一 branch 重複 submit 的回傳；merge 後 branch 是否還在；已 merge 的 head 再要求 merge 一次會怎樣（verifier r2 的 F2 包裝與假實作行為相同，不是反例）。change id 一律回 `None` 的 forge 通過整個 suite（`contract_teeth` 的 `forge_without_change_ids_passes`，原本的 `SubmitWithoutId`）。

## Store（`STO`，12 條）

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| STO-1 | 建立的 task 讀回來每個欄位都相同 | [ARCHITECTURE](../../docs/ARCHITECTURE.md)：store 是唯一真相來源 | `DropsDependsOn` |
| STO-2 | 不存在的 task 讀到 `None`，不是錯誤 | `load_task -> Option` 簽章 | `MissingTaskIsAnError` |
| STO-3 | 重複建立同一個 id 失敗，原本的 task 不變 | 推得：寫入只能經 CAS | `CreateOverwrites` |
| STO-4 | 用目前版本 CAS 會寫入，版本每次都嚴格變大（連續多次，擋 1→2→1）；跨重新開啟也一樣：新版本大於之前發出的每一個版本，拿著重開前版本的寫入者在重開後被擋 | **推得**自 [gate 1 P2](../../docs/gates/gate-01-core.md#p2trait-簽章)（寫入帶 CAS 版本）：版本回到舊值（ABA）時，拿著舊版本的寫入者會成功；重啟前拿到版本的寫入者（例如還沒收到結果的 agent）在重啟後仍然存在 | `TogglingVersions`、`CounterStore` |
| STO-5 | 用較舊的版本 CAS 回 `Conflict`，task 不變 | 同上 | `LastWriterWins` |
| STO-6 | 用比目前新的版本 CAS 也回 `Conflict`，task 不變（CAS 是相等，不是「至少」） | **推得**自同上：compare-and-swap 比的是相等 | `AcceptsFutureVersions` |
| STO-7 | `Conflict` 回報的 `current_version` 是實際存著的版本 | `CasResult::Conflict{current_version}` | `GuessesCurrentVersion` |
| STO-8 | 對不存在的 task CAS 回 `Conflict{None}`，不會建立它 | 同上 | `CasCreatesMissingTask` |
| STO-9 | workflow 依 (id, 版本) 精確讀回；沒存過的版本是 `None`，不退回別的版本 | [GLOSSARY workflow](../../docs/GLOSSARY.md)：task 固定建立時的版本 | `FallsBackToOlderWorkflow` |
| STO-10 | 同一個 task 的事件依附加順序保存 | `append_event` | `PrependsEvents` |
| STO-11 | 事件依 task 分開，交錯附加也不混 | `append_event(task_id, …)` 簽章 | `SharedEventLog` |
| STO-12 | 每次重新開啟（daemon 重啟）後 task、版本、workflow、事件都還在；保存的版本 CAS 可寫入 | [GLOSSARY store](../../docs/GLOSSARY.md)：SQLite、唯一真相來源（D8）；[pipeline](../../docs/architecture/pipeline.md#worktree-與-branch-生命週期)：先寫 DB，崩潰後開機接續 | `InMemoryOnly`、`TruncOnOpen`、`SharedMem`、`FrozenDatabase` |

不釘：第一個版本號；對不存在的 task 附加事件。

## Runtime（`RTM`，9 條）

「在跑」從 trait 外面看（`RuntimeFixture::is_running`：只拿 `Persisted`，daemon 不在時也能問；真實作只看鎖檔的 `flock` 是否被持有，不連 socket——連了會搶走 runtime 自己的長連線，見[第 4 施工關 P3](../../docs/gates/gate-04-holder.md#p3run-目錄不重複判斷存活)、[第 6 施工關 H16](../../docs/gates/gate-06-daemon-holder.md#待你追認)）。

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| RTM-1 | `start_holder` 回的 handle 是啟動的那個 instance，socket 路徑非空 | [D3](../../docs/decisions/d01-d08.md#d3) | `HandleNamesTheExecutable` |
| RTM-2 | `start_holder` 回來之後 holder 真的在跑 | 同上 | `StartsNothing` |
| RTM-3 | `recover_holders` 剛好列出正在跑的 holder | [ARCHITECTURE](../../docs/ARCHITECTURE.md) 程序模型 2：重啟後重連 holder | `RecoversNothing` |
| RTM-4 | recover 回的 handle 與 start 回的完全相同（pid、socket） | **推得**自同上：daemon 靠 handle 重連，handle 錯了就連不上 | `RecoversWrongHandles` |
| RTM-5 | `stop_holder` 之後 holder 真的停了，不只是從清單拿掉；別的 holder 不受影響 | 推得：停止 | `StopOnlyHides` |
| RTM-6 | 停掉的 holder 不再被 recover | 同 RTM-3 | `RecoversStoppedHolders` |
| RTM-7 | 停掉的 instance 可以再啟動 | 推得：重啟 agent | `RefusesRestart` |
| RTM-8 | 每次 daemon 重啟後（不只啟動它的 daemon 之後那一次），之前啟動、還沒停的 holder 都還在跑；新的 runtime 的 `recover_holders` 列出它們，handle 與當初 `start_holder` 回的相同 | [D3](../../docs/decisions/d01-d08.md#d3)：daemon 重啟時 agent 不斷線，重啟後重連 holder；[ARCHITECTURE](../../docs/ARCHITECTURE.md) 程序模型 2；[V1-LESSONS #7](../../docs/V1-LESSONS.md) | `DaemonScoped`、`RecoversOnlyOwnHolders`、`Handoff`、`RewriteOnChange`、`LastOneOut`、`FrozenRegistry` |
| RTM-9 | 重啟後的新 runtime 停得掉之前的 runtime 啟動的 holder（真的停，別的不受影響），每次重啟都一樣 | **推得**自 D3：重啟後由新 daemon 管理重連的 holder | `StopsOnlyOwnHolders` |

不釘：同一 instance 啟動兩次；停止不存在的 instance。

## Notifier（`NTF`，4 條）

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| NTF-1 | severity、title、task_id 原樣送達（三種 severity，有無 task_id 都試） | `Notification` 型別 | `DropsTaskIdOnError`、`AttentionBecomesError` |
| NTF-2 | body 完整送達：3,000 字、多位元組、多行也不截斷 | [delivery](../../docs/architecture/delivery.md#送達模型)、[V1-LESSONS #2](../../docs/V1-LESSONS.md) | `TruncatesAt64Bytes`、`TruncatesAt200Chars`、`TruncatesAt4096Bytes` |
| NTF-3 | title 與 body 的前後空白、空行都保留，不修剪 **（新增，已追認）** | 由 NTF-2「完整內容」延伸 | `TrimsWhitespace` |
| NTF-4 | 依送出順序到達 **（新增，已追認）** | 第 2 施工關原有，文件沒寫 | `HoldsInfoBack` |

不釘：送出失敗的重試（trait 只回錯誤）。

## Clock（`CLK`，4 條）

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| CLK-1 | 讀值是 unix 毫秒（2020–2100 之間），不是秒或微秒 | `now_unix_ms` | `Seconds` |
| CLK-2 | 連續讀取不倒退 **（新增，已追認）** | 第 2 施工關原有；文件沒寫，系統時鐘被校時可能倒退 | `Jitters` |
| CLK-3 | 是 UTC：與 fixture 從外面讀的 UTC 時間相差 1 秒內 | 推得：unix 時間以 UTC 定義 | `LocalTimeOffset` |
| CLK-4 | 時間經過時讀值會變大（不是凍結的） | 推得：`now` | `Frozen` |

不釘：1 毫秒以下的解析度。

## Runner（`RUN`，9 條）

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| RUN-1 | exit code 照實回報（0 與非 0），沒逾時 `timed_out` 是 false | [D28](../../docs/DECISIONS.md)；`command` 關卡 exit 0 才通過 | `NonZeroExitBecomesOne` |
| RUN-2 | stdout 與 stderr 分開回報 | `CommandOutput` | `MergesStderrIntoStdout` |
| RUN-3 | 輸出逐位元組不變：不修剪空白、不轉碼（含非 UTF-8 位元組） | `stdout: Vec<u8>` | `TrimsStdout`、`DecodesOutputLossily` |
| RUN-4 | 大量輸出（stdout、stderr 各 256 KiB）完整回來，不因管線塞滿卡住或誤報逾時 **（新增，已追認）** | 由 RUN-3 延伸；checks 的輸出常超過 64 KiB | `CapsOutputAt64KiB`、`RealDrainsAfterExit` |
| RUN-5 | 超過 timeout 回 `timed_out: true` 且沒有 exit code | **推得**自 [V1-LESSONS #10](../../docs/V1-LESSONS.md)（外部指令一律帶 timeout）：逾時的指令沒有自己結束，沒有 exit code 可回報；回 124 之類的值會被當成指令失敗 | `TimeoutExitCode124` |
| RUN-6 | 逾時在 timeout 後不久回報（200 ms 逾時要在 2 秒內），不等指令自己結束 | **推得**自同上：等指令自己結束就等於沒有 timeout | `TimesOutLate` |
| RUN-7 | 逾時的指令被停掉（`sh` 沒機會寫標記檔） | **推得**自同上：沒停掉的指令繼續占資源、改檔案 | `LeavesCommandRunning` |
| RUN-8 | 逾時時指令啟動的子程序也一起停掉（整個 process group） **（新增，已追認）** | owner 2026-09-25 要求；文件沒寫 | `RealKillsShOnly` |
| RUN-9 | 在指定的工作目錄執行 | `run(cmd, dir, timeout)`（D28） | `RealIgnoresWorkingDirectory` |

不釘：被 signal 殺掉的指令的 exit code；故意離開 process group 的程序（`setsid`）；stdin。

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-testkit --test contract_teeth
```
