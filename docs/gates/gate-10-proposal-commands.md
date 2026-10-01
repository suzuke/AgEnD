# Gate 10 提案：重啟、命令與驗收替身

> **TL;DR**
> - P9–P11 定義四次開機、命令與真／假邊界。
> - 狀態與驗證證據見 [Gate 10 入口](gate-10-pipeline.md)。
> - 下一步：閱讀 [已決定事項與邊界](gate-10-proposal-boundaries.md)。

### P9：開機對帳與四次開機的重啟契約

- 問題：daemon 被 `kill -9`、或斷電丟了最後幾筆寫入（第 5 施工關：macOS 沒開 `fullfsync`），重開後怎麼接著做？怎麼證明真的接得上？
- 建議：
  - 什麼時候：開機時（第 6 施工關的開機計畫做完後、第 8 施工關 bind socket **之前**）與每天一次（掛在第 6 施工關的 housekeeping，今天做過就跳過，照 pipeline.md 的「開機與每日」）。每天那次只做下面第 2、3 項。以 DB 為準：
    1. 每個未結束的 task：`restore` 快照（P2）；失敗 → `Failed`＋「需要你」。
    2. binding：`pending` 的接著建（P4）；`ready` 但 worktree 不見了 → 從 branch 重建；branch 也不見 → task `Failed`。重寫**全部** binding 快照檔（檔案只是 DB 的投影），刪掉 DB 沒有的 instance 的快照檔。
    3. 命名空間裡的孤兒（`agend/<task>/…` branch、`worktrees/<task>*`、`checks/<task>-*`，DB 沒有這個 task、它已結束、或是上一次開機留下的 checks 目錄）→ 照 P4 的釋放流程（先存 WIP patch 再刪）。命名空間外一律不碰。
    4. 重做 `outstanding_actions`（P2）：派工訊息用固定的訊息 id `dispatch:<ticket>` 重送（第 7 施工關的送達以 id 冪等）；checks 在新目錄重跑同一個 attempt（P6）；merge 送出中先照 P7 判斷是不是已經 merge。
    5. 逾時：daemon 跑著時，每進入一個關卡（`ScheduleTimeout`）就排一個到 deadline 的計時器，時間到餵 `StageTimedOut`；開機時照下面重排。`command` 與 `merge` 以外的關卡（merge 送出中 core 拒絕 `StageTimedOut`，每次開機都會多一行錯誤 log），deadline＝`stage_entered_at_unix_ms`＋關卡的 timeout，已經過了就立刻餵 `StageTimedOut`（上次已經報過的「通知」逾時會回 `StaleResult`，這是正常的、只記 debug，不算錯誤）；`command` 的逾時由 runner 管（P6），重跑時重新計時。
  - 斷電丟了最後幾筆：DB 回到較舊的狀態，對帳就從那個狀態接著做。各種外部動作的處理：
    - merge：`merge_intent` 可能也丟了，所以 P7 先找 trailer，不靠 `merge_intent`。
    - worktree、hook：已存在就檢查、沿用。
    - 派工訊息：`messages` 那一列若也丟了，同一個 id 會再送一次，agent 可能看到兩次；它帶的 ticket 相同，第二次的結果回 `stale_result`，不會做兩次。這是接受的代價，不是「完全安全」。
  - 重啟契約（比照 CONTRACTS 的四次開機與第 6 施工關 P4，跨真的 process、真的 `agend daemon`）：
    - 開機 1（做事）：以操作者身分 `agend task create --team … --role dev` 建 task，假 agent 做完、`done`；checks 跑到一半時測試 `kill -9` daemon（測試自己起的 pid）。
    - 開機 2（閒置）：測試不做事；對帳在新目錄重跑 checks，task 走到 agent 審查、假 reviewer 核准、停在人工核准。
    - 開機 3（做事）：測試按 `approve`；在 failpoint `after-main-moved`（main 已經移動、`merge_commit` 還沒存）中止。
    - 開機 4（檢查）：task `done`；main 上這個 task 的 merge commit **剛好一個**；branch、worktree、checks 目錄都不見；每個 ticket 的派工訊息只送過一次。
    - 反向檢查：每次開機用新的 `AGEND_HOME` → 開機 2 就失敗。
  - failpoint：`AGEND_FAILPOINT=<名稱>` 讓 daemon 在那一點 `abort()`，**只在 debug build 編進去**。只有兩個：`after-merge-intent`、`after-main-moved`（merge 前後最窄的兩個窗口；其他地方用測試的 `kill -9` 就碰得到）。
- 理由：單一對帳取代 v1 約 9 個清理機制（V1-LESSONS #5）；靠 kill 的時機碰運氣測不到「剛好在 merge 中間」，failpoint 讓那一刀落在指定的地方。
- 替代方案：開機時不對帳、等事件自然發生（死在 checks 中的 task 永遠卡住）；每小時對帳（pipeline.md 寫的是每日，沒有需要更頻繁的證據）；失敗點用隨機 kill 多跑幾次（不穩、測不到窄窗口）；failpoint 也編進 release（正式 binary 多一條可以讓它自己 abort 的路）。
- 例子：`boot 3 daemon pid=<C> t-1 merge: main moved to 9f8e…; aborted at failpoint after-main-moved`、`boot 4 t-1: merge found on main by trailer (9f8e…); not merged again`。
- [x] 使用者確認（2026-09-26）

### P10：本關擁有的命令與請示

- 問題：分工表把幾個 agent 命令的 daemon 端、請示、`agend team`／`agend workflow` 交給本關。各自做到哪？
- 建議：
  - agent 命令的 daemon 端（CLI 語法、錯誤碼對應、斷線重送規則照第 9 施工關）：

    | 命令 | daemon 做什麼 |
    |---|---|
    | `done <ticket>` | ticket 拆成 task 與 identity；讀 branch head（P5）、拒絕空 branch（P7）→ `WorkCompleted{Branch}` |
    | `result <ticket> "<摘要>"` | `WorkCompleted{Result}`（`research` 這類沒有 repo 的 workflow） |
    | `review approve <ticket>`／`changes <ticket> "<理由>"` | 只收這次審查的 reviewer；head 取審查 binding 的 head（agent 不必給）→ `ApprovalGranted`／`ChangesRequested` |
    | `block`／`unblock` | `TaskOperation::Block`／`Unblock`；理由顯示在 `agend status` 與 TUI，不進「需要你」（要你處理就用 `ask`） |
    | `remind` | `reminders` 表記一筆（task、到期時間），到期時送訊息 `remind:<task>/<序號>` 給 task 持有者；送出後刪那一列；重開機後照表補 |
    | `task create` | 照 D18（語法照第 9 施工關：`task create --role <role> "<title>" [--team] [--workflow]`）；agent 與操作者都能跑（使用者 2026-09-26 決定），操作者一定要帶 `--team`，agent 不帶時 team 預設是自己的；workflow 預設是 team 的 `default_workflow`；`--role` 必須等於那個 workflow 第一個 work 關卡的角色，不同就拒絕並寫出應該是哪個（本關不做「改寫 workflow 的角色」）；存檔檢查、要 repo 的 workflow 在沒 repo 的 team 被拒；用到 fanout、`reassign`、或 `command` 關卡寫了 `on_timeout`（P6）被拒。審查者照 `policy::assign` 的 `Review`：排除 task 持有者（作者），優先不同 backend。只有兩種情況出現 `no-role:<team>/<role>`：team 沒有這個角色（`AskForRole`），或這個角色只有作者（`NoEligibleReviewer`）；角色有別人但都在忙 → 只排隊（`Queue{AtCapacity}`），不進「需要你」 |

  - 請示（D35）：`asks` 表（id、instance、task、狀態、建立時間）與 `ask_turns` 表（提問、選項、回答、追問、結論，依序），保留「永久」（D31）。`ask` 建一筆、出現在「需要你」；`answer_ask` 把回答記下並送給 agent（訊息 id `ask:<ask id>/<輪次>`）；`AskFollowUp` 再出現一次；`AskResolve` 結束。
  - 操作者命令（本關的 CLI，走第 9 施工關的 `operator` 請求）：
    - `agend team add <team> [--repo <path>] [--workflow <id>]`、`agend team list`、`agend team set-workflow <team> <id>`、`agend team join <team> <name> --role <role>`（`<name>` 是第 9 施工關 `agend instance add <name> <backend>` 的 name）
    - `agend workflow list`、`show <id>`、`check <file>`、`apply <file>`；`check`／`apply` 在 core 的存檔檢查之外，也拒絕本關還不支援的：fanout、`reassign`、`command` 關卡的 `on_timeout`（D19 的 `new --from`、`edit`、`history`、`rollback`、`delete` 之後再做）
    - 新請求 `task_cancel { task_id, reason }`（只收操作者）→ `Cancel` 事件，照 P4 釋放。正式命令 `agend task cancel <task>`（只收操作者；使用者 2026-09-26 決定）：語法歸第 9 施工關，daemon 端在本關。
    - task 已經在 merge 關卡（包括 `merge-blocked`）時，core 不接受取消（merge 送出後不能取消，pipeline.md）：`task_cancel` 回錯誤 `merge_in_flight: <task> is merging; it cannot be cancelled now`。`merge-blocked` 的出路是把擋住的 checkout 清乾淨（commit 或 stash 你的修改、或切離 main），再按 `retry`。
- 理由：這些都只有接上 pipeline 才有真資料可驗；team 與 workflow 的命令是跑 pipeline 的前提，放在同一關才不會兩邊互等。
- 替代方案：team 與角色放進第 9 施工關的 `instance add` 旗標（第 9 施工關就要先有 `teams` 表）；請示本關不做（第 11 施工關 B 段的「回答請示」就沒有真來源）。
- 例子：假 reviewer 在 `t-3/review/2` 已經開始後才送 `agend review approve t-3/review/1` → `stale_result`；`agend team join g10 g10-rev --role reviewer` → 排隊中的審查馬上派給它，`no-role:g10/reviewer` 消失。
- [x] 使用者確認（2026-09-26）

### P11：什麼是假的、什麼是真的

- 問題：「假 driver」指什麼？沒有真 backend，誰來 commit、誰來審查？送達怎麼算確認？
- 建議：
  - **真的**：git（系統的 git，≥ 2.38）、暫存 repo（`/tmp/g10-…`）、forge local、runner（`sh`）、SQLite、holder、shim 與 hook、`agend daemon` binary、client 協定、CLI。
  - **假的**：agent 的腦袋。testkit 加一個假 agent 程式 `fake-worker`（放在 holder 裡跑，跟第 6 施工關的計數器一樣）：每秒跑一次 `agend inbox --after <上次最後一則的 id>`（游標存在自己 workspace 的檔案裡），收到派工就在 worktree 寫檔、經 shim `git commit`、`agend done <ticket>`；收到審查就 `agend review approve <ticket>`。旗標：`--fail-checks-once`（第一次故意不加檔案）、`--leave-wip`（done 前留一個未 commit 的檔）、`--changes-once`（reviewer 第一次要求修改）、`--hold`（收到派工後什麼都不做）。
  - 送達：instance 加一欄 `delivery`（`push` 預設／`inbox`）。**與 [delivery](../architecture/delivery.md#送達模型)「推送一律帶完整內容、inbox 只作補查」不同，使用者 2026-09-26 決定照這裡做**：`inbox` 多了一條只靠拉取的路，只給沒有 driver 的程式（假 agent）用。`inbox` 的 instance daemon 不建立任何 driver；假 agent 的 backend 固定登記成 `claude`（第 12 施工關之前沒有 claude driver，`fake-worker` 忽略第 6 施工關加的 `--session-id`／`--resume` 參數），**不用 `codex`**（會帶出第 7 施工關的 driver 與啟動包裝）。`inbox` 的 instance daemon 不主動推：訊息寫進第 7 施工關的 `messages` 表就記 `sent`（已放到 agent 拿得到的地方）。**讀取不算確認**（第 9 施工關的 `inbox` 是唯讀游標）。確認的來源是 agent 用了它：work 派工訊息用 `dispatch:<ticket>`；多人審查共用關卡 ticket，但每位 reviewer 的訊息用 `dispatch:<ticket>/<reviewer>`，避免全局 message id 衝突。daemon 接受同 ticket 且身分匹配的結果命令（`done`、`result`、`review`）時，在 task／snapshot／event 的同一個 CAS 交易中把該筆訊息標 `confirmed`；其他訊息沒有這種回應，就一直是 `sent`（照第 7 施工關「不能確認就誠實標未確認」）。等第 12 施工關有 claude driver，真的 claude 仍是 `push`，不受影響。
  - pipeline 迴圈的單元測試用 testkit 的假實作（`FakeStore`、`FakeForge`、`FakeRunner`、`FakeDriver`、`FakeClock`）；整合測試與 demo 用上面「真的」那一組。
  - 開發用 example `pipeline_probe`：
    - `setup`（daemon 停著時，直接開 DB；`AGEND_HOME` 沒設就拒絕，比照第 6 施工關 P1）：在 `$AGEND_HOME` 建 home、在 `$AGEND_HOME-repo` 建暫存 repo、team `g10`（`g10-dev` dev、`g10-rev` reviewer）與 team `g10h`（`g10-hold`，`--hold` 的假 agent）、`demo` 與 `slow` 兩個 workflow。
    - 開 task 不需要 probe：操作者直接 `agend task create --team …`（P10）。
    - `cancel <task>`（daemon 跑著時）：以操作者身分送 `task_cancel`。只在開工時第 9 施工關還沒有 `agend task cancel` 時用；有了就不做這個子命令。
    - `teardown`：照第 6 施工關收掉 holder、刪暫存目錄。
  - `demo` workflow：work(dev, branch) → submit(local) → checks（`test -f hello.txt`）→ review(reviewer, 綁 head) → approve(human, 綁 head) → merge。`slow` 一樣，只是 checks 先 `sleep 20`。`escape` 一樣，只是 checks 是 `touch <repo>/escaped && test -f hello.txt`（`setup` 時寫入 repo 的絕對路徑），用來看沙箱擋寫入。
  - `check-deps`：不加新規則（`agend-daemon` 不能依賴 `agend-shim`／`agend-holder` 已有；hook 經子命令）。
- 理由：要驗的是 daemon 的流水線，不是 LLM；假 agent 走的是真的 CLI、真的 shim、真的 hook，只有「決定寫什麼」是假的。`delivery = inbox` 是一欄一個分支，而且不綁任何 backend，第 12 施工關之後照樣能用。
- 替代方案：用第 7 施工關的 codex driver ＋假 app-server，讓假 app-server 收到訊息時執行腳本（要改假 app-server，而且綁 codex）；測試直接在 process 裡扮 agent、不經 CLI（驗不到 ticket 與 shim）；讀了就算確認（agent 讀到不代表看懂或照做，而且第 9 施工關的游標是唯讀的）；加一個 `fake` backend（`Backend` 只能是三個，要改決策）。
- 例子：`AGEND_HOME=/tmp/g10.ab12` 時 `pipeline_probe setup` 印 `repo=/tmp/g10.ab12-repo`、`team g10: dev g10-dev, reviewer g10-rev`、`team g10h: dev g10-hold (--hold)`、`workflow demo v1: work -> submit -> checks -> review -> approve(human) -> merge`。
- [x] 使用者確認（2026-09-26）

## 下一步

閱讀 [已決定事項與邊界](gate-10-proposal-boundaries.md)。
