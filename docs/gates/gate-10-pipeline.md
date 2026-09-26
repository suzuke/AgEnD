# 第 10 施工關：daemon：流水線（`pipeline`）

> **TL;DR**
> - daemon 用 core 的狀態機把一個 task 從派工推到 merge：建 worktree 並裝 hook、跑 checks、等核准、在本機 repo merge；daemon 被硬殺後接著做，不重複 merge。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：逐題看「開工前提案」P1–P10，每題打勾或寫下你要改的地方；確認後 merge 這份提案，等第 7–9 施工關完成再開工。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**提案中**（2026-09-26）：開工前提案 P1–P10 待你確認。依賴：第 6 施工關已 merge（#125）；第 7（送達、`messages` 表）、第 8（client 協定 1.1、「需要你」）、第 9（CLI 命令）施工關還在提案或實作中，本頁把需要它們提供的東西寫成「假設」（見 P10），不替它們設計。

## 範圍

- 驅動 core 狀態機：daemon 裡唯一的 pipeline 迴圈推進 task，每一步先 CAS 存檔再做事（P1）
- `PipelineState` 存成 task 資料列上的快照欄位，與 task 一起 CAS，不靠 replay events 重建（第 5 施工關 P4）；D32 延伸到 `PipelineState`，golden JSON 測試；core 加一個純函式列出「還在等結果的要求」；第 1 施工關的測試要重跑（P2）
- team、角色與分派：`teams` 表、instance 的 team／角色、core `policy::assign`；supervisor 與 task 的關係（P3）
- worktree 與 binding：先記錄再建立、固定命名空間、審查 worktree、結束時清理並把 WIP 存成 patch（P4）
- 從第 6 施工關移來（[gate-06 P9](gate-06-daemon-holder.md#p9hooks-與-binding-快照依賴規則)，使用者 2026-09-26 追認）：綁定／釋放 worktree 時安裝／移除 agend hook；daemon 為每個 agent 寫唯讀 binding 快照 `$AGEND_HOME/bindings/<instance>.json`（見 [GLOSSARY](../GLOSSARY.md)「binding 快照」）；`check-deps` 規則「`agend-daemon` 不能依賴 `agend-holder` 或 `agend-shim`」照舊成立（P4）
- git adapter：daemon 自己跑的 git 全部經 `Runner`、帶 timeout、不經 shim、不跑任何 hook（P5）
- runner（`command` 關卡）：每個 task 一個 checks worktree、環境白名單、timeout（P6）
- forge local：merge-tree + CAS `update-ref`、merge 前才處理 main 前進（D14）、merge 不會做兩次（P7）
- 「需要你」的新來源：人工核准、task 失敗、merge 被擋（P8）
- 開機對帳（reconcile）與四次開機的重啟契約（P9）
- 什麼是假的、什麼是真的；需要第 7–9 施工關提供的東西（P10）

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子。文件已定、這裡不重問的：狀態機是純函式 `step`、trait 在 core（D27）；`command` 關卡與 git 都經 `Runner`（D28）；forge 維持 3 個方法、GitHub CI 用 `command` 關卡接、forge local 沒有 change id（D29）；workflow 用 TOML 存 DB、task 固定建立時的版本（D19、D21）；人工核准 merge＝`approval(by = "human")`（D20）；一個 agent 一個 task、返工回持有者（D33）；main 前進的保留規則（D14）；worktree 只由 daemon 建、固定命名空間、先記錄再建立、WIP 存 patch、單一對帳（[pipeline](../architecture/pipeline.md#worktree-與-branch-生命週期)）；hook 設計與 binding 快照格式（第 3 施工關 T1、T21）；`PipelineState` 存成快照、不重播事件（第 5 施工關 P4）；daemon 起 holder、環境白名單、重起 3 次後 `failed`（第 6 施工關 P3、P6）；「需要你」的欄位、`resolve_attention` 只收操作者（第 8 施工關 P2、P5）；保留期限（D31）。

### P1：誰推進 task、怎麼存檔

- 問題：事件從很多地方來（agent 的命令、checks 結果、forge、逾時、你按的核准）。誰呼叫 `step`？存檔和做事哪個先？兩件事同時到怎麼辦？
- 建議：
  - daemon 裡**只有一個 pipeline 迴圈**（一個 tokio task，吃一條 channel）。所有事件都排進這條 channel，一次處理一個：讀 task 的狀態 → `step` → **先存檔** → 再把 actions 交給執行者（git、runner、forge、送達）。執行者做完，把結果當成新事件排回 channel。
  - 存檔是一個交易：task 列（含 P2 的快照欄位）用 CAS 寫入，同時附加 `task_events`。core 的 `Store` trait 加一個方法（例如 `advance_task(task_id, expected_version, 新狀態, 事件)`）；testkit 的 `FakeStore` 與 STO 契約一起補，新規則「存檔與事件同成同敗」。
  - `step` 回錯誤（`StaleResult`、`MergeInFlight`…）：什麼都不存；如果事件來自 agent 的命令，把錯誤碼回給它（`stale_result` 已在 client 協定）。
  - 只有一個寫入者，CAS 衝突照理不會發生；發生就是 bug：記 error log、重讀、放棄這個事件（不重試）。
  - core 的 `TaskStatus` 沒有「失敗」「取消」：加 `Failed`、`Cancelled` 兩個值（core 改動，第 1 施工關測試重跑；migration 重建 `tasks.status` 的 CHECK）。`agend status` 看的是 `tasks.status`，不必解析快照。
- 理由：一條迴圈就沒有「兩個事件同時改同一個 task」的問題，不用鎖；先存再做，daemon 在任何一刻死掉，DB 裡都是某個完整的狀態，沒做完的事由 P9 重做。
- 替代方案：每個 task 一個 actor（並行度高，但目前 task 數少，多一層排程）；先做事再存（死在中間時做過的事 DB 不知道，例如 merge 做了卻沒記錄）；outbox 表存待做的 actions（多一張表；P2 改用「從狀態算出來」）。
- 例子：假 agent 對同一個 attempt 送兩次 `done`：第一次 task 進 submit；第二次 `step` 回 `StaleResult`，DB 不變，agent 看到 `stale_result: this result is for work attempt 1, which already finished`（確切字樣開工時細化）。
- [ ] 使用者確認

### P2：`PipelineState` 快照、怎麼防偽造、重開機後要重做什麼

- 問題：第 5 施工關 P4 已定「存快照」。格式怎麼鎖？`PipelineState` 欄位私有、「不可偽造」，從 JSON 讀回來算不算偽造？daemon 重開後怎麼知道哪些事做到一半？
- 建議：
  - core 加 serde derive 到 `PipelineState` 與它的欄位型別（D32 延伸，要新決策編號）。**快照不含 workflow**：workflow 由 task 的 `workflow_id`＋`workflow_version`（D21）從 `workflows` 表讀，不存兩份。
  - 讀回只有一條路：`PipelineState::restore(快照, ValidatedWorkflow) -> Result`，檢查 task id、關卡位置在範圍內、每個關卡都有 attempt、紀錄裡的 stage id 都在 workflow 裡。壞掉的快照不會 panic：那個 task 標 `Failed`，出現在「需要你」（P8），其他 task 照常。
  - golden JSON 測試鎖格式；只准加欄位（比照 D26）。舊快照要讀得回來（每加欄位附一份舊 fixture，比照第 5 施工關 P5）。
  - core 加純函式 `outstanding_actions(&PipelineState) -> Vec<PipelineAction>`：目前關卡「送出了、還沒收到結果」的要求（例如 checks 的 `RunCommand`、merge 送出中的 `Merge`）。探索器加一條不變量：每一步之後，它等於「這一步送出、還沒被回應」的那些要求。P9 開機時用它重做，不另存 actions。
  - 新 migration（開工時的下一個空號；第 8 施工關用 `0003`，第 7 施工關預定 `0004`）：`tasks` 加 `pipeline`（JSON 快照）、`stage_deadline_unix_ms`（逾時，P9）、`merge_intent`（P7）；P3 的 `teams` 表與 instance 欄位、P4 的 `bindings` 表、P10 的 `delivery` 欄位也在同一個 migration，每張新表都列進第 5 施工關 P8 的保留規則表。
- 理由：重播事件的問題第 5 施工關已經講過；快照加一道 `restore` 檢查，外面仍然造不出不合規則的狀態（竄改測試照跑）。「還在等什麼」從狀態算，不必多一張會跟狀態對不上的表。
- 替代方案：快照連 workflow 一起存（兩份真相）；不檢查直接 `Deserialize`（等於開放偽造）；outbox 表存 actions（多一張表、兩份真相）。
- 例子：task 在 checks 第 2 次時 daemon 被硬殺；重開後 `outstanding_actions` 回 `RunCommand{checks, attempt 2, head a1b2…}`，daemon log `t-7: re-running checks attempt 2 after restart`。
- [ ] 使用者確認

### P3：team、角色、分派；supervisor 跟 task 的關係

- 問題：目前只有 `general`（沒有 repo）。task 要有 repo 才能走 `code`。instance 沒有角色，怎麼分派？instance 死了、被刪了，手上的 task 怎麼辦？
- 建議：
  - 同一個 migration 加 `teams` 表（`id`、`repo`＝canonical checkout 的絕對路徑、可空、`default_workflow`），保留期限「永久」；`instances` 加 `team_id`（預設 `general`）、`role`。
  - 分派用 core 的 `policy::assign::choose`：候選人＝同 team、同角色、狀態 `running`、手上沒有未結束 task 的 instance；人數上限先等於現有人數，所以**不開臨時 instance**（`SpawnEphemeral` 不會出現）。沒有人可用 → task 留在原地排隊，下一次有 instance 空出或新增時再試。team 裡沒有這個角色 → 同樣排隊，另外出現一個「需要你」（P8）。
  - task 持有者（D33）記在 `tasks.assignee`，到 task 結束才清掉；審查者持有的是那次審查，核准或要求修改後就空出來。
  - supervisor（第 6 施工關）不動：instance `failed` 時它手上的 task **停著等**；第 8 施工關那個「需要你」項目的 `unblocks` 從 0 變成 1。你按 `retry` 讓 agent 回來後，它的 binding 還在，照原來的工作繼續。
  - 本關**不做改派**：持有者被刪、額度用盡、逾時動作 `reassign` 都在之後的施工關。所以本關拒絕刪除手上有未結束 task 的 instance（訊息指出 task），也拒絕建立用到 `on_timeout = reassign` 的 task（訊息寫這個動作還不支援）。
- 理由：一條最短的路：一個 team、一個 dev、一個 reviewer 就能走完 `code`。改派要交接 branch 與審查意見（D33），牽涉最多，等有真 backend 額度資料時再做。
- 替代方案：instance 的 team／角色寫在設定檔（D8：持久狀態只在 DB）；本關就做角色範本與臨時 instance（要選 backend、模型，第 12 施工關有真 backend 後比較驗得到）；instance 死了立刻改派（會和 `retry` 打架，也違反 D33「返工回持有者」）。
- 例子：team `g10` 有 `g10-dev`（dev）與 `g10-rev`（reviewer）。建第一個 task → `g10-dev` 接；建第二個 → log `t-2: queued (no free dev in g10)`；第一個 merge 後，第二個自動派給 `g10-dev`。
- [ ] 使用者確認

### P4：worktree、binding、hook、binding 快照

- 問題：worktree 什麼時候建、建在哪？hook 什麼時候裝、怎麼裝（daemon 不能依賴 `agend-shim`）？binding 快照誰寫、什麼時候寫？審查的人在哪看程式？task 結束時留著的未 commit 修改怎麼辦？
- 建議：
  - 綁定（work 關卡第一次派給持有者時）：
    1. DB 的 `bindings` 表寫一列 `pending`（instance、task、kind、worktree、branch）。
    2. 在 canonical checkout 跑 `git worktree add -b agend/<task>/<slug> $AGEND_HOME/worktrees/<task>/ <main 的 SHA>`。
    3. 裝 hook：daemon 跑 `agend hooks install <worktree>`（`agend` 的內部子命令，呼叫 `agend_shim::install_hooks`；不在 `--help` 列出）。
    4. 寫 binding 快照（先寫暫存檔再 rename、0444）。
    5. `bindings` 標 `ready`，然後才送派工訊息。
  - 每一步都可重做：死在中間，P9 從 `pending` 接著做；worktree 已存在就檢查它是不是這個 branch。
  - binding 快照的型別從 `agend_shim::binding::Snapshot` 搬到 core（第 3 施工關 T1 已建議）：路徑改成 `String`（core 是 no_std），shim 與 daemon 共用同一個型別；golden JSON 測試證明格式沒變。這也是 D32 的延伸，跟 P2 一起給新決策編號。`bindings` 表的列在釋放時刪除（保留期限：隨 binding）。
  - 審查：reviewer 拿到 detached 的審查 worktree `$AGEND_HOME/worktrees/<task>-review/`（在審的 head），binding kind `review`，一樣裝 hook（審查 binding 不能寫任何 branch）。核准或要求修改後就拆掉。
  - 釋放（task done、失敗、取消；或審查結束）：
    1. binding 快照先改成沒有 binding（shim 從這一刻起拒絕寫入）。
    2. 有未 commit 的修改或未追蹤檔 → 存成 `archive/<task>-<unix 秒>.patch`；task 沒 merge（失敗、取消）時，branch 上的 commit 也用 `format-patch` 存進同一個檔。保留 30 天（D31 的 WIP patch）。
    3. `agend hooks uninstall <worktree>` → `git worktree remove --force` → `git branch -D`。
    4. 刪掉 `bindings` 那一列；patch 路徑記在 task 事件，`agend status` 與 TUI 看得到。
- 理由：hook 是 protected ref 的硬保證（第 3 施工關威脅模型），一定要在 agent 拿到 worktree **之前**裝好；經子命令呼叫，daemon 就不必連結 shim（第 6 施工關 P9 的依賴規則）。一個型別兩邊用，格式不會漂移（#1493）。
- 替代方案：daemon 直接呼叫 `install_hooks`（違反依賴規則）；hook 在 agent 第一次跑 git 時由 shim 自己裝（agent 可以繞過 shim）；失敗或取消的 branch 留著不刪（v1 的 137 個 branch）；審查者直接讀持有者的 worktree（持有者可能正在改，審的不是那個 head）。
- 例子：task `t-3` 派給 `g10-dev` → `worktrees/t-3/` 出現、branch `agend/t-3/hello`、`bindings/g10-dev.json` 有 `{"kind":"work","task_id":"t-3",…}`；在那個 worktree 跑 `git update-ref refs/heads/main HEAD` → `agend-shim: refused … (agend reference-transaction hook)`。task merge 後三樣都不見，`bindings/g10-dev.json` 只剩 instance 與 repo。
- [ ] 使用者確認

### P5：daemon 自己跑的 git

- 問題：daemon 也要跑 git（建 worktree、算 patch-id、merge、看 head）。會不會被 shim 或 hook 擋？會不會觸發專案自己的 hook？卡住怎麼辦？daemon 會不會在 agent 的 worktree 裡寫東西？
- 建議：
  - daemon 開機時找一次真的 git：`PATH` 上第一個**不在** `$AGEND_HOME/bin` 的 `git`，記住絕對路徑；版本低於 2.38（`merge-tree --write-tree` 需要）就不處理有 repo 的 team，log 指出原因（`agend doctor` 由第 9 施工關檢查）。
  - 每個 git 呼叫：`env_clear` 後只給 `HOME`、`PATH`、`LANG=C`、`GIT_TERMINAL_PROMPT=0`（沒有 `AGEND_*`、沒有 daemon 的 `GIT_*`）；一律加 `-c core.hooksPath=/dev/null`（不跑 agend hook，也不跑專案的 hook）；經 `Runner`，timeout 60 秒。
  - 在 canonical checkout 跑：`worktree add/remove`、`branch -D`、`rev-parse`、`merge-base`、`diff`／`patch-id`、`merge-tree`、`commit-tree`、`update-ref`。
  - 在 agent worktree 裡只做兩件事：讀（`status`、`diff`，給 P4 的 WIP patch 用），以及 P7 的 rebase（只在 task 不在 work 關卡、worktree 乾淨時）。**daemon 從不在 agent worktree 裡 commit。**
  - 什麼時候看 branch 的 head：沒有輪詢。只在「收到任何結果事件時」與「送出 `Merge` 前」讀一次；和狀態裡的 head 不同就先餵 `CommitCreated`（附新的 patch-id），再處理原本的事件。
- 理由：第 3 施工關已定「可信的呼叫者用 `-c core.hooksPath=/dev/null`」；daemon 沒有 agent 身分，不關 hook 會被自己的 hook 擋。專案 hook（例如 husky 的 `post-checkout` 跑 `npm install`）在 `worktree add` 時跑，會又慢又不可預期。每個外部指令都有 timeout（V1-LESSONS #10）。
- 替代方案：用 `AGEND_SHIM_BYPASS=1` 跑 PATH 上的 git（只跳 shim，hook 照樣擋）；保留專案 hook（`worktree add` 可能跑幾分鐘）；定時輪詢每個 branch 的 head（多數時間白跑）。
- 例子：agent `done` 之後又 commit 了一次才被 checks 跑到：checks 結果回來時 daemon 發現 head 變了 → log `t-3: head moved a1b2… -> c3d4…; checks run again`，舊的結果不算。
- [ ] 使用者確認

### P6：`command` 關卡（runner）

- 問題：checks 在哪跑？用什麼環境？能不能碰到 agent 的東西？跑多久算逾時？輸出存哪？daemon 被硬殺時正在跑的 check 怎麼辦？
- 建議：
  - 每個 task 一個 checks worktree：`$AGEND_HOME/checks/<task>/`，detached。每次跑之前 `checkout --detach --force <head>` 再 `clean -ffd`（**保留** ignored 檔，例如 `target/`，同一個 task 的第二次 checks 可以增量編譯）；task 結束時刪掉。不裝 hook、沒有 binding。
  - 指令：`sh -c <已展開的指令>`，自己一個 process group（RUN-8），stdin 是 `/dev/null`。環境：第 6 施工關的 agent 白名單，但**沒有** `AGEND_*`，`PATH` 也拿掉 `$AGEND_HOME/bin`（checks 用真的 git，不是 agent）。
  - timeout：關卡的 `timeout_ms`，沒寫就 30 分鐘；逾時 → 停掉整個 process group → 餵 `StageTimedOut`，照關卡的逾時動作走。
  - 輸出：完整 stdout／stderr 存 `logs/checks/<task>/<stage>-<attempt>.log`（每個檔最多 10 MiB，超過截斷並註明）；task 事件只放最後 20 行。保留 14 天（跟事件一樣，列進第 5 施工關 P8 的規則表）。
  - 同時最多跑 1 個 check，其他排隊（log 寫出在等誰）。
  - 不做沙箱：同一個使用者、可以上網。checks 是你自己寫進 workflow 的指令，不是 agent 給的。
  - daemon 被硬殺時正在跑的 check 會變孤兒、跑到自己結束，結果沒人收；重開後 P9 對同一個 attempt 重跑（先 `clean`）。孤兒還在寫檔時可能跟新的撞在同一個目錄：見「已知風險」。
- 理由：跟 agent 的 worktree 分開（`runner.rs` 的 Must NOT）；每個 task 一個、重複使用，比每次新建快很多（Rust 專案每次冷編譯要好幾分鐘）；一次一個最簡單，也不會讓兩個 `cargo test` 搶 CPU 互相逾時。
- 替代方案：每次新建一個 worktree（最乾淨，但每次冷編譯）；在 agent 的 worktree 跑（agent 可能正在改）；同時跑多個（要設上限與排序）；容器或 `sandbox-exec`（兩個平台不同，本關先不做）。
- 例子：checks 是 `test -f hello.txt`，第一次 agent 忘了加檔 → exit 1 → `t-3: checks attempt 1 failed (exit 1); back to work (g10-dev)`；agent 補上再 `done` → `checks attempt 2 passed`。
- [ ] 使用者確認

### P7：forge local 與 merge

- 問題：merge 怎麼做才不動到你的工作目錄？canonical checkout 正好停在 main 怎麼辦？main 在這期間前進了（別的 task 先 merge、或你自己 commit）怎麼辦？怎麼保證 daemon 死在 merge 中間也不會 merge 兩次？
- 建議：
  - merge 步驟（在 canonical checkout）：
    1. `git merge-tree --write-tree <main> <head>`；有衝突 → 回 `MergeFailed`。
    2. `git commit-tree`：父 commit 是 main 與 head（永遠是一個 merge commit，不 fast-forward）；訊息 `Merge agend/<task>/<slug>: <title>`，加 trailer `Agend-Task: <task>`。
    3. 把這個 commit 的 SHA 存進 `tasks.merge_intent`（CAS），**再**移動 main。
    4. 移動 main：canonical checkout 不在 main → `git update-ref refs/heads/main <新> <舊>`（CAS）。停在 main 而且工作目錄乾淨 → `git merge --ff-only <新>`（main 被別人動過就會失敗，等同 CAS，工作目錄也一起更新）。停在 main 但有未 commit 的修改 → **不 merge**，出現「需要你」`merge-blocked`（P8）。
  - main 前進（D14）**只在送出 merge 前檢查一次**：head 不包含目前的 main → daemon 在持有者的 worktree 裡 rebase（task 此時不在 work 關卡；worktree 不乾淨就當成衝突）→ 餵 `MainAdvanced{rebased_head, patch_id, conflict}` 再餵 `MergeFailed`（core 已規定送出中的 head 變更要等 `MergeFailed` 才套用）。patch-id 相同 → 保留核准、重跑 checks；不同或衝突 → 回 work。所以 checks 一定是在「已包含最新 main 的 head」上通過的，merge 出來的樹就是測過的樹。
  - merge 不會做兩次：送 `Merge` 前如果 head 已經是 main 的祖先（上次 merge 其實成功了）→ 直接回 `MergeCompleted`，merge commit 取 `merge_intent`。這只對 forge local 成立（它從不 squash）；GitHub 在第 12 施工關另外處理。
  - done 以 DB 裡的 `merge_commit` 為準；不從 git 推論其他 task（V1-LESSONS #6）。
- 理由：`merge-tree` 不碰任何工作目錄（pipeline.md 已定）；先存 `merge_intent` 再動 main，死在任何一步都能判斷 merge 到底有沒有成功。只在 merge 前處理 main 前進，只有一個觸發點，也不必在 agent 正在改的時候動它的 worktree。
- 替代方案：main 一前進就 rebase 所有進行中的 task（要處理 agent 正在改的 worktree）；fast-forward merge（main 上看不出哪個 task 什麼時候 merge）；canonical 停在 main 又不乾淨時照樣 `update-ref`（你的工作目錄會變成一堆反向修改）；不存 `merge_intent`、開機時在 main 的歷史裡找 trailer（要掃歷史）。
- 例子：t-3、t-4 都到了 merge。t-3 先 merge；t-4 送 merge 前發現 head 不含新的 main → rebase 無衝突、patch-id 相同 → log `t-4: main advanced; rebased, approval kept, checks run again` → checks 通過 → merge。`git log --oneline main` 最上面兩個是 `Merge agend/t-4/…`、`Merge agend/t-3/…`。
- [ ] 使用者確認

### P8：「需要你」的新來源與協定補充

- 問題：人工核准、task 失敗、merge 被擋，你怎麼知道、怎麼處理？請示（`agend ask`）歸誰？
- 建議：沿用第 8 施工關的 `attention_required`／`resolve_attention`／`attention_resolved`，本關加四種來源（跟 `instance-failed` 一樣從 DB 算，不另存表）：

  | `attention_id` | 什麼時候出現 | `actions` | 按了之後 |
  |---|---|---|---|
  | `approval:<task>:<stage>:<attempt>` | `approval(by = "human")` 關卡等你 | `approve`、`request_changes` | `ApprovalGranted`（綁 head 的關卡綁送出要求時的 head）或 `ChangesRequested`（理由必填） |
  | `task-failed:<task>` | task 失敗（含 P2 快照壞掉） | `acknowledge` | 從清單拿掉；task 保持 `Failed`，WIP 已存 patch |
  | `merge-blocked:<task>` | canonical 停在 main 又有未 commit 的修改（P7） | `retry` | 再試一次 merge |
  | `no-role:<team>:<role>` | 派工時 team 裡沒有這個角色（P3） | 無 | 你加了這個角色的 instance 後自動消失 |

  - `request_changes` 要帶理由：`resolve_attention` 加一個選填欄位 `note`。這是協定的新增，client 協定升到 **1.2**（比照第 8 施工關 P3「一個施工關的新增算一次 minor」）；`note` 缺少時回 `invalid_request`。
  - `approval:` 的 id 帶 attempt：head 變了、開了新的 attempt，舊項目自動消失、新項目出現；拿舊 id 按會回 `unknown_attention`，不會核准到新的 head。
  - `unblocks` 一律 1（一個 task）；`waiting_since` 是 task 進入那個狀態的時間（從 task 事件取，重開機不重算）；`task_id`、`instance_id`（持有者）照填。脈絡摘要（D37）本關只填最短的：目標＝task 標題、在問什麼＝「核准 `<branch>` 的 `<head 前 7 碼>`」、之後＝「merge 進 main」；完整內容是第 11 施工關的事。
  - **與 D18 字面不同，請明確決定**：D18 寫「需要不存在的角色時轉成 ask」。這裡改成 `no-role` 項目，因為 ask 是 agent 與你的對話（D35），而這件事的解法是「加一個 instance」，不是回答問題。
  - 請示（`agend ask`、`answer_ask`）**假設由第 9 施工關做**（它是 D17 的 agent 命令之一）；本關只讓 pipeline 用得到的東西出現在「需要你」。
- 理由：同一套「需要你」機制，TUI（第 11 施工關）與 Telegram（第 12 施工關）不必各做一次核准；只多一個選填欄位。
- 替代方案：人工核准做成 ask（理由可以用自由文字回答，但要先有請示的儲存與對話，本關不一定有）；操作者命令 `agend task approve`（第 9 施工關的命令，TUI 還是要另一條路）；`task-failed` 不放進「需要你」（失敗的 task 容易沒人發現）。
- 例子：`demo` workflow 走到人工核准 → `agend debug watch` 印 `attention_required approval:t-3:review:1 (unblocks 1; if ignored: t-3 waits for you) actions: approve, request_changes` → 你按 `approve` → `attention_resolved …` → merge。
- [ ] 使用者確認

### P9：開機對帳與四次開機的重啟契約

- 問題：daemon 被 `kill -9`、或斷電丟了最後幾筆寫入（第 5 施工關：macOS 沒開 `fullfsync`），重開後怎麼接著做？怎麼證明真的接得上？
- 建議：
  - 開機順序：第 6 施工關的開機計畫（instance）做完後、第 8 施工關 bind socket **之前**，跑一次對帳；之後每小時的 housekeeping 也跑一次（只做下面第 2、3 項）。對帳以 DB 為準：
    1. 每個未結束的 task：`restore` 快照（P2）；失敗 → `Failed`＋「需要你」。
    2. binding：`pending` 的接著建（P4）；`ready` 但 worktree 不見了 → 從 branch 重建；branch 也不見 → task `Failed`。重寫**全部** binding 快照檔（檔案只是 DB 的投影），刪掉 DB 沒有的 instance 的快照檔。
    3. 命名空間裡的孤兒（`agend/<task>/…` branch、`worktrees/<task>*`、`checks/<task>/`，DB 沒有這個 task 或它已結束）→ 照 P4 的釋放流程（先存 WIP patch 再刪）。命名空間外一律不碰。
    4. 重做 `outstanding_actions`（P2）：派工訊息用固定的訊息 id `<task>:<stage>:<attempt>` 重送（第 7 施工關的送達以 id 冪等，agent 不會收到兩次）；checks 重跑同一個 attempt；merge 送出中照 P7（head 已在 main 裡就算完成）。
    5. 逾時：從 `stage_deadline_unix_ms` 重排；已經過了就立刻餵 `StageTimedOut`。
  - 斷電丟了最後幾筆：DB 回到較舊的狀態，對帳就從那個狀態接著做；已經做過的外部動作（merge、建 worktree）都是冪等的，重做只會發現「已經做過」。
  - 重啟契約（比照 CONTRACTS 的四次開機與第 6 施工關 P4，跨真的 process、真的 `agend daemon`）：
    - 開機 1（做事）：建 task，假 agent 做完、`done`；checks 跑到一半時測試 `kill -9` daemon。
    - 開機 2（閒置）：測試不做事；對帳重跑 checks，task 走到審查、假 reviewer 核准。
    - 開機 3（做事）：在「main 已經移動、`merge_commit` 還沒存」這一刻 `kill -9`（用 failpoint，見下）。
    - 開機 4（檢查）：task `done`；main 上這個 task 的 merge commit **剛好一個**；branch、worktree、checks 目錄都不見；每個 (stage, attempt) 的派工訊息只送過一次。
    - 反向檢查：每次開機用新的 `AGEND_HOME` → 開機 2 就失敗。
  - failpoint：`AGEND_FAILPOINT=<名稱>` 讓 daemon 在那一點 `abort()`，**只在 debug build 編進去**。名稱：`after-cas-before-actions`、`after-worktree-add`、`after-merge-intent`、`after-main-moved`。
- 理由：單一對帳取代 v1 約 9 個清理機制（V1-LESSONS #5）；靠 kill 的時機碰運氣測不到「剛好在 merge 中間」，failpoint 讓每一刀都落在指定的地方。
- 替代方案：開機時不對帳、等事件自然發生（死在 checks 中的 task 永遠卡住）；失敗點用隨機 kill 多跑幾次（不穩、測不到窄窗口）；failpoint 也編進 release（正式 binary 多一條可以讓它自己 abort 的路）。
- 例子：`boot 3 daemon pid=<C> t-1 merge: main moved to 9f8e…; killed at failpoint after-main-moved`、`boot 4 t-1: head already in main; merge completed (9f8e…) — not merged again`。
- [ ] 使用者確認

### P10：什麼是假的、什麼是真的；需要第 7–9 施工關提供的東西

- 問題：「假 driver」指什麼？沒有真 backend，誰來 commit、誰來核准？CLI 命令還沒有時怎麼驗？
- 建議：
  - **真的**：git（系統的 git，≥ 2.38）、暫存 repo（`/tmp/g10-…`）、forge local、runner（`sh`）、SQLite、holder、shim 與 hook、`agend daemon` binary、client 協定。
  - **假的**：agent 的腦袋。testkit 加一個假 agent 程式 `fake-worker`（放在 holder 裡跑，跟第 6 施工關的計數器一樣）：每秒 `agend inbox` 一次，收到派工就在 worktree 寫檔、經 shim `git commit`、`agend done`；收到審查就 `agend review approve`。旗標：`--fail-checks-once`（第一次故意不加檔案）、`--leave-wip`（done 前留一個未 commit 的檔）、`--changes-once`（reviewer 第一次要求修改）、`--hold`（收到派工後什麼都不做，task 停在 work 關卡）。`pipeline_probe` 另有一個慢 checks 的 workflow（checks 約 20 秒），給「停掉 daemon」那一步用。
  - 送達：instance 加一欄 `delivery`（`push` 預設／`inbox`）。`inbox` 的 instance daemon 不主動推，訊息留在第 7 施工關的 `messages` 表，agent 用 `agend inbox` 自己拿（拿到就算 `confirmed`）。假 agent 用 `inbox`；等第 12 施工關有 claude driver，真的 claude 仍是 `push`，不受影響。
  - pipeline 迴圈的單元測試用 testkit 的假實作（`FakeStore`、`FakeForge`、`FakeRunner`、`FakeDriver`、`FakeClock`）；整合測試與 demo 用上面「真的」那一組。
  - 開發用 example `pipeline_probe`（比照 `daemon_probe`，只在 daemon 停著時用）：`setup` 一次建好暫存 repo、home、team `g10`、`demo` workflow、`g10-dev`／`g10-rev` 兩個假 agent，印出 `export AGEND_HOME=…` 與 repo 路徑。
  - **對第 9 施工關的假設**（不在這裡設計語法）：`agend task create`（操作者也能用）、`agend status`、`agend inbox`、`agend done`、`agend review approve|changes`（帶派工訊息裡的 stage 與 attempt）。操作者的 team／instance／workflow 設定命令若第 9 施工關沒有，本關只用 `pipeline_probe`，不自己加正式命令。第 8 施工關的 `agend debug watch` 與 `client_probe resolve` 拿來看進度與按核准。
  - `check-deps`：不加新規則（`agend-daemon` 不能依賴 `agend-shim`／`agend-holder` 已有；hook 經子命令）。
- 理由：要驗的是 daemon 的流水線，不是 LLM；假 agent 走的是真的 CLI、真的 shim、真的 hook，只有「決定寫什麼」是假的。`delivery = inbox` 是一欄一個分支，而且不綁任何 backend，第 12 施工關之後照樣能用。
- 替代方案：用第 7 施工關的 codex driver ＋假 app-server，讓假 app-server 收到訊息時執行腳本（要改假 app-server，而且綁 codex）；測試直接在 process 裡扮 agent、不經 CLI（驗不到 `agend done` 的身分與 shim）；加一個 `fake` backend（`Backend` 只能是三個，要改決策）。
- 例子：`pipeline_probe setup` 印 `repo=/tmp/g10-1234/repo`、`export AGEND_HOME=/tmp/g10-1234/home`、`team g10: dev g10-dev (fake-worker), reviewer g10-rev (fake-worker)`、`workflow demo v1: work -> submit -> checks -> review(human) -> merge`。
- [ ] 使用者確認

### 本關不做（明確列出）

- fanout／`epic`（子 task）、`depends_on`、supersede、reopen：用到的 workflow 或操作在本關回錯誤並指出還不支援。
- 改派（持有者被刪、額度用盡、逾時動作 `reassign`）、臨時 instance、角色範本（P3）。
- forge github、GitHub CI（第 12 施工關）。
- 卡住偵測、usage limit、依緊急程度自動選忙碌等級（第 7 施工關 P9 說要移來本關：本關一律用 `Queue`，其餘建議移到第 12 施工關，有真 backend 資料時再做）。
- codex approval 轉給人回答（第 7 施工關 P9 說移到第 10、11 施工關：建議整個給第 11 施工關）。
- 派工時的檔案衝突警告（`policy::conflict`）：正確性由 P7 的 merge 前 rebase 保證，警告之後再加。
- 請示的儲存與對話（假設第 9 施工關做，P8）。
- 逾時動作「通知」只記 log 與 task 事件；Telegram 通知在第 12 施工關。
- checks 的沙箱與網路限制（P6）；Windows。

### 已知風險（開工時處理）

- 依賴三個還沒完成的施工關（P10）。第 9 施工關的命令語法或第 7 施工關的 `messages` 表跟這裡的假設不同時，改這頁的「你親自驗收」，不改它們。
- 孤兒 check（P6）：daemon 被硬殺後，舊的 check 可能還在同一個 `checks/<task>/` 寫檔，跟重跑的撞在一起。開工時量：若會發生，改成每個 attempt 一個子目錄（`checks/<task>/<attempt>/`），代價是 ignored 檔不能共用。
- `-c core.hooksPath=/dev/null` 也跳過專案自己的 `post-checkout` 等 hook（P5）：依賴 hook 準備環境的專案，checks worktree 裡少了那一步；要的話寫進 checks 指令。
- 在持有者的 worktree 裡 rebase（P7）：這時 task 不在 work 關卡，但 agent 仍可能剛好在跑 git。worktree 不乾淨就當成衝突退回 work，不硬做。
- `git merge --ff-only`（canonical 停在 main 時）會跑你的 repo 的 hook 嗎：加了 `-c core.hooksPath=/dev/null` 就不會；開工時實測 `post-merge` 沒跑。
- core 要改四處（serde、`TaskStatus` 兩個新值、`outstanding_actions`、`Store` trait 的新方法）加上 binding 快照型別搬家：第 1 施工關的測試與探索器全部重跑；第 3 施工關的 shim 測試重跑。
- `tasks.status` 的 CHECK 改動在 SQLite 要重建整張表（migration 裡的標準 12 步）；附舊版 fixture。
- client 協定 1.2 以第 8 施工關的 1.1 已 merge 為前提；若第 8 施工關還沒 merge，`note` 併進它的那一次 minor。

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-core`、`-p agend-testkit`、`-p agend-shim`、`-p agend-daemon`、`-p agend` 單獨通過，包括：
  - `PipelineState` golden JSON、舊快照讀得回來、竄改的快照 `restore` 失敗不 panic（P2）
  - 探索器新不變量：`outstanding_actions` 等於送出、還沒被回應的要求（P2）
  - binding 快照型別搬家後 golden JSON 不變、shim 全部測試照過（P4）
  - STO 新規則「存檔與事件同成同敗」對 `FakeStore` 與真 store 都過，有 mutant（P1）
  - 分派：沒有空的 dev 時排隊、空出來後自動派出；沒有角色 → `no-role`（P3）
  - daemon 的 git 呼叫都有 `-c core.hooksPath=/dev/null`、都沒有 `AGEND_*`（P5）
  - checks：`--fail-checks-once` 回 work 再通過；逾時停掉整個 process group（P6）
  - merge：canonical 不在 main、在 main 且乾淨、在 main 但不乾淨三種；main 前進 → rebase、保留核准、重跑 checks；head 已在 main → 不再 merge（P7）
  - 「需要你」四種來源與 `resolve_attention` 的 `note`；舊 attempt 的 `approval:` id 回 `unknown_attention`（P8）
- [ ] 四次開機（P9）通過：`crates/agend/tests/` 裡真的 `agend daemon`、兩次 failpoint `kill`，開機 4 的檢查全部成立；反向檢查（每次新的 `AGEND_HOME`）在開機 2 失敗
- [ ] 每個 failpoint 各一個測試：在那一點中止、重開後 task 照樣走到 done，main 上的 merge commit 剛好一個
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

   **這步在驗什麼**：同一套流水線在真的 git、真的 daemon 上跑完每一段：順利、checks 失敗返工、reviewer 要求修改、main 前進、留下 WIP、四次開機。錯了代表後面手動看到的都不可信。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo xtask accept pipeline
   ```

   應該看到：依序 `== happy`、`== checks-fail`、`== changes`、`== main-advanced`、`== wip`、`== hooks`、`== restart`，倒數第二行 `pipeline demo: all sections passed`，最後一行 `gate 10 (pipeline): checks passed`（確切輸出開工時細化）。

   - [ ] 通過

2. 四次開機，兩次在最危險的地方被硬殺。

   **這步在驗什麼**：daemon 死在 checks 中間、死在「main 已經動了、還沒記下來」的那一刻，重開後 task 照樣走完，而且只 merge 一次（P2、P7、P9）。錯了的話 daemon 當掉一次，main 上就可能多一個重複的 merge，或 task 永遠卡住。

   操作：同一次輸出，找 `== restart`。應該看到（開工時細化）：

   ```text
   boot 1 daemon pid=<A> <t-1> work done; checks attempt 1 running; killed -9 by test
   boot 2 daemon pid=<B> (idle) <t-1>: re-running checks attempt 1 after restart; passed; review approved
   boot 3 daemon pid=<C> <t-1> merge: main moved to <M>; killed at failpoint after-main-moved
   boot 4 daemon pid=<D> <t-1>: head already in main; merge completed (<M>) — not merged again
   check: merge commits for <t-1> on main = 1; branch gone; worktree gone; assignments sent once per attempt
   negative check (new AGEND_HOME each boot): boot 2 failed: <t-1> unknown
   ```

   | 看什麼 | 意思 |
   |---|---|
   | boot 2 的 `re-running checks` | 死在 checks 中間的 task 被接起來，不是永遠卡住 |
   | boot 4 的 `not merged again` 與 `= 1` | merge 只做了一次 |
   | 最後一行 `boot 2 failed` | 反向檢查：換新的 home 就接不起來，證明這套檢查真的跨重啟 |

   - [ ] 通過

3. 你自己動手：建暫存 repo，前景啟動 daemon。

   **這步在驗什麼**：真的 `agend daemon` 讀到 team、repo、兩個假 agent，而且都起來了（P3、P10）。錯了的話後面沒有東西可驗。

   在第一個分頁：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-daemon --example pipeline_probe -- setup
   ```

   照它印的貼上 `export AGEND_HOME=…`，記下 `repo=` 那個路徑（下面寫成 `<repo>`），然後：

   ```bash
   agend daemon
   ```

   應該看到：`team g10: dev g10-dev (fake-worker), reviewer g10-rev (fake-worker)`；daemon 最後一行 `agend daemon ready: instances=2 … tasks=0`（確切字樣開工時細化）。daemon 留在前景。

   - [ ] 通過

4. 派一個 task，看它一關一關走，停在等你核准。

   **這步在驗什麼**：pipeline 依 workflow 的順序推進，人工核准出現在「需要你」（P1、P8）。錯了的話 task 會跳關，或卡住而你不知道。

   第二個分頁（先跑開頭那段、貼上 `export AGEND_HOME=…`）：

   ```bash
   agend debug watch
   ```

   第三個分頁（同樣先設定）：

   ```bash
   agend task create --team g10 --workflow demo "hello"
   ```

   應該看到：第三個分頁印 task id `<t-N>`；watch 依序出現 `<t-N>` 的 `work` → `submit` → `checks` → `review`，最後 `attention_required approval:<t-N>:review:1 … actions: approve, request_changes`，然後停住（確切字樣開工時細化；`task create` 的語法照第 9 施工關）。

   - [ ] 通過

5. 按核准，看它 merge。

   **這步在驗什麼**：只有你的核准能讓它 merge；merge 落在 main、是一個 merge commit（P7、P8）。錯了的話不是 merge 不了，就是沒核准也 merge 了。

   第三個分頁：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-client --example client_probe -- resolve approval:<t-N>:review:1 approve
   git -C <repo> log --oneline -1 main
   ```

   應該看到：`resolved`；watch 出現 `attention_resolved` 與 `<t-N> merge` → `done`；`git log` 那行是 `<sha> Merge agend/<t-N>/hello: hello`。

   - [ ] 通過

6. 確認清理。

   **這步在驗什麼**：task 結束後 branch、worktree、checks 目錄都不留，binding 快照也清掉（P4）。錯了的話會像 v1 一樣越堆越多。

   ```bash
   git -C <repo> branch --list "agend/*"; ls "$AGEND_HOME/worktrees/" "$AGEND_HOME/checks/"
   cat "$AGEND_HOME/bindings/g10-dev.json"
   ```

   應該看到：前兩個指令什麼都不印；快照裡沒有 `binding`，只有 instance 與 repo。

   - [ ] 通過

7. 故意弄壞：在 agent 的 worktree 裡改 main。

   **這步在驗什麼**：daemon 綁定時真的裝了 hook，在 agent 的 worktree 裡誰都不能動 main（P4、第 3 施工關）。錯了的話 agent 手滑一次就能改掉你的 main。

   操作（開工時細化）：派一個 task 給會停住的假 agent（`pipeline_probe` 的 `--hold`，task 留在 work 關卡），然後在它的 worktree：

   ```bash
   git -C "$AGEND_HOME/worktrees/<t-M>" update-ref refs/heads/main HEAD; echo "exit=$?"
   git -C <repo> log --oneline -1 main
   ```

   應該看到：`agend-shim: refused …` 並指出是 `(agend reference-transaction hook)`，`exit` 不是 0；main 還是步驟 5 那個 commit。

   - [ ] 通過

8. 故意弄壞：task 結束時 worktree 裡留著未 commit 的變更。

   **這步在驗什麼**：WIP 不會跟著 worktree 一起消失，而是先存成 patch（P4）。錯了的話 agent 沒 commit 的東西就永遠不見了。

   操作（開工時細化）：用 `--leave-wip` 的假 agent 派一個 task，照步驟 5 核准，等它 done。

   ```bash
   ls "$AGEND_HOME/archive/"
   ```

   應該看到：一個 `<t-K>-<秒>.patch`，打開看得到那個沒 commit 的檔；watch 裡那個 task 的 done 那行寫著 `archived WIP: archive/<t-K>-<秒>.patch`；worktree 照樣被刪。

   - [ ] 通過

9. 故意弄壞：checks 跑到一半時停掉 daemon。

   **這步在驗什麼**：daemon 停掉再起來，跑到一半的 checks 會重跑，task 照樣走完，不會卡住也不會重複（P9）。錯了的話每次重啟 daemon 都可能留下卡住的 task。

   操作（開工時細化）：用 `pipeline_probe` 的慢 checks（約 20 秒）派一個 task；watch 出現 `checks` 之後，在 daemon 的分頁按 Ctrl-C，再跑 `agend daemon`。

   應該看到：daemon 開機時 `<t-J>: re-running checks attempt 1 after restart`；之後照步驟 5 核准，`git log --oneline main` 裡 `Merge agend/<t-J>/…` 只有一行。

   - [ ] 通過

10. 收尾。

    **這步在驗什麼**：什麼都不留（第 6 施工關的孤兒巡查照舊）。

    操作：各分頁 Ctrl-C，然後 `~/.cargo/bin/cargo run -q -p agend-daemon --example pipeline_probe -- teardown`（開工時細化：刪 instance、再起一次 daemon 收掉 holder、刪暫存目錄）。

    應該看到：`pgrep -fl "agend holder g10-"` 什麼都不印；`/tmp/g10-…` 不見了。

    - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-26 開工前提案 P1–P10 寫定（draft PR），待使用者確認；「你親自驗收」改成 10 步；狀態改為提案中。

## 下一步

```bash
cat docs/gates/gate-10-pipeline.md
~/.cargo/bin/cargo xtask accept pipeline
```
