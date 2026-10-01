# Gate 10 提案：狀態、持久化與派工

> **TL;DR**
> - P1–P3 為使用者已確認的 runtime 提案。
> - 狀態與驗證證據見 [Gate 10 入口](gate-10-pipeline.md)。
> - 下一步：閱讀 [worktree／checks 提案](gate-10-proposal-worktrees.md)。

### P1：誰推進 task、怎麼存檔

- 問題：事件從很多地方來（agent 的命令、checks 結果、forge、逾時、你按的核准）。誰呼叫 `step`？存檔和做事哪個先？兩件事同時到怎麼辦？
- 建議：
  - daemon 裡**只有一個 pipeline 迴圈**（一個 tokio task，吃一條 channel）。所有事件都排進這條 channel，一次處理一個：讀 task 的狀態 → `step` → **先存檔** → 再把 actions 交給執行者（git、runner、forge、送達）。執行者做完，把結果當成新事件排回 channel。
  - 存檔是一個交易：task 列（含 P2 的快照欄位）用 CAS 寫入，同時附加 `task_events`。core 的 `Store` trait 加一個方法（例如 `advance_task(task_id, expected_version, 新狀態, 事件)`）；testkit 的 `FakeStore` 與 STO 契約一起補，新規則「存檔與事件同成同敗」。
  - `step` 回錯誤（`StaleResult`、`MergeInFlight`…）：什麼都不存；事件來自 agent 的命令就把錯誤碼回給它（`stale_result` 已在 client 協定，訊息照第 9 施工關 P2 印出目前的 ticket）。
  - 只有一個寫入者，CAS 衝突照理不會發生；發生就是 bug：記 error log、重讀、放棄這個事件（不重試）。
  - core 的 `TaskStatus` 沒有「失敗」「取消」：加 `Failed`、`Cancelled`（core 改動，第 1 施工關測試重跑；migration 重建 `tasks.status` 的 CHECK）。`agend status` 看的是 `tasks.status`，不必解析快照。
- 理由：一條迴圈就沒有「兩個事件同時改同一個 task」的問題，不用鎖；先存再做，daemon 在任何一刻死掉，DB 裡都是某個完整的狀態，沒做完的事由 P9 重做。
- 替代方案：每個 task 一個 actor（並行度高，但目前 task 數少，多一層排程）；先做事再存（死在中間時做過的事 DB 不知道，例如 merge 做了卻沒記錄）；outbox 表存待做的 actions（多一張表；P2 改用「從狀態算出來」）。
- 例子：假 agent 對同一個 ticket 送兩次 `agend done t-3/work/1`：第一次 task 進 submit；第二次 `step` 回 `StaleResult`，DB 不變，agent 看到 `stale_result: t-3/work/1 is no longer current (now t-3/submit/1)`。
- [x] 使用者確認（2026-09-26）

### P2：`PipelineState` 快照、怎麼防偽造、重開機後要重做什麼

- 問題：第 5 施工關 P4 已定「存快照」。格式怎麼鎖？`PipelineState` 欄位私有、「不可偽造」，從 JSON 讀回來算不算偽造？daemon 重開後怎麼知道哪些事做到一半？
- 建議：
  - core 加 serde derive 到 `PipelineState` 與它的欄位型別（D32 延伸，見 [D38](../decisions/d38.md#d38)）。**快照不含 workflow**：workflow 由 task 的 `workflow_id`＋`workflow_version`（D21）從 `workflows` 表讀，不存兩份。
  - 讀回只有一條路：`PipelineState::restore(快照, ValidatedWorkflow) -> Result`，檢查 task id、關卡位置在範圍內、每個關卡都有 attempt、紀錄裡的 stage id 都在 workflow 裡。壞掉的快照不會 panic：那個 task 標 `Failed`，出現在「需要你」（P8），其他 task 照常。
  - golden JSON 測試鎖格式；只准加欄位（比照 D26）。舊快照要讀得回來（每加欄位附一份舊 fixture，比照第 5 施工關 P5）。
  - core 加純函式 `outstanding_actions(&PipelineState) -> Vec<PipelineAction>`：目前關卡「送出了、還沒收到結果」的要求（例如 checks 的 `RunCommand`、merge 送出中的 `Merge`）。探索器加一條不變量：每一步之後，它等於「這一步送出、還沒被回應」的那些要求。P9 開機時用它重做，不另存 actions。
  - 新 migration（第 8 施工關用 `0003`、第 7 施工關用 `0004`；本關用開工時的下一個空號，目前是 `0005`）：`tasks` 加 `pipeline`（JSON 快照）、`stage_entered_at_unix_ms`（進入目前關卡的時間：逾時與「需要你」的等待起點都從它算，不看會在 14 天後被刪的事件）、`merge_intent`（P7）。P3 的 `teams` 表與 instance 欄位、P4 的 `bindings` 表、P10 的 `asks`／`ask_turns`／`reminders` 表、P11 的 `delivery` 欄位也在同一個 migration，每張新表都列進第 5 施工關 P8 的保留規則表。
- 理由：重播事件的問題第 5 施工關已經講過；快照加一道 `restore` 檢查，外面仍然造不出不合規則的狀態（竄改測試照跑）。「還在等什麼」從狀態算，不必多一張會跟狀態對不上的表。
- 替代方案：快照連 workflow 一起存（兩份真相）；不檢查直接 `Deserialize`（等於開放偽造）；outbox 表存 actions（多一張表、兩份真相）。
- 例子：task 在 checks 第 2 次時 daemon 被硬殺；重開後 `outstanding_actions` 回 `RunCommand{checks, attempt 2, head a1b2…}`，daemon log `t-7: re-running checks t-7/checks/2 after restart`。
- [x] 使用者確認（2026-09-26）

### P3：team、角色、分派；supervisor 跟 task 的關係

- 問題：目前只有 `general`（沒有 repo）。task 要有 repo 才能走 `code`。instance 沒有角色，怎麼分派？instance 死了、被刪了，手上的 task 怎麼辦？
- 建議：
  - `teams` 表（`id`、`repo`＝canonical checkout 的絕對路徑、可空、`default_workflow`），保留期限「永久」；`instances` 加 `team_id`（預設 `general`）、`role`。設定用本關的 `agend team` 命令（P10）。
  - 分派用 core 的 `policy::assign::choose`：候選人＝同 team 裡**每一個**狀態 `running` 的 instance（不先過濾），`held_task` 照實填：`tasks.assignee` 的未結束 task，或正在做的審查；誰有空、返工回持有者（`Rework`）、審查排除作者，都由 core 判斷。daemon 先過濾會讓返工找不到持有者（變成接手或排隊）、`NoEligibleReviewer` 永遠不出現；人數上限先等於現有人數，所以**不開臨時 instance**（`SpawnEphemeral` 不會出現）。沒有人可用 → task 留在原地排隊，下一次有 instance 空出或加入時再試。team 裡沒有這個角色 → 同樣排隊，另外出現「需要你」`no-role`（P8）。
  - task 持有者（D33）記在 `tasks.assignee`，等待 checks／審查／人工核准與真正返工都保留原持有者，到 task 結束才清掉。D34 的 `planned` 有明確向前交接：planner 的 Result 經人工核准後，前進到不同角色的 dev Work，先釋放 planner binding 並保存 WIP，再清 assignee、交由 core `NewTask` 選 dev；這不屬於 `ReturnToWork`，failed holder 的真正返工仍等 retry。審查者持有那次審查，核准或要求修改後就空出來。
  - supervisor（第 6 施工關）不動：instance `failed` 時它手上的 task **停著等**；第 8 施工關那個「需要你」項目的 `unblocks` 從 0 變成 1。你按 `retry` 讓 agent 回來後，它的 binding 還在，照原來的工作繼續。
  - 本關**不做改派**：額度用盡、逾時動作 `reassign` 都在之後的施工關；用到 `on_timeout = reassign` 的 task 在建立時被拒（訊息寫這個動作還不支援）。
  - **與 D33 第 3 點不同，使用者 2026-09-26 決定照這裡做**：D33 說「持有者被操作者刪除 → 立即改派」。本關改成拒絕刪除手上有未結束 task 的 instance（第 9 施工關的 `agend instance remove <name>` 收到的錯誤指出 task，並提示先 `task_cancel`，P10）。改派要交接 branch 與審查意見，是本關最大的一塊，本關先不做。
- 理由：一條最短的路：一個 team、一個 dev、一個 reviewer 就能走完。改派牽涉最多，等有真 backend 額度資料時再做。
- 替代方案：instance 的 team／角色寫在設定檔（D8：持久狀態只在 DB）；本關就做角色範本與臨時 instance（要選 backend、模型，第 12 施工關有真 backend 後比較驗得到）；照 D33 本關就做刪除時改派（範圍加上交接）。
- 例子：team `g10` 有 `g10-dev`（dev）與 `g10-rev`（reviewer）。建第一個 task → `g10-dev` 接；建第二個 → log `t-2: queued (no free dev in g10)`；第一個 merge 後，第二個自動派給 `g10-dev`。
- [x] 使用者確認（2026-09-26）

## 下一步

閱讀 [worktree／checks 提案](gate-10-proposal-worktrees.md)。
