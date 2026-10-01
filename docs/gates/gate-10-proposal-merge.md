# Gate 10 提案：merge 與需要你

> **TL;DR**
> - P7–P8 定義 local merge 與人工例外。
> - 狀態與驗證證據見 [Gate 10 入口](gate-10-pipeline.md)。
> - 下一步：閱讀 [重啟／命令提案](gate-10-proposal-commands.md)。

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
       - 只有 canonical checkout 在 main，而且工作目錄乾淨 → 在那裡 `git merge --ff-only <新>`（main 被別人動過就失敗，等同 CAS；工作目錄一起更新）。**與 [pipeline](../architecture/pipeline.md#merge-與-main-前進)「不在使用者工作目錄 `git merge`」不同，使用者 2026-09-26 決定照這裡做**：這裡只做 fast-forward、只在乾淨時做，不會產生衝突或新 commit；不同意的話改成「main 被任何地方 checkout 就 `merge-blocked`」。
       - 其他情況（canonical 在 main 但有未 commit 的修改、或別的 worktree checkout 了 main）→ **不 merge**，出現「需要你」`merge-blocked`（P8），寫出是哪個目錄。
  - main 前進（D14）**只在送出 merge 前檢查一次**：head 不包含目前的 main → daemon 在持有者的 worktree 裡 rebase（P5）→ 餵 `MainAdvanced{rebased_head, patch_id, conflict}` 再餵 `MergeFailed`（core 已規定送出中的 head 變更要等 `MergeFailed` 才套用）。patch-id 相同 → 保留核准、重跑 checks；不同或衝突 → 回 work。所以 checks 一定是在「已包含最新 main 的 head」上通過的，merge 出來的樹就是測過的樹。
  - 判斷「這個 task 是不是已經 merge 了」（送 `Merge` 前、以及 P9 開機時）：
    1. 在 main 的 first-parent 歷史裡找 trailer `Agend-Task: <task>`、第二個父 commit 等於目前 head 的 commit（`git log --first-parent --grep`，最多看 1000 個）→ 找到就是已 merge，merge commit 取它。
    2. 找不到，但 `merge_intent` 有值而且在 main 裡 → 已 merge，取 `merge_intent`。
    3. 都沒有，但 head 已經在 main 裡（有人手動 merge 了它）→ 也算完成：餵 `MergeCompleted`，merge commit 取 main 的 first-parent 歷史裡**最舊（最早）**一個包含 head 的 commit，task 事件註明「merged outside agend」。**與 [pipeline](../architecture/pipeline.md#merge-與-main-前進)「done 以 daemon 的 merge 記錄為準，不從 git 推論」不同，使用者 2026-09-26 決定照這裡做**：這一條是從 git 推論 done（只在這種手動 merge 時）；不同意的話要改 core，讓 merge 送出中也能收 `StageFailed`，task 失敗交給你。不用 `StageFailed`：送出 `Merge` 之後 core 只接受 merge 結果與 head 變更（`MergeInFlight`），`StageFailed` 會被拒、task 卡住。
    4. 以上都不是 → 還沒 merge，照上面的步驟做。
  - 只對 forge local 成立（它從不 squash、一定寫 trailer）；GitHub 在第 12 施工關另外處理。done 以 DB 裡的 `merge_commit` 為準；不從 git 推論其他 task（V1-LESSONS #6）。
- 理由：`merge-tree` 不碰任何工作目錄（pipeline.md 已定）；拒絕空 branch 之後，「已經 merge」的證據只剩我們自己寫的 trailer 與 `merge_intent`，兩者都綁這個 task，不會把別人的 commit 當成自己的 merge。只在 merge 前處理 main 前進，只有一個觸發點。
- 替代方案：只看「head 在 main 裡」、不拒絕空 branch（空 branch 一 `done` 就被當成已 merge）；手動 merge 時讓 task 失敗（要改 core：送出 `Merge` 後 core 不接受 `StageFailed`）；main 一前進就 rebase 所有進行中的 task（要處理 agent 正在改的 worktree）；fast-forward merge（main 上看不出哪個 task 什麼時候 merge，也沒有 trailer 可找）；main 被 checkout 時照樣 `update-ref`（那個工作目錄會變成一堆反向修改）。
- 例子：t-3、t-4 都到了 merge。t-3 先 merge；t-4 送 merge 前發現 head 不含新的 main → rebase 無衝突、patch-id 相同 → log `t-4: main advanced; rebased, approvals kept, checks run again` → checks 通過 → merge。`git log --oneline main` 最上面兩個是 `Merge agend/t-4/…`、`Merge agend/t-3/…`。
- [x] 使用者確認（2026-09-26）

### P8：「需要你」的新來源與協定補充

- 問題：人工核准、請示、task 失敗、merge 被擋、缺角色，你怎麼知道、怎麼處理？項目 id 怎麼跟 ticket 對上？
- 建議：沿用第 8 施工關的 `attention_required`／`resolve_attention`／`attention_resolved`，本關加六種來源（跟 `instance-failed` 一樣從 DB 算，不另存表）：

  | `attention_id` | 什麼時候出現 | `actions` | 按了之後 |
  |---|---|---|---|
  | `approval:<task>/<stage>/<attempt>` | `approval(by = "human")` 關卡等你 | `approve`、`request_changes` | `ApprovalGranted`（綁 head 的關卡綁送出要求時的 head）或 `ChangesRequested`（理由必填） |
  | `<ask id>`（不加前綴） | agent `agend ask`（P10） | 照第 8 施工關的請示：`answer_ask` | 回答送回 agent；agent 追問就再出現，結論後消失 |
  | `task-failed:<task>` | task 失敗（含 P2 快照壞掉） | `acknowledge` | 從清單拿掉；task 保持 `Failed`，WIP 已存 patch |
  | `sandbox-missing:<task>` | 找不到沙箱工具或試跑失敗，checks 沒跑（P6） | `retry` | 再找一次工具、找到就跑 checks |
  | `merge-blocked:<task>` | main 被 checkout 著不能動（P7） | `retry` | 再檢查一次、可以就 merge；這時不能取消（P10），要先把那個 checkout 清乾淨 |
  | `no-role:<team>/<role>` | team 裡沒有這個角色，或審查時這個角色只有作者（P3、P10）；都在忙不算 | 無 | 你用 `agend team join` 加了這個角色後自動消失 |

  - **id 的寫法**：請示照第 8 施工關 P5，直接用 ask id、不加前綴；其他是 `<種類>:<對象>`，跟第 8 施工關的 `instance-failed:<id>` 一樣用 `:` 分開種類；對象是 task 關卡時**就是第 9 施工關的 ticket**（`t-3/review/1`），所以同一串字會同時出現在 `agend status`、派工訊息與「需要你」裡。
  - `request_changes` 要帶理由：`resolve_attention` 加一個選填欄位 `note`；缺少時回 `invalid_request`。
  - `approval:` 的 id 帶 attempt：head 變了、開了新的 attempt，舊項目自動消失、新項目出現；拿舊 id 按會回 `unknown_attention`，不會核准到新的 head。
  - `unblocks`：task 相關的是 1，`no-role` 是在排隊的 task 數；`waiting_since` 是 `tasks.stage_entered_at_unix_ms`（P2，重開機不重算；請示用 ask 建立的時間）；`task_id`、`instance_id`（持有者）照填。脈絡摘要（D37）依當前 WorkProduct 顯示 Result summary／output、Plan items 或 Branch／head；下一步依 workflow 顯示接手 Work 或繼續後續關卡，核准 planned Result 時須能看到實際成果內容。
  - 協定新增（`note`、P10 的 `task_cancel` 與 team／workflow 請求）算一次 minor：client 協定用**實作時的下一個 minor**（第 8 施工關是 1.1；第 9 施工關先 merge 就拿 1.2，本關再下一個）。
  - **與 D18 字面不同，使用者 2026-09-26 決定照這裡做**：D18 寫「需要不存在的角色時轉成 ask」。這裡改成 `no-role` 項目，因為 ask 是 agent 與你的對話（D35），而這件事的解法是「加一個成員」，不是回答問題。
- 理由：同一套「需要你」機制，TUI（第 11 施工關）與 Telegram（第 12 施工關）不必各做一次核准；只多一個選填欄位。
- 替代方案：人工核准做成請示（理由可以自由文字回答，但核准要綁 head，請示沒有身分）；`task-failed` 不放進「需要你」（失敗的 task 容易沒人發現）；id 全用 `:`（`approval:t-3:review:1`，跟 ticket 對不起來）。
- 例子：`demo` workflow 走到人工核准 → `agend debug watch` 印 `attention_required approval:t-3/approve/1 (unblocks 1; if ignored: t-3 waits for you) actions: approve, request_changes` → 你按 `approve` → `attention_resolved …` → merge。
- [x] 使用者確認（2026-09-26）

## 下一步

閱讀 [重啟／命令提案](gate-10-proposal-commands.md)。
