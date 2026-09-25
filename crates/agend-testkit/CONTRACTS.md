# 契約規則表

> **TL;DR**
> - 7 個 trait 的契約規則，每條一個編號（`DRV-1`…`RUN-9`，共 52 條，5 條標「新增，待你追認」）；契約 suite 的每個 case 標著它驗的規則編號。
> - 記住：**每條規則至少一個 case、至少一個故意弄壞的實作（mutant）**；`tests/contract_teeth/` 的覆蓋測試讀這張表，缺一個就失敗。
> - 下一步：加規則 = 這裡加一列（含 mutant 名）+ 標了編號的 case + 註冊 mutant。

每列：規則（一行、可測）· 來源 · mutant（`tests/contract_teeth/<trait>.rs` 裡的名字）。來源寫「推得」表示文件沒逐字寫，但是文件規則的必然結果。標 **（新增，待你追認）** 的規則文件沒有寫，是實作者判斷需要而加的，見 [gate 2「待你追認」](../../docs/gates/gate-02-testkit.md#待你追認)。「不釘」列出刻意不驗的行為與理由。

## Driver（`DRV`，8 條）

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| DRV-1 | 送到閒置的 instance，回條是 `Sent` 或 `Confirmed`，不是 `Queued`／`Failed` | [delivery](../../docs/architecture/delivery.md#送達模型)；排隊只在忙碌時 | `QueuedReceipt` |
| DRV-2 | 送到不存在的 instance 是錯誤，不會改送別的 instance | 推得：`deliver(instance_id, …)` | `DeliversUnknownToDefault` |
| DRV-3 | 送達之後，事件裡最後會出現 `TurnCompleted` | [GLOSSARY driver](../../docs/GLOSSARY.md)：收狀態事件 | `HidesTurnCompleted` |
| DRV-4 | 每個事件的 cursor 都不同 | 推得：能從任何 cursor 接續 | `ConstantCursor` |
| DRV-5 | `events(after)` 不含該 cursor 本身，也不含更舊的事件 | `events(after_cursor)` 簽章；重連補回（[ARCHITECTURE](../../docs/ARCHITECTURE.md) 程序模型 2） | `IgnoresCursor`、`IncludesTheCursorEvent` |
| DRV-6 | `events(after)` 回傳該 cursor 之後的全部事件：一個不少、不截斷、順序不變 | 同上：重連補回斷線期間的事件 | `SkipsFirstAfterCursor`、`OnlyLastTwoAfterCursor` |
| DRV-7 | 同一個 cursor 重讀，結果只會變長；讀取不消耗事件 | 推得：能從任何 cursor 接續 | `ConsumesOnRead` |
| DRV-8 | 讀不存在的 instance 的事件是錯誤，不是空清單 | 推得：`events(instance_id, …)` | `UnknownInstanceHasNoEvents` |

不釘：送達是否一定被確認（有些路徑無法確認，[delivery](../../docs/architecture/delivery.md)）；忙碌時的行為（第 7 施工關對真 backend 定）；未知 cursor；不同 instance 的事件是否分開（fixture 只給一個 instance）。

## Forge（`FRG`，9 條）

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| FRG-1 | `head` 回報 branch 目前最新的 commit | [pipeline：head](../../docs/architecture/pipeline.md#merge-與-main-前進) | `CachesFirstHead` |
| FRG-2 | 不存在的 branch，`head` 是錯誤 | 推得：沒有 `refs/heads/<branch>` | `UnknownBranchHeadIsBase` |
| FRG-3 | `submit` 回報非空的 change id 與送出當下的 head | [GLOSSARY change id](../../docs/GLOSSARY.md) | `SubmitWithoutId` |
| FRG-4 | 不存在的 branch，`submit` 是錯誤 | 推得 | `SubmitsUnknownBranch` |
| FRG-5 | `expected_head` 等於 merge 當下的 head 就 merge：回 `Merged{merge_commit}`，base 移到那個 commit（submit 之後又 commit 也一樣） | [pipeline](../../docs/architecture/pipeline.md#merge-與-main-前進)：帶核准的 head merge | `MergedWithoutMerging`、`ComparesSubmittedHead` |
| FRG-6 | head 不符回 `HeadChanged{actual_head}`，`actual_head` 是當下真正的 head | 事件身分（[GLOSSARY](../../docs/GLOSSARY.md)） | `EchoesExpectedHead` |
| FRG-7 | head 不符時什麼都不變：base 不動、branch 不動；之後用目前的 head 仍可 merge | 同上；merge 只有 daemon 做、只做一次 | `MergesStaleHeads`、`MergesThenRefuses` |
| FRG-8 | `expected_head` 要整串相等：空字串、短 SHA、少一個字都算不符 | 推得：核准綁的是完整 head SHA | `PrefixMatchesHead` |
| FRG-9 | 不存在的 branch，merge 是錯誤 | 推得 | `UnknownBranchMergeIsHeadChanged` |

不釘（local 與 GitHub 不同）：同一 branch 重複 submit 的回傳；merge 後 branch 是否還在；已 merge 的 head 再要求 merge 一次會怎樣（verifier r2 的 F2 包裝與假實作行為相同，不是反例）。

## Store（`STO`，11 條）

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| STO-1 | 建立的 task 讀回來每個欄位都相同 | [ARCHITECTURE](../../docs/ARCHITECTURE.md)：store 是唯一真相來源 | `DropsDependsOn` |
| STO-2 | 不存在的 task 讀到 `None`，不是錯誤 | `load_task -> Option` 簽章 | `MissingTaskIsAnError` |
| STO-3 | 重複建立同一個 id 失敗，原本的 task 不變 | 推得：寫入只能經 CAS | `CreateOverwrites` |
| STO-4 | 用目前版本 CAS 會寫入，版本每次都嚴格變大（連續多次，擋 1→2→1） | [gate 1 P2](../../docs/gates/gate-01-core.md#p2trait-簽章)：寫入帶 CAS 版本 | `TogglingVersions` |
| STO-5 | 用較舊的版本 CAS 回 `Conflict`，task 不變 | 同上 | `LastWriterWins` |
| STO-6 | 用比目前新的版本 CAS 也回 `Conflict`，task 不變（CAS 是相等，不是「至少」） | 同上 | `AcceptsFutureVersions` |
| STO-7 | `Conflict` 回報的 `current_version` 是實際存著的版本 | `CasResult::Conflict{current_version}` | `GuessesCurrentVersion` |
| STO-8 | 對不存在的 task CAS 回 `Conflict{None}`，不會建立它 | 同上 | `CasCreatesMissingTask` |
| STO-9 | workflow 依 (id, 版本) 精確讀回；沒存過的版本是 `None`，不退回別的版本 | [GLOSSARY workflow](../../docs/GLOSSARY.md)：task 固定建立時的版本 | `FallsBackToOlderWorkflow` |
| STO-10 | 同一個 task 的事件依附加順序保存 | `append_event` | `PrependsEvents` |
| STO-11 | 事件依 task 分開，交錯附加也不混 | `append_event(task_id, …)` 簽章 | `SharedEventLog` |

不釘：第一個版本號；對不存在的 task 附加事件。

## Runtime（`RTM`，7 條）

「在跑」從 trait 外面看（`RuntimeFixture::is_running`：真實作看程序與 socket）。

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| RTM-1 | `start_holder` 回的 handle 是啟動的那個 instance，socket 路徑非空 | [D3](../../docs/decisions/d01-d08.md#d3) | `HandleNamesTheExecutable` |
| RTM-2 | `start_holder` 回來之後 holder 真的在跑 | 同上 | `StartsNothing` |
| RTM-3 | `recover_holders` 剛好列出正在跑的 holder | [ARCHITECTURE](../../docs/ARCHITECTURE.md) 程序模型 2：重啟後重連 holder | `RecoversNothing` |
| RTM-4 | recover 回的 handle 與 start 回的完全相同（pid、socket） | 同上：daemon 靠 handle 重連 | `RecoversWrongHandles` |
| RTM-5 | `stop_holder` 之後 holder 真的停了，不只是從清單拿掉；別的 holder 不受影響 | 推得：停止 | `StopOnlyHides` |
| RTM-6 | 停掉的 holder 不再被 recover | 同 RTM-3 | `RecoversStoppedHolders` |
| RTM-7 | 停掉的 instance 可以再啟動 | 推得：重啟 agent | `RefusesRestart` |

不釘：同一 instance 啟動兩次；停止不存在的 instance。

## Notifier（`NTF`，4 條）

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| NTF-1 | severity、title、task_id 原樣送達（三種 severity，有無 task_id 都試） | `Notification` 型別 | `DropsTaskIdOnError`、`AttentionBecomesError` |
| NTF-2 | body 完整送達：3,000 字、多位元組、多行也不截斷 | [delivery](../../docs/architecture/delivery.md#送達模型)、[V1-LESSONS #2](../../docs/V1-LESSONS.md) | `TruncatesAt64Bytes`、`TruncatesAt200Chars`、`TruncatesAt4096Bytes` |
| NTF-3 | title 與 body 的前後空白、空行都保留，不修剪 **（新增，待你追認）** | 由 NTF-2「完整內容」延伸 | `TrimsWhitespace` |
| NTF-4 | 依送出順序到達 **（新增，待你追認）** | 第 2 施工關原有，文件沒寫 | `HoldsInfoBack` |

不釘：送出失敗的重試（trait 只回錯誤）。

## Clock（`CLK`，4 條）

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| CLK-1 | 讀值是 unix 毫秒（2020–2100 之間），不是秒或微秒 | `now_unix_ms` | `Seconds` |
| CLK-2 | 連續讀取不倒退 **（新增，待你追認）** | 第 2 施工關原有；文件沒寫，系統時鐘被校時可能倒退 | `Jitters` |
| CLK-3 | 是 UTC：與 fixture 從外面讀的 UTC 時間相差 1 秒內 | 推得：unix 時間以 UTC 定義 | `LocalTimeOffset` |
| CLK-4 | 時間經過時讀值會變大（不是凍結的） | 推得：`now` | `Frozen` |

不釘：1 毫秒以下的解析度。

## Runner（`RUN`，9 條）

| ID | 規則 | 來源 | mutant |
|---|---|---|---|
| RUN-1 | exit code 照實回報（0 與非 0），沒逾時 `timed_out` 是 false | [D28](../../docs/DECISIONS.md)；`command` 關卡 exit 0 才通過 | `NonZeroExitBecomesOne` |
| RUN-2 | stdout 與 stderr 分開回報 | `CommandOutput` | `MergesStderrIntoStdout` |
| RUN-3 | 輸出逐位元組不變：不修剪空白、不轉碼（含非 UTF-8 位元組） | `stdout: Vec<u8>` | `TrimsStdout`、`DecodesOutputLossily` |
| RUN-4 | 大量輸出（stdout、stderr 各 256 KiB）完整回來，不因管線塞滿卡住或誤報逾時 **（新增，待你追認）** | 由 RUN-3 延伸；checks 的輸出常超過 64 KiB | `CapsOutputAt64KiB`、`RealDrainsAfterExit` |
| RUN-5 | 超過 timeout 回 `timed_out: true` 且沒有 exit code | [V1-LESSONS #10](../../docs/V1-LESSONS.md)：外部指令一律帶 timeout | `TimeoutExitCode124` |
| RUN-6 | 逾時在 timeout 後不久回報（200 ms 逾時要在 2 秒內），不等指令自己結束 | 同上 | `TimesOutLate` |
| RUN-7 | 逾時的指令被停掉（`sh` 沒機會寫標記檔） | 同上 | `LeavesCommandRunning` |
| RUN-8 | 逾時時指令啟動的子程序也一起停掉（整個 process group） **（新增，待你追認）** | owner 2026-09-25 要求；文件沒寫 | `RealKillsShOnly` |
| RUN-9 | 在指定的工作目錄執行 | `run(cmd, dir, timeout)`（D28） | `RealIgnoresWorkingDirectory` |

不釘：被 signal 殺掉的指令的 exit code；故意離開 process group 的程序（`setsid`）；stdin。

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-testkit --test contract_teeth
```
