# 第 10 施工關：daemon：流水線（`pipeline`）

> **TL;DR**
> - daemon 用 core 的狀態機把一個 task 從派工推到 merge：建 worktree 並裝 hook、跑 checks、agent 審查、你核准、在本機 repo merge；daemon 被硬殺後接著做，不重複 merge。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：逐題看「開工前提案」P1–P11 與「待你決定」，每題打勾或寫下你要改的地方；確認後 merge 這份提案，等第 7–9 施工關完成再開工。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**提案中**（2026-09-26）：開工前提案 P1–P11 待你確認。依賴：第 6 施工關已 merge（#125）；第 7（送達、`messages` 表）、第 8（client 協定 1.1、「需要你」）、第 9（CLI 語法、ticket、`operator` 請求）施工關還在提案或實作中。分工見下方「範圍」的最後一段。

## 範圍

- 驅動 core 狀態機：daemon 裡唯一的 pipeline 迴圈推進 task，每一步先 CAS 存檔再做事（P1）
- `PipelineState` 存成 task 資料列上的快照欄位，與 task 一起 CAS，不靠 replay events 重建（第 5 施工關 P4）；D32 延伸到 `PipelineState`，golden JSON 測試；core 加一個純函式列出「還在等結果的要求」；第 1 施工關的測試要重跑（P2）
- team、角色與分派：`teams` 表、instance 的 team／角色、core `policy::assign`；supervisor 與 task 的關係（P3）
- worktree 與 binding：先記錄再建立、固定命名空間、審查 worktree、結束時清理並把 WIP 存成 patch（P4）
- 從第 6 施工關移來（[gate-06 P9](gate-06-daemon-holder.md#p9hooks-與-binding-快照依賴規則)，使用者 2026-09-26 追認）：綁定／釋放 worktree 時安裝／移除 agend hook；daemon 為每個 agent 寫唯讀 binding 快照 `$AGEND_HOME/bindings/<instance>.json`（見 [GLOSSARY](../GLOSSARY.md)「binding 快照」）；`check-deps` 規則「`agend-daemon` 不能依賴 `agend-holder` 或 `agend-shim`」照舊成立（P4）
- git adapter：daemon 自己跑的 git 全部經 `Runner`、帶 timeout、不經 shim、不跑任何 hook（P5）
- runner（`command` 關卡）：每次 checks 一個新的 detached worktree、環境白名單、timeout（P6）
- forge local：merge-tree + CAS `update-ref`、merge 前才處理 main 前進（D14）、merge 不會做兩次（P7）
- 「需要你」的新來源：人工核准、請示、task 失敗、merge 被擋、缺角色（P8）
- 開機對帳（reconcile）與四次開機的重啟契約（P9）
- 本關擁有的命令與請示（P10）
- 什麼是假的、什麼是真的（P11）

**與第 9 施工關的分工**（協調者 2026-09-26 定）：

| 誰 | 負責 |
|---|---|
| 第 10 施工關（本關） | 請示 `ask` 的 daemon 端與儲存；`done`、`result`、`review approve`／`changes`、`block`、`unblock`、`remind`、`task create` 的 daemon 端處理；`agend workflow`、`agend team` 命令（CLI 與 daemon 兩端）；新的操作者請求 `task_cancel` |
| 第 9 施工關 | 上面那些 agent 命令的 CLI 語法、client 接線、錯誤碼對應、哪些命令斷線後可以重送；ticket 格式 `<task>/<stage>/<attempt>`；`agend inbox --after`（唯讀游標）；`operator` 請求 |

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子。文件已定、這裡不重問的：狀態機是純函式 `step`、trait 在 core（D27）；`command` 關卡與 git 都經 `Runner`（D28）；forge 維持 3 個方法、GitHub CI 用 `command` 關卡接、forge local 沒有 change id（D29）；workflow 用 TOML 存 DB、task 固定建立時的版本（D19、D21）；人工核准 merge＝`approval(by = "human")`（D20）；一個 agent 一個 task、返工回持有者（D33 第 1、2 點）；main 前進的保留規則（D14）；worktree 只由 daemon 建、固定命名空間、先記錄再建立、WIP 存 patch、開機與每日的單一對帳（[pipeline](../architecture/pipeline.md#worktree-與-branch-生命週期)）；hook 設計與 binding 快照格式（第 3 施工關 T1、T21）；`PipelineState` 存成快照、不重播事件（第 5 施工關 P4）；daemon 起 holder、環境白名單、重起 3 次後 `failed`（第 6 施工關 P3、P6）；「需要你」的欄位、`resolve_attention` 只收操作者（第 8 施工關 P2、P5）；請示是對話（D35）；保留期限（D31）；結果類命令帶 ticket（第 9 施工關 P2）。

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
- [ ] 使用者確認

### P2：`PipelineState` 快照、怎麼防偽造、重開機後要重做什麼

- 問題：第 5 施工關 P4 已定「存快照」。格式怎麼鎖？`PipelineState` 欄位私有、「不可偽造」，從 JSON 讀回來算不算偽造？daemon 重開後怎麼知道哪些事做到一半？
- 建議：
  - core 加 serde derive 到 `PipelineState` 與它的欄位型別（D32 延伸，要新決策編號）。**快照不含 workflow**：workflow 由 task 的 `workflow_id`＋`workflow_version`（D21）從 `workflows` 表讀，不存兩份。
  - 讀回只有一條路：`PipelineState::restore(快照, ValidatedWorkflow) -> Result`，檢查 task id、關卡位置在範圍內、每個關卡都有 attempt、紀錄裡的 stage id 都在 workflow 裡。壞掉的快照不會 panic：那個 task 標 `Failed`，出現在「需要你」（P8），其他 task 照常。
  - golden JSON 測試鎖格式；只准加欄位（比照 D26）。舊快照要讀得回來（每加欄位附一份舊 fixture，比照第 5 施工關 P5）。
  - core 加純函式 `outstanding_actions(&PipelineState) -> Vec<PipelineAction>`：目前關卡「送出了、還沒收到結果」的要求（例如 checks 的 `RunCommand`、merge 送出中的 `Merge`）。探索器加一條不變量：每一步之後，它等於「這一步送出、還沒被回應」的那些要求。P9 開機時用它重做，不另存 actions。
  - 新 migration（開工時的下一個空號；第 8 施工關用 `0003`，第 7 施工關預定 `0004`）：`tasks` 加 `pipeline`（JSON 快照）、`stage_entered_at_unix_ms`（進入目前關卡的時間：逾時與「需要你」的等待起點都從它算，不看會在 14 天後被刪的事件）、`merge_intent`（P7）。P3 的 `teams` 表與 instance 欄位、P4 的 `bindings` 表、P10 的 `asks`／`ask_turns`／`reminders` 表、P11 的 `delivery` 欄位也在同一個 migration，每張新表都列進第 5 施工關 P8 的保留規則表。
- 理由：重播事件的問題第 5 施工關已經講過；快照加一道 `restore` 檢查，外面仍然造不出不合規則的狀態（竄改測試照跑）。「還在等什麼」從狀態算，不必多一張會跟狀態對不上的表。
- 替代方案：快照連 workflow 一起存（兩份真相）；不檢查直接 `Deserialize`（等於開放偽造）；outbox 表存 actions（多一張表、兩份真相）。
- 例子：task 在 checks 第 2 次時 daemon 被硬殺；重開後 `outstanding_actions` 回 `RunCommand{checks, attempt 2, head a1b2…}`，daemon log `t-7: re-running checks t-7/checks/2 after restart`。
- [ ] 使用者確認

### P3：team、角色、分派；supervisor 跟 task 的關係

- 問題：目前只有 `general`（沒有 repo）。task 要有 repo 才能走 `code`。instance 沒有角色，怎麼分派？instance 死了、被刪了，手上的 task 怎麼辦？
- 建議：
  - `teams` 表（`id`、`repo`＝canonical checkout 的絕對路徑、可空、`default_workflow`），保留期限「永久」；`instances` 加 `team_id`（預設 `general`）、`role`。設定用本關的 `agend team` 命令（P10）。
  - 分派用 core 的 `policy::assign::choose`：候選人＝同 team、同角色、狀態 `running`、手上沒有未結束 task 的 instance；人數上限先等於現有人數，所以**不開臨時 instance**（`SpawnEphemeral` 不會出現）。沒有人可用 → task 留在原地排隊，下一次有 instance 空出或加入時再試。team 裡沒有這個角色 → 同樣排隊，另外出現「需要你」`no-role`（P8）。
  - task 持有者（D33）記在 `tasks.assignee`，到 task 結束才清掉；審查者持有的是那次審查，核准或要求修改後就空出來。
  - supervisor（第 6 施工關）不動：instance `failed` 時它手上的 task **停著等**；第 8 施工關那個「需要你」項目的 `unblocks` 從 0 變成 1。你按 `retry` 讓 agent 回來後，它的 binding 還在，照原來的工作繼續。
  - 本關**不做改派**：額度用盡、逾時動作 `reassign` 都在之後的施工關；用到 `on_timeout = reassign` 的 task 在建立時被拒（訊息寫這個動作還不支援）。
  - **與 D33 第 3 點不同，請明確決定**：D33 說「持有者被操作者刪除 → 立即改派」。本關改成拒絕刪除手上有未結束 task 的 instance（第 9 施工關 `instance remove` 收到的錯誤指出 task，並提示先 `task_cancel`，P10）。改派要交接 branch 與審查意見，是本關最大的一塊，本關先不做。
- 理由：一條最短的路：一個 team、一個 dev、一個 reviewer 就能走完。改派牽涉最多，等有真 backend 額度資料時再做。
- 替代方案：instance 的 team／角色寫在設定檔（D8：持久狀態只在 DB）；本關就做角色範本與臨時 instance（要選 backend、模型，第 12 施工關有真 backend 後比較驗得到）；照 D33 本關就做刪除時改派（範圍加上交接）。
- 例子：team `g10` 有 `g10-dev`（dev）與 `g10-rev`（reviewer）。建第一個 task → `g10-dev` 接；建第二個 → log `t-2: queued (no free dev in g10)`；第一個 merge 後，第二個自動派給 `g10-dev`。
- [ ] 使用者確認

### P4：worktree、binding、hook、binding 快照

- 問題：worktree 什麼時候建、建在哪？hook 什麼時候裝、怎麼裝（daemon 不能依賴 `agend-shim`）？binding 快照誰寫、什麼時候寫？審查的 agent 在哪看程式？task 結束時留著的修改怎麼辦？
- 建議：
  - 綁定（work 關卡第一次派給持有者時）：
    1. DB 的 `bindings` 表寫一列 `pending`（instance、task、kind、worktree、branch）。
    2. 在 canonical checkout 跑 `git worktree add -b agend/<task>/<slug> $AGEND_HOME/worktrees/<task>/ <main 的 SHA>`。
    3. 裝 hook：daemon 跑 `agend hooks install <worktree>`（`agend` 的內部子命令，呼叫 `agend_shim::install_hooks`；不在 `--help` 列出）。
    4. 寫 binding 快照（先寫暫存檔再 rename、0444）。
    5. `bindings` 標 `ready`，然後才送派工訊息。
  - 每一步都可重做：死在中間，P9 從 `pending` 接著做；worktree 已存在就檢查它是不是這個 branch。
  - binding 快照的型別從 `agend_shim::binding::Snapshot` 搬到 core（第 3 施工關 T1 已建議）：路徑改成 `String`（core 是 no_std），shim 與 daemon 共用同一個型別；golden JSON 測試證明格式沒變。這也是 D32 的延伸，跟 P2 一起給新決策編號。`bindings` 表的列在釋放時刪除。
  - 審查（`approval(by = <角色>)`）：reviewer 拿到 detached 的審查 worktree `$AGEND_HOME/worktrees/<task>-review/`（在審的 head），binding kind `review`，一樣裝 hook（審查 binding 不能寫任何 branch）。核准或要求修改後就拆掉。`approval(by = "human")` 沒有 worktree，走「需要你」（P8）。
  - 釋放（task done、失敗、取消；或審查結束）：
    1. binding 快照先改成沒有 binding（shim 從這一刻起拒絕寫入）。
    2. 有未 commit 的修改或未追蹤檔 → 存成 `archive/<task>-<unix 秒>.patch`；task 沒 merge（失敗、取消）時，branch 上的 commit 也用 `format-patch` 存進同一個檔。保留 30 天（D31 的 WIP patch）。
    3. `agend hooks uninstall <worktree>` → `git worktree remove --force` → `git branch -D`。
    4. 刪掉 `bindings` 那一列；patch 路徑記在 task 事件，`agend status` 與 TUI 看得到。
- 理由：hook 是 protected ref 的硬保證（第 3 施工關威脅模型），一定要在 agent 拿到 worktree **之前**裝好；經子命令呼叫，daemon 就不必連結 shim（第 6 施工關 P9 的依賴規則）。一個型別兩邊用，格式不會漂移（#1493）。審查者看的是固定的 head，不是持有者正在改的 worktree。
- 替代方案：daemon 直接呼叫 `install_hooks`（違反依賴規則）；hook 在 agent 第一次跑 git 時由 shim 自己裝（agent 可以繞過 shim）；失敗或取消的 branch 留著不刪（v1 的 137 個 branch）；審查者直接讀持有者的 worktree（審的不一定是那個 head）；本關不做 agent 審查、只有人工核准（`code` workflow 跑不了，`review` 命令沒有用處）。
- 例子：task `t-3` 派給 `g10-dev` → `worktrees/t-3/` 出現、branch `agend/t-3/hello`、`bindings/g10-dev.json` 有 `{"kind":"work","task_id":"t-3",…}`；在那個 worktree 跑 `git update-ref refs/heads/main HEAD` → `agend-shim: refused … (agend reference-transaction hook)`。task merge 後三樣都不見，`bindings/g10-dev.json` 只剩 instance 與 repo。
- [ ] 使用者確認

### P5：daemon 自己跑的 git

- 問題：daemon 也要跑 git（建 worktree、算 patch-id、merge、看 head）。會不會被 shim 或 hook 擋？會不會觸發專案自己的 hook？卡住怎麼辦？daemon 會不會改 agent 的 worktree？
- 建議：
  - daemon 開機時找一次真的 git：`PATH` 上第一個**不在** `$AGEND_HOME/bin` 的 `git`，記住絕對路徑；版本低於 2.38（`merge-tree --write-tree` 需要）就不處理有 repo 的 team，log 指出原因（`agend doctor` 由第 9 施工關檢查）。
  - 每個 git 呼叫：`env_clear` 後只給 `HOME`、`PATH`、`LANG=C`、`GIT_TERMINAL_PROMPT=0`（沒有 `AGEND_*`、沒有 daemon 的 `GIT_*`）；一律加 `-c core.hooksPath=/dev/null`（不跑 agend hook，也不跑專案的 hook）；經 `Runner`，timeout 60 秒。
  - 在 canonical checkout 跑：`worktree add/remove/list`、`branch -D`、`rev-parse`、`merge-base`、`diff`／`patch-id`、`log`、`merge-tree`、`commit-tree`、`update-ref`。
  - 在 agent 的 worktree 裡：讀（`status`、`diff`，給 P4 的 WIP patch 用）；以及 P7 的 rebase——**這會在持有者的 worktree 裡產生新的 commit、改寫它的 branch**，所以只在 task 不在 work 關卡、worktree 乾淨時做，不乾淨就不做、當成衝突。除此之外 daemon 不在 agent 的 worktree 裡寫任何東西。
  - 什麼時候看 branch 的 head：沒有輪詢。只在「收到任何結果事件時」與「送出 `Merge` 前」讀一次；和狀態裡的 head 不同就先餵 `CommitCreated`（附新的 patch-id），再處理原本的事件。
- 理由：第 3 施工關已定「可信的呼叫者用 `-c core.hooksPath=/dev/null`」；daemon 沒有 agent 身分，不關 hook 會被自己的 hook 擋。專案 hook（例如 husky 的 `post-checkout` 跑 `npm install`）在 `worktree add` 時跑，會又慢又不可預期。每個外部指令都有 timeout（V1-LESSONS #10）。
- 替代方案：用 `AGEND_SHIM_BYPASS=1` 跑 PATH 上的 git（只跳 shim，hook 照樣擋）；保留專案 hook（`worktree add` 可能跑幾分鐘）；定時輪詢每個 branch 的 head（多數時間白跑）。
- 例子：agent `done` 之後又 commit 了一次才被 checks 跑到：checks 結果回來時 daemon 發現 head 變了 → log `t-3: head moved a1b2… -> c3d4…; checks run again`，舊的結果不算。
- [ ] 使用者確認

### P6：`command` 關卡（runner）

- 問題：checks 在哪跑？用什麼環境？跑多久算逾時？輸出存哪？daemon 被硬殺時正在跑的 check 怎麼辦？
- 建議：
  - **每次跑一個新的** detached worktree：`$AGEND_HOME/checks/<task>-<stage>-<attempt>-<unix 毫秒>/`，在要測的 head；跑完（不管結果）就刪。不裝 hook、沒有 binding。
  - 指令：`sh -c <已展開的指令>`，自己一個 process group（RUN-8），stdin 是 `/dev/null`。環境：第 6 施工關的 agent 白名單，但**沒有** `AGEND_*`，`PATH` 也拿掉 `$AGEND_HOME/bin`。
  - timeout：`RunCommand` 帶來的 `timeout_ms`（關卡沒寫就是 core 的預設 5 分鐘，`DEFAULT_STAGE_TIMEOUT_MS`）。逾時 → 停掉整個 process group → 餵 `CommandFinished{exit_code: None}`：core 當成 checks 失敗、退回 work，持有者會收到「checks timed out after N s」。**不餵 `StageTimedOut`**：預設逾時動作是「通知」，只記一筆，task 會一直停在 checks、「需要你」也沒有。所以 `command` 關卡的 `on_timeout` 沒有作用：寫了 `on_timeout` 的 `command` 關卡在建立 task 時被拒（跟 `reassign` 一樣），訊息寫「command 逾時一律當失敗、退回 work」。
  - 每次都是冷的 worktree，5 分鐘可能不夠：workflow 要自己寫 `timeout_ms`。本關的 `demo` workflow 寫 60 秒（checks 是 `test -f`），`slow` 寫 120 秒；Rust 專案建議至少 30 分鐘或設共用的 `CARGO_TARGET_DIR`（見已知風險）。
  - 輸出：完整 stdout／stderr 存 `logs/checks/<task>/<stage>-<attempt>.log`（每個檔最多 10 MiB，超過截斷並註明）；task 事件只放最後 20 行。保留 14 天（跟事件一樣，列進第 5 施工關 P8 的規則表）。
  - 同時最多跑 1 個 check，其他排隊（log 寫出在等誰）。
  - daemon 被硬殺時正在跑的 check 會變孤兒、跑到自己結束，結果沒人收；重開後 P9 對同一個 attempt 在**另一個新目錄**重跑，兩者不共用檔案；舊目錄由對帳刪掉。
  - 不做沙箱。**checks 跑的是 agent 寫的程式碼**：指令是你寫進 workflow 的（例如 `cargo test`），但它會執行 agent 改過的測試、`build.rs`、腳本，而且沒有 hook 保護（見「已知風險」）。
- 理由：新的 worktree 就在要測的那個 head 上，沒有上一次留下的檔案，所以「merge 出來的樹＝測過的樹」（P7）；跟 agent 的 worktree 分開（`runner.rs` 的 Must NOT、[pipeline](../architecture/pipeline.md#6-種關卡)「在 head 的臨時 detached worktree 執行」）；孤兒 check 不會跟重跑的撞在同一個目錄。一次一個最簡單，也不會讓兩個 `cargo test` 搶 CPU 互相逾時。
- 替代方案：每個 task 重複用一個 checks worktree、保留 ignored 檔（增量編譯快，但上一次的產物可能讓測試假綠，也會跟孤兒撞目錄）；在 agent 的 worktree 跑（agent 可能正在改）；同時跑多個（要設上限與排序）；容器或 `sandbox-exec`（兩個平台不同，本關先不做）；checks worktree 也裝 agend hook（沒有 binding 時 hook 拒絕寫任何 branch，可以擋住測試改 main；但會讓在 repo 裡建 branch 的測試失敗）。
- 例子：checks 是 `test -f hello.txt`，第一次 agent 忘了加檔 → exit 1 → `t-3: checks t-3/checks/1 failed (exit 1); back to work (g10-dev)`；agent 補上再 `done` → `checks t-3/checks/2 passed`。
- [ ] 使用者確認

### P7：forge local 與 merge

- 問題：merge 怎麼做才不動到任何人的工作目錄？main 被某個 worktree checkout 著怎麼辦？main 在這期間前進了怎麼辦？怎麼保證 daemon 死在 merge 中間、甚至斷電丟了 DB 的最後幾筆，也不會 merge 兩次或把沒 merge 的當成已 merge？
- 建議：
  - `done` 時拒絕空的 branch：head 等於 main、或已經是 main 的祖先（沒有任何新 commit）→ 回錯誤 `nothing to merge: agend/<task>/<slug> has no commits beyond main; commit your work, then agend done <ticket>`，task 留在 work。所以之後「head 在 main 裡」不會是因為 branch 本來就是空的。
  - merge 步驟（在 canonical checkout）：
    1. `git merge-tree --write-tree <main> <head>`；有衝突 → 回 `MergeFailed`。
    2. `git commit-tree`：父 commit 是 main 與 head（永遠是一個 merge commit，不 fast-forward）；訊息 `Merge agend/<task>/<slug>: <title>`，加 trailer `Agend-Task: <task>`。
    3. 把這個 commit 的 SHA 存進 `tasks.merge_intent`（CAS），**再**移動 main。
    4. 移動 main 前先 `git worktree list --porcelain`，看 main 被誰 checkout：
       - 沒有任何 worktree checkout main → `git update-ref refs/heads/main <新> <舊>`（CAS）。
       - 只有 canonical checkout 在 main，而且工作目錄乾淨 → 在那裡 `git merge --ff-only <新>`（main 被別人動過就失敗，等同 CAS；工作目錄一起更新）。**與 [pipeline](../architecture/pipeline.md#merge-與-main-前進)「不在使用者工作目錄 `git merge`」不同，請明確決定**：這裡只做 fast-forward、只在乾淨時做，不會產生衝突或新 commit；不同意的話改成「main 被任何地方 checkout 就 `merge-blocked`」。
       - 其他情況（canonical 在 main 但有未 commit 的修改、或別的 worktree checkout 了 main）→ **不 merge**，出現「需要你」`merge-blocked`（P8），寫出是哪個目錄。
  - main 前進（D14）**只在送出 merge 前檢查一次**：head 不包含目前的 main → daemon 在持有者的 worktree 裡 rebase（P5）→ 餵 `MainAdvanced{rebased_head, patch_id, conflict}` 再餵 `MergeFailed`（core 已規定送出中的 head 變更要等 `MergeFailed` 才套用）。patch-id 相同 → 保留核准、重跑 checks；不同或衝突 → 回 work。所以 checks 一定是在「已包含最新 main 的 head」上通過的，merge 出來的樹就是測過的樹。
  - 判斷「這個 task 是不是已經 merge 了」（送 `Merge` 前、以及 P9 開機時）：
    1. 在 main 的 first-parent 歷史裡找 trailer `Agend-Task: <task>`、第二個父 commit 等於目前 head 的 commit（`git log --first-parent --grep`，最多看 1000 個）→ 找到就是已 merge，merge commit 取它。
    2. 找不到，但 `merge_intent` 有值而且在 main 裡 → 已 merge，取 `merge_intent`。
    3. 都沒有，但 head 已經在 main 裡（有人手動 merge 了它）→ 也算完成：餵 `MergeCompleted`，merge commit 取 main 的 first-parent 歷史裡**最舊（最早）**一個包含 head 的 commit，task 事件註明「merged outside agend」。**與 [pipeline](../architecture/pipeline.md#merge-與-main-前進)「done 以 daemon 的 merge 記錄為準，不從 git 推論」不同，請明確決定**：這一條是從 git 推論 done（只在這種手動 merge 時）；不同意的話要改 core，讓 merge 送出中也能收 `StageFailed`，task 失敗交給你。不用 `StageFailed`：送出 `Merge` 之後 core 只接受 merge 結果與 head 變更（`MergeInFlight`），`StageFailed` 會被拒、task 卡住。
    4. 以上都不是 → 還沒 merge，照上面的步驟做。
  - 只對 forge local 成立（它從不 squash、一定寫 trailer）；GitHub 在第 12 施工關另外處理。done 以 DB 裡的 `merge_commit` 為準；不從 git 推論其他 task（V1-LESSONS #6）。
- 理由：`merge-tree` 不碰任何工作目錄（pipeline.md 已定）；拒絕空 branch 之後，「已經 merge」的證據只剩我們自己寫的 trailer 與 `merge_intent`，兩者都綁這個 task，不會把別人的 commit 當成自己的 merge。只在 merge 前處理 main 前進，只有一個觸發點。
- 替代方案：只看「head 在 main 裡」、不拒絕空 branch（空 branch 一 `done` 就被當成已 merge）；手動 merge 時讓 task 失敗（要改 core：送出 `Merge` 後 core 不接受 `StageFailed`）；main 一前進就 rebase 所有進行中的 task（要處理 agent 正在改的 worktree）；fast-forward merge（main 上看不出哪個 task 什麼時候 merge，也沒有 trailer 可找）；main 被 checkout 時照樣 `update-ref`（那個工作目錄會變成一堆反向修改）。
- 例子：t-3、t-4 都到了 merge。t-3 先 merge；t-4 送 merge 前發現 head 不含新的 main → rebase 無衝突、patch-id 相同 → log `t-4: main advanced; rebased, approvals kept, checks run again` → checks 通過 → merge。`git log --oneline main` 最上面兩個是 `Merge agend/t-4/…`、`Merge agend/t-3/…`。
- [ ] 使用者確認

### P8：「需要你」的新來源與協定補充

- 問題：人工核准、請示、task 失敗、merge 被擋、缺角色，你怎麼知道、怎麼處理？項目 id 怎麼跟 ticket 對上？
- 建議：沿用第 8 施工關的 `attention_required`／`resolve_attention`／`attention_resolved`，本關加五種來源（跟 `instance-failed` 一樣從 DB 算，不另存表）：

  | `attention_id` | 什麼時候出現 | `actions` | 按了之後 |
  |---|---|---|---|
  | `approval:<task>/<stage>/<attempt>` | `approval(by = "human")` 關卡等你 | `approve`、`request_changes` | `ApprovalGranted`（綁 head 的關卡綁送出要求時的 head）或 `ChangesRequested`（理由必填） |
  | `<ask id>`（不加前綴） | agent `agend ask`（P10） | 照第 8 施工關的請示：`answer_ask` | 回答送回 agent；agent 追問就再出現，結論後消失 |
  | `task-failed:<task>` | task 失敗（含 P2 快照壞掉） | `acknowledge` | 從清單拿掉；task 保持 `Failed`，WIP 已存 patch |
  | `merge-blocked:<task>` | main 被 checkout 著不能動（P7） | `retry` | 再檢查一次、可以就 merge；這時不能取消（P10），要先把那個 checkout 清乾淨 |
  | `no-role:<team>/<role>` | team 裡沒有這個角色，或審查時這個角色只有作者（P3、P10）；都在忙不算 | 無 | 你用 `agend team join` 加了這個角色後自動消失 |

  - **id 的寫法**：請示照第 8 施工關 P5，直接用 ask id、不加前綴；其他是 `<種類>:<對象>`，跟第 8 施工關的 `instance-failed:<id>` 一樣用 `:` 分開種類；對象是 task 關卡時**就是第 9 施工關的 ticket**（`t-3/review/1`），所以同一串字會同時出現在 `agend status`、派工訊息與「需要你」裡。
  - `request_changes` 要帶理由：`resolve_attention` 加一個選填欄位 `note`；缺少時回 `invalid_request`。
  - `approval:` 的 id 帶 attempt：head 變了、開了新的 attempt，舊項目自動消失、新項目出現；拿舊 id 按會回 `unknown_attention`，不會核准到新的 head。
  - `unblocks`：task 相關的是 1，`no-role` 是在排隊的 task 數；`waiting_since` 是 `tasks.stage_entered_at_unix_ms`（P2，重開機不重算；請示用 ask 建立的時間）；`task_id`、`instance_id`（持有者）照填。脈絡摘要（D37）本關只填最短的：目標＝task 標題、在問什麼＝「核准 `<branch>` 的 `<head 前 7 碼>`」、之後＝「merge 進 main」；完整內容是第 11 施工關的事。
  - 協定新增（`note`、P10 的 `task_cancel` 與 team／workflow 請求）算一次 minor：client 協定用**實作時的下一個 minor**（第 9 施工關先 merge 就是 1.3）。
  - **與 D18 字面不同，請明確決定**：D18 寫「需要不存在的角色時轉成 ask」。這裡改成 `no-role` 項目，因為 ask 是 agent 與你的對話（D35），而這件事的解法是「加一個成員」，不是回答問題。
- 理由：同一套「需要你」機制，TUI（第 11 施工關）與 Telegram（第 12 施工關）不必各做一次核准；只多一個選填欄位。
- 替代方案：人工核准做成請示（理由可以自由文字回答，但核准要綁 head，請示沒有身分）；`task-failed` 不放進「需要你」（失敗的 task 容易沒人發現）；id 全用 `:`（`approval:t-3:review:1`，跟 ticket 對不起來）。
- 例子：`demo` workflow 走到人工核准 → `agend debug watch` 印 `attention_required approval:t-3/approve/1 (unblocks 1; if ignored: t-3 waits for you) actions: approve, request_changes` → 你按 `approve` → `attention_resolved …` → merge。
- [ ] 使用者確認

### P9：開機對帳與四次開機的重啟契約

- 問題：daemon 被 `kill -9`、或斷電丟了最後幾筆寫入（第 5 施工關：macOS 沒開 `fullfsync`），重開後怎麼接著做？怎麼證明真的接得上？
- 建議：
  - 什麼時候：開機時（第 6 施工關的開機計畫做完後、第 8 施工關 bind socket **之前**）與每天一次（掛在第 6 施工關的 housekeeping，今天做過就跳過，照 pipeline.md 的「開機與每日」）。每天那次只做下面第 2、3 項。以 DB 為準：
    1. 每個未結束的 task：`restore` 快照（P2）；失敗 → `Failed`＋「需要你」。
    2. binding：`pending` 的接著建（P4）；`ready` 但 worktree 不見了 → 從 branch 重建；branch 也不見 → task `Failed`。重寫**全部** binding 快照檔（檔案只是 DB 的投影），刪掉 DB 沒有的 instance 的快照檔。
    3. 命名空間裡的孤兒（`agend/<task>/…` branch、`worktrees/<task>*`、`checks/<task>-*`，DB 沒有這個 task、它已結束、或是上一次開機留下的 checks 目錄）→ 照 P4 的釋放流程（先存 WIP patch 再刪）。命名空間外一律不碰。
    4. 重做 `outstanding_actions`（P2）：派工訊息用固定的訊息 id `dispatch:<ticket>` 重送（第 7 施工關的送達以 id 冪等）；checks 在新目錄重跑同一個 attempt（P6）；merge 送出中先照 P7 判斷是不是已經 merge。
    5. 逾時：`command` 與 `merge` 以外的關卡（merge 送出中 core 拒絕 `StageTimedOut`，每次開機都會多一行錯誤 log），deadline＝`stage_entered_at_unix_ms`＋關卡的 timeout，已經過了就立刻餵 `StageTimedOut`；`command` 的逾時由 runner 管（P6），重跑時重新計時。
  - 斷電丟了最後幾筆：DB 回到較舊的狀態，對帳就從那個狀態接著做。各種外部動作的處理：
    - merge：`merge_intent` 可能也丟了，所以 P7 先找 trailer，不靠 `merge_intent`。
    - worktree、hook：已存在就檢查、沿用。
    - 派工訊息：`messages` 那一列若也丟了，同一個 id 會再送一次，agent 可能看到兩次；它帶的 ticket 相同，第二次的結果回 `stale_result`，不會做兩次。這是接受的代價，不是「完全安全」。
  - 重啟契約（比照 CONTRACTS 的四次開機與第 6 施工關 P4，跨真的 process、真的 `agend daemon`）：
    - 開機 1（做事）：`pipeline_probe task` 建 task，假 agent 做完、`done`；checks 跑到一半時測試 `kill -9` daemon（測試自己起的 pid）。
    - 開機 2（閒置）：測試不做事；對帳在新目錄重跑 checks，task 走到 agent 審查、假 reviewer 核准、停在人工核准。
    - 開機 3（做事）：測試按 `approve`；在 failpoint `after-main-moved`（main 已經移動、`merge_commit` 還沒存）中止。
    - 開機 4（檢查）：task `done`；main 上這個 task 的 merge commit **剛好一個**；branch、worktree、checks 目錄都不見；每個 ticket 的派工訊息只送過一次。
    - 反向檢查：每次開機用新的 `AGEND_HOME` → 開機 2 就失敗。
  - failpoint：`AGEND_FAILPOINT=<名稱>` 讓 daemon 在那一點 `abort()`，**只在 debug build 編進去**。只有兩個：`after-merge-intent`、`after-main-moved`（merge 前後最窄的兩個窗口；其他地方用測試的 `kill -9` 就碰得到）。
- 理由：單一對帳取代 v1 約 9 個清理機制（V1-LESSONS #5）；靠 kill 的時機碰運氣測不到「剛好在 merge 中間」，failpoint 讓那一刀落在指定的地方。
- 替代方案：開機時不對帳、等事件自然發生（死在 checks 中的 task 永遠卡住）；每小時對帳（pipeline.md 寫的是每日，沒有需要更頻繁的證據）；失敗點用隨機 kill 多跑幾次（不穩、測不到窄窗口）；failpoint 也編進 release（正式 binary 多一條可以讓它自己 abort 的路）。
- 例子：`boot 3 daemon pid=<C> t-1 merge: main moved to 9f8e…; aborted at failpoint after-main-moved`、`boot 4 t-1: merge found on main by trailer (9f8e…); not merged again`。
- [ ] 使用者確認

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
    | `task create` | 照 D18（語法照第 9 施工關：`task create --role <role> "<title>" [--team] [--workflow]`）：team 預設是呼叫者的 team、workflow 預設是 team 的 `default_workflow`；`--role` 必須等於那個 workflow 第一個 work 關卡的角色，不同就拒絕並寫出應該是哪個（本關不做「改寫 workflow 的角色」）；存檔檢查、要 repo 的 workflow 在沒 repo 的 team 被拒；用到 fanout 或 `reassign` 被拒。審查者照 `policy::assign` 的 `Review`：排除 task 持有者（作者），優先不同 backend。只有兩種情況出現 `no-role:<team>/<role>`：team 沒有這個角色（`AskForRole`），或這個角色只有作者（`NoEligibleReviewer`）；角色有別人但都在忙 → 只排隊（`Queue{AtCapacity}`），不進「需要你」 |

  - 請示（D35）：`asks` 表（id、instance、task、狀態、建立時間）與 `ask_turns` 表（提問、選項、回答、追問、結論，依序），保留「永久」（D31）。`ask` 建一筆、出現在「需要你」；`answer_ask` 把回答記下並送給 agent（訊息 id `ask:<ask id>/<輪次>`）；`AskFollowUp` 再出現一次；`AskResolve` 結束。
  - 操作者命令（本關的 CLI，走第 9 施工關的 `operator` 請求）：
    - `agend team add <team> [--repo <path>] [--workflow <id>]`、`agend team list`、`agend team set-workflow <team> <id>`、`agend team join <team> <instance> --role <role>`
    - `agend workflow list`、`show <id>`、`check <file>`、`apply <file>`（D19 的 `new --from`、`edit`、`history`、`rollback`、`delete` 之後再做）
    - 新請求 `task_cancel { task_id, reason }`（只收操作者）→ `Cancel` 事件，照 P4 釋放。本關給 `pipeline_probe cancel` 用；要不要有正式 CLI 命令見「待你決定」。
    - task 已經在 merge 關卡（包括 `merge-blocked`）時，core 不接受取消（merge 送出後不能取消，pipeline.md）：`task_cancel` 回錯誤 `merge_in_flight: <task> is merging; it cannot be cancelled now`。`merge-blocked` 的出路是把擋住的 checkout 清乾淨（commit 或 stash 你的修改、或切離 main），再按 `retry`。
- 理由：這些都只有接上 pipeline 才有真資料可驗；team 與 workflow 的命令是跑 pipeline 的前提，放在同一關才不會兩邊互等。
- 替代方案：team 與角色放進第 9 施工關的 `instance add` 旗標（第 9 施工關就要先有 `teams` 表）；請示本關不做（第 11 施工關 B 段的「回答請示」就沒有真來源）。
- 例子：假 reviewer 在 `t-3/review/2` 已經開始後才送 `agend review approve t-3/review/1` → `stale_result`；`agend team join g10 g10-rev --role reviewer` → 排隊中的審查馬上派給它，`no-role:g10/reviewer` 消失。
- [ ] 使用者確認

### P11：什麼是假的、什麼是真的

- 問題：「假 driver」指什麼？沒有真 backend，誰來 commit、誰來審查？送達怎麼算確認？
- 建議：
  - **真的**：git（系統的 git，≥ 2.38）、暫存 repo（`/tmp/g10-…`）、forge local、runner（`sh`）、SQLite、holder、shim 與 hook、`agend daemon` binary、client 協定、CLI。
  - **假的**：agent 的腦袋。testkit 加一個假 agent 程式 `fake-worker`（放在 holder 裡跑，跟第 6 施工關的計數器一樣）：每秒跑一次 `agend inbox --after <上次最後一則的 id>`（游標存在自己 workspace 的檔案裡），收到派工就在 worktree 寫檔、經 shim `git commit`、`agend done <ticket>`；收到審查就 `agend review approve <ticket>`。旗標：`--fail-checks-once`（第一次故意不加檔案）、`--leave-wip`（done 前留一個未 commit 的檔）、`--changes-once`（reviewer 第一次要求修改）、`--hold`（收到派工後什麼都不做）。
  - 送達：instance 加一欄 `delivery`（`push` 預設／`inbox`）。**與 [delivery](../architecture/delivery.md#送達模型)「推送一律帶完整內容、inbox 只作補查」不同，請明確決定**：`inbox` 多了一條只靠拉取的路，只給沒有 driver 的程式（假 agent）用。`inbox` 的 instance daemon 不建立任何 driver；假 agent 的 backend 固定登記成 `claude`（第 12 施工關之前沒有 claude driver，`fake-worker` 忽略第 6 施工關加的 `--session-id`／`--resume` 參數），**不用 `codex`**（會帶出第 7 施工關的 driver 與啟動包裝）。`inbox` 的 instance daemon 不主動推：訊息寫進第 7 施工關的 `messages` 表就記 `sent`（已放到 agent 拿得到的地方）。**讀取不算確認**（第 9 施工關的 `inbox` 是唯讀游標）。確認的來源是 agent 用了它：派工訊息 `dispatch:<ticket>` 在 daemon 收到帶同一個 ticket 的結果命令（`done`、`result`、`review`）時標 `confirmed`；其他訊息沒有這種回應，就一直是 `sent`（照第 7 施工關「不能確認就誠實標未確認」）。等第 12 施工關有 claude driver，真的 claude 仍是 `push`，不受影響。
  - pipeline 迴圈的單元測試用 testkit 的假實作（`FakeStore`、`FakeForge`、`FakeRunner`、`FakeDriver`、`FakeClock`）；整合測試與 demo 用上面「真的」那一組。
  - 開發用 example `pipeline_probe`：
    - `setup`（daemon 停著時，直接開 DB）：建暫存 repo 與 home、team `g10`（`g10-dev` dev、`g10-rev` reviewer）與 team `g10h`（`g10-hold`，`--hold` 的假 agent）、`demo` 與 `slow` 兩個 workflow；印出 `export AGEND_HOME=…` 與 repo 路徑。
    - `task <team> <workflow> "<title>"`（daemon 跑著時）：以那個 team 裡某個假 agent 的身分送 `task create`（agent 本來就能開 task，D18）。操作者能不能直接開 task 見「待你決定」。
    - `cancel <task>`（daemon 跑著時）：以操作者身分送 `task_cancel`。
    - `teardown`：照第 6 施工關收掉 holder、刪暫存目錄。
  - `demo` workflow：work(dev, branch) → submit(local) → checks（`test -f hello.txt`）→ review(reviewer, 綁 head) → approve(human, 綁 head) → merge。`slow` 一樣，只是 checks 先 `sleep 20`。
  - `check-deps`：不加新規則（`agend-daemon` 不能依賴 `agend-shim`／`agend-holder` 已有；hook 經子命令）。
- 理由：要驗的是 daemon 的流水線，不是 LLM；假 agent 走的是真的 CLI、真的 shim、真的 hook，只有「決定寫什麼」是假的。`delivery = inbox` 是一欄一個分支，而且不綁任何 backend，第 12 施工關之後照樣能用。
- 替代方案：用第 7 施工關的 codex driver ＋假 app-server，讓假 app-server 收到訊息時執行腳本（要改假 app-server，而且綁 codex）；測試直接在 process 裡扮 agent、不經 CLI（驗不到 ticket 與 shim）；讀了就算確認（agent 讀到不代表看懂或照做，而且第 9 施工關的游標是唯讀的）；加一個 `fake` backend（`Backend` 只能是三個，要改決策）。
- 例子：`pipeline_probe setup` 印 `repo=/tmp/g10-1234/repo`、`export AGEND_HOME=/tmp/g10-1234/home`、`team g10: dev g10-dev, reviewer g10-rev`、`team g10h: dev g10-hold (--hold)`、`workflow demo v1: work -> submit -> checks -> review -> approve(human) -> merge`。
- [ ] 使用者確認

### 待你決定（開放問題）

- 操作者直接開 task：第 9 施工關把 `task create` 定為 agent 命令，操作者不能跑。本關的驗收改用 `pipeline_probe task`（以 agent 身分）。你平常要不要能直接 `agend task create`？建議：要，做成 `operator` 請求的一個變體，放在本關或第 9 施工關都可以——請決定放哪一關。
- 操作者取消 task 要不要正式 CLI 命令（例如 `agend task cancel <task>`）：建議要，跟上一題放同一關。
- 與決策或架構頁不同、各自在該題標了「請明確決定」的：D18（P8 `no-role`）、D33 第 3 點（P3 拒絕刪除持有者）、pipeline.md 不在使用者目錄 `git merge`（P7 的 `--ff-only`）、delivery.md 推送為主（P11 的 `delivery = inbox`）、pipeline.md 不從 git 推論 done（P7 的手動 merge 記成完成）。

### 本關不做（明確列出）

- fanout／`epic`（子 task）、`depends_on`、supersede、reopen：用到的 workflow 或操作在本關回錯誤並指出還不支援。
- 改派（持有者被刪、額度用盡、逾時動作 `reassign`）、臨時 instance、角色範本（P3）。
- forge github、GitHub CI（第 12 施工關）。
- 卡住偵測、usage limit、依緊急程度自動選忙碌等級（第 7 施工關 P9 說要移來本關：本關一律用 `Queue`，其餘建議移到第 12 施工關，有真 backend 資料時再做）。
- codex approval 轉給人回答（第 7 施工關 P9 說移到第 10、11 施工關：建議整個給第 11 施工關）。
- 派工時的檔案衝突警告（`policy::conflict`）：正確性由 P7 的 merge 前 rebase 保證，警告之後再加。
- `agend workflow` 的 `new --from`、`edit`、`history`、`rollback`、`delete`（P10）。
- 逾時動作「通知」只記 log 與 task 事件；Telegram 通知在第 12 施工關。
- checks 的沙箱與網路限制（P6）；Windows。

### 已知風險（開工時處理）

- 依賴三個還沒完成的施工關。第 7 施工關的 `messages` 表或第 9 施工關的 ticket／`inbox --after`／`operator` 請求跟這裡的理解不同時，改這頁的「你親自驗收」，不改它們。
- **checks 沒有 hook 保護**（P6）：checks worktree 跟 canonical 共用同一組 ref，agent 寫的測試或 `build.rs` 在裡面跑 `git update-ref refs/heads/main …` 不會被任何 hook 擋（agent 自己的 worktree 裡會被擋）。照第 3 施工關的威脅模型，這要刻意寫才會發生；保底的是 P7 的 CAS 會讓那次 merge 失敗、`merge-blocked` 或 `MergeFailed` 讓你看到。要更強就在 checks worktree 也裝 hook（P6 的替代方案）。
- `-c core.hooksPath=/dev/null` 也跳過專案自己的 `post-checkout` 等 hook（P5）：依賴 hook 準備環境的專案，checks worktree 裡少了那一步；要的話寫進 checks 指令。
- checks 每次都是新的 worktree（P6）：Rust 這類專案每次冷編譯，checks 會變慢；workflow 的指令可以自己設共用的快取目錄（例如 `CARGO_TARGET_DIR`）。
- 在持有者的 worktree 裡 rebase（P7）：這時 task 不在 work 關卡，但 agent 仍可能剛好在跑 git。worktree 不乾淨就當成衝突退回 work，不硬做。
- trailer 搜尋最多看 main 的 1000 個 first-parent commit（P7）：merge 之後 main 又前進超過 1000 個 commit、而且 DB 同時丟了 `merge_intent`，才會找不到；開工時量搜尋的耗時。
- core 要改四處（serde、`TaskStatus` 兩個新值、`outstanding_actions`、`Store` trait 的新方法）加上 binding 快照型別搬家：第 1 施工關的測試與探索器全部重跑；第 3 施工關的 shim 測試重跑。
- `tasks.status` 的 CHECK 改動在 SQLite 要重建整張表（migration 裡的標準 12 步）；附舊版 fixture。

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-core`、`-p agend-testkit`、`-p agend-shim`、`-p agend-daemon`、`-p agend` 單獨通過，包括：
  - `PipelineState` golden JSON、舊快照讀得回來、竄改的快照 `restore` 失敗不 panic（P2）
  - 探索器新不變量：`outstanding_actions` 等於送出、還沒被回應的要求（P2）
  - binding 快照型別搬家後 golden JSON 不變、shim 全部測試照過（P4）
  - STO 新規則「存檔與事件同成同敗」對 `FakeStore` 與真 store 都過，有 mutant（P1）
  - 分派：沒有空的 dev 時排隊、空出來後自動派出；沒有角色 → `no-role`，`team join` 後消失（P3、P10）
  - daemon 的 git 呼叫都有 `-c core.hooksPath=/dev/null`、都沒有 `AGEND_*`（P5）
  - checks：每次一個新目錄、跑完就刪；`--fail-checks-once` 回 work 再通過；逾時停掉整個 process group、餵 `CommandFinished{exit_code: None}`、task 回 work（P6）
  - merge：main 沒被 checkout、只被乾淨的 canonical checkout、被 dirty 的 canonical 或別的 worktree checkout 三種；空 branch 的 `done` 被拒；main 前進 → rebase、保留核准、重跑 checks；已 merge 的由 trailer 找回、`merge_intent` 當後備、手動 merge 的記成完成（`MergeCompleted`，註明 merged outside agend）（P7）
  - 「需要你」五種來源、`resolve_attention` 的 `note`；舊 attempt 的 `approval:` id 回 `unknown_attention`；重開機後 `waiting_since` 不變（P8）
  - P10 每個命令的 daemon 端；請示的一輪提問、回答、追問、結論；`remind` 跨重開機照樣送出；`task_cancel` 只收操作者
  - `delivery = inbox`：讀取不改狀態；結果命令帶同一個 ticket 時派工訊息變 `confirmed`（P11）
- [ ] 四次開機（P9）通過：`crates/agend/tests/` 裡真的 `agend daemon`，開機 1 被測試 `kill -9`、開機 3 在 failpoint 中止，開機 4 的檢查全部成立；反向檢查（每次新的 `AGEND_HOME`）在開機 2 失敗
- [ ] 兩個 failpoint 各一個測試：在那一點中止、重開後 task 照樣走到 done，main 上的 merge commit 剛好一個；另一個測試把 `merge_intent` 清掉（模擬斷電）後照樣由 trailer 找回
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept pipeline` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新；想改的共用文件列在 PR 裡由你決定
- [ ] 測試不留殘留：沒有 `g10-` 或測試 id 的 holder、`/tmp/g10-*` 已刪；kill 只對自己起的、大於 1 的 pid
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出；`<t-N>` 這類尖括號是會變的 id。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.x.y`（目前是 `agend 0.0.0`）。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

步驟 1–2 看同一次 `accept` 的輸出。步驟 3 起用同一個暫存 home，每個終端都要貼上步驟 3 印出的那行 `export AGEND_HOME=…`。

1. 跑 demo。

   **這步在驗什麼**：同一套流水線在真的 git、真的 daemon 上跑完每一段：順利、checks 失敗返工、reviewer 要求修改、main 前進、留下 WIP、hook、四次開機。錯了代表後面手動看到的都不可信。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo xtask accept pipeline
   ```

   應該看到：依序 `== happy`、`== checks-fail`、`== changes`、`== main-advanced`、`== wip`、`== hooks`、`== restart`，倒數第二行 `pipeline demo: all sections passed`，最後一行 `gate 10 (pipeline): checks passed`（確切輸出開工時細化）。

   - [ ] 通過

2. 四次開機，兩次在危險的地方被中止。

   **這步在驗什麼**：daemon 死在 checks 中間、死在「main 已經動了、還沒記下來」的那一刻，重開後 task 照樣走完，而且只 merge 一次（P2、P7、P9）。錯了的話 daemon 當掉一次，main 上就可能多一個重複的 merge，或 task 永遠卡住。

   操作：同一次輸出，找 `== restart`。應該看到（開工時細化）：

   ```text
   boot 1 daemon pid=<A> <t-1> work done; checks <t-1>/checks/1 running; killed -9 by test
   boot 2 daemon pid=<B> (idle) <t-1>: re-running checks <t-1>/checks/1 after restart; passed; review approved; waiting for approve
   boot 3 daemon pid=<C> approve <t-1>/approve/1; merge: main moved to <M>; aborted at failpoint after-main-moved
   boot 4 daemon pid=<D> <t-1>: merge found on main by trailer (<M>); not merged again
   check: merge commits for <t-1> on main = 1; branch gone; worktree gone; checks dirs gone; one dispatch per ticket
   negative check (new AGEND_HOME each boot): boot 2 failed: <t-1> unknown
   ```

   | 看什麼 | 意思 |
   |---|---|
   | boot 2 的 `re-running checks` | 死在 checks 中間的 task 被接起來，不是永遠卡住 |
   | boot 4 的 `not merged again` 與 `= 1` | merge 只做了一次 |
   | 最後一行 `boot 2 failed` | 反向檢查：換新的 home 就接不起來，證明這套檢查真的跨重啟 |

   - [ ] 通過

3. 你自己動手：建暫存 repo，前景啟動 daemon。

   **這步在驗什麼**：真的 `agend daemon` 讀到兩個 team、repo、三個假 agent，而且都起來了（P3、P11）。錯了的話後面沒有東西可驗。

   在第一個分頁：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-daemon --example pipeline_probe -- setup
   ```

   照它印的貼上 `export AGEND_HOME=…`，記下 `repo=` 那個路徑（下面寫成 `<repo>`），然後：

   ```bash
   agend daemon
   ```

   應該看到：`setup` 印出兩個 team 與三個 instance；daemon 最後一行 `agend daemon ready: instances=3 …`（確切字樣開工時細化）。daemon 留在前景。

   - [ ] 通過

4. 派一個 task，看它一關一關走，停在等你核准。

   **這步在驗什麼**：pipeline 依 workflow 的順序推進，agent 審查通過後，人工核准出現在「需要你」（P1、P8）。錯了的話 task 會跳關，或卡住而你不知道。

   第二個分頁（先跑開頭那段、貼上 `export AGEND_HOME=…`）：

   ```bash
   agend debug watch
   ```

   第三個分頁（同樣先設定）。操作者現在還不能直接開 task（見「待你決定」），所以用開發工具以 `g10` 裡的 agent 身分開：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-daemon --example pipeline_probe -- task g10 demo "hello"
   ```

   應該看到：印出 task id `<t-N>`；watch 依序出現 `<t-N>` 的 `work` → `submit` → `checks` → `review`，最後 `attention_required approval:<t-N>/approve/1 … actions: approve, request_changes`，然後停住（確切字樣開工時細化）。

   - [ ] 通過

5. 按核准，看它 merge。

   **這步在驗什麼**：只有你的核准能讓它 merge；merge 落在 main、是一個帶 trailer 的 merge commit（P7、P8）。錯了的話不是 merge 不了，就是沒核准也 merge 了。

   第三個分頁：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-client --example client_probe -- resolve approval:<t-N>/approve/1 approve
   git -C <repo> log -1 main
   ```

   應該看到：`resolved`；watch 出現 `attention_resolved` 與 `<t-N> merge` → `done`；`git log` 的標題是 `Merge agend/<t-N>/hello: hello`，最後一行 `Agend-Task: <t-N>`。

   - [ ] 通過

6. 確認清理。

   **這步在驗什麼**：task 結束後 branch、worktree、checks 目錄都不留，binding 快照也清掉（P4、P6）。錯了的話會像 v1 一樣越堆越多。

   ```bash
   git -C <repo> branch --list "agend/*"; find "$AGEND_HOME/worktrees" "$AGEND_HOME/checks" -mindepth 1
   cat "$AGEND_HOME/bindings/g10-dev.json"
   ```

   應該看到：前兩個指令什麼都不印（daemon 開機時一定建好這兩個目錄，所以 `find` 不會報錯）；快照裡沒有 `binding`，只有 instance 與 repo。

   - [ ] 通過

7. 故意弄壞：在 agent 的 worktree 裡改 main，然後取消那個 task。

   **這步在驗什麼**：daemon 綁定時真的裝了 hook，在 agent 的 worktree 裡誰都不能動 main（P4、第 3 施工關）；你取消 task 後它照樣被清掉（P10）。錯了的話 agent 手滑一次就能改掉你的 main，或卡住的 task 永遠佔著 agent。

   第三個分頁。`g10h` 那個 team 的假 agent 收到派工後什麼都不做，task 會停在 work 關卡，不影響 `g10`：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-daemon --example pipeline_probe -- task g10h demo "held"
   git -C "$AGEND_HOME/worktrees/<t-M>" update-ref refs/heads/main HEAD; echo "exit=$?"
   git -C <repo> log --oneline -1 main
   ~/.cargo/bin/cargo run -q -p agend-daemon --example pipeline_probe -- cancel <t-M>
   ```

   應該看到：第二行 `agend-shim: refused …` 並指出是 `(agend reference-transaction hook)`，`exit` 不是 0；main 還是步驟 5 那個 commit；`cancel` 之後 watch 出現 `<t-M> cancelled`，`worktrees/<t-M>` 不見了。

   - [ ] 通過

8. 故意弄壞：task 結束時 worktree 裡留著未 commit 的變更。

   **這步在驗什麼**：WIP 不會跟著 worktree 一起消失，而是先存成 patch（P4）。錯了的話 agent 沒 commit 的東西就永遠不見了。

   操作（開工時細化）：`pipeline_probe` 讓 `g10-dev` 這次帶 `--leave-wip`，再用 `task g10 demo "wip"` 派一個 task，照步驟 5 核准，等它 done。

   ```bash
   ls "$AGEND_HOME/archive/"
   ```

   應該看到：一個 `<t-K>-<秒>.patch`，打開看得到那個沒 commit 的檔；watch 裡那個 task 的 done 那行寫著 `archived WIP: archive/<t-K>-<秒>.patch`；worktree 照樣被刪。

   - [ ] 通過

9. 故意弄壞：checks 跑到一半時停掉 daemon。

   **這步在驗什麼**：daemon 停掉再起來，跑到一半的 checks 會在新目錄重跑，task 照樣走完，不會卡住也不會重複（P6、P9）。錯了的話每次重啟 daemon 都可能留下卡住的 task。

   操作：第三個分頁 `pipeline_probe -- task g10 slow "slow"`；watch 出現 `checks` 之後，在 daemon 的分頁按 Ctrl-C，再跑 `agend daemon`。

   應該看到：daemon 開機時 `<t-J>: re-running checks <t-J>/checks/1 after restart`；之後照步驟 5 核准，`git log --oneline main` 裡 `Merge agend/<t-J>/slow: slow` 只有一行。

   - [ ] 通過

10. 收尾。

    **這步在驗什麼**：什麼都不留（第 6 施工關的孤兒巡查照舊）。

    操作：各分頁 Ctrl-C，然後 `~/.cargo/bin/cargo run -q -p agend-daemon --example pipeline_probe -- teardown`（開工時細化）。

    應該看到：`pgrep -fl "agend holder g10-"` 什麼都不印；`/tmp/g10-…` 不見了。

    - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-26 第 3 輪 review REFUTED（2 MEDIUM、3 LOW）後修正：手動 merge 記成完成標出與 pipeline.md 不同、改寫成「最舊一個包含 head 的 commit」；開機逾時排除 merge 關卡；`command` 關卡寫 `on_timeout` 在建立時被拒；`no-role` 只在缺角色或只有作者時出現。
- 2026-09-26 第 2 輪 review REFUTED（2 HIGH、2 MEDIUM、4 LOW）後修正：手動 merge 改記 `MergeCompleted`（`StageFailed` 在 merge 送出中會被 core 拒絕）；checks 逾時改餵 `CommandFinished{exit_code: None}` 回 work，demo workflow 寫明 timeout；請示的 attention id 用 ask id；merge 中不能取消、`merge-blocked` 的出路；`delivery = inbox` 與 `--ff-only` 標出與架構頁不同；假 agent 固定 `claude` backend；步驟 6 改用 `find`；`task create --role` 與審查者排除作者。
- 2026-09-26 fresh review REFUTED（2 HIGH、7 MEDIUM、6 LOW）後修正：與第 9 施工關的分工寫進範圍（請示、task 類命令的 daemon 端、`agend team`／`workflow` 歸本關）；派工訊息與「需要你」用 ticket；`inbox` 讀取不算確認；驗收用 `pipeline_probe task` 開 task、`g10h` 放停住的 task 並可取消；空 branch 在 `done` 被拒、已 merge 靠 trailer 找回；demo 審查路徑一致；checks 每次新 worktree；D33 衝突標出；timeout、`waiting_since`、每日對帳、`worktree list` 檢查、rebase 措辭；failpoint 只留兩個；新增 P10（本關的命令）與「待你決定」。
- 2026-09-26 開工前提案 P1–P10 寫定（draft PR），待使用者確認；「你親自驗收」改成 10 步；狀態改為提案中。

## 下一步

```bash
cat docs/gates/gate-10-pipeline.md
~/.cargo/bin/cargo xtask accept pipeline
```
