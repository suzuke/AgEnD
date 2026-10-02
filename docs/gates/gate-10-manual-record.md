# Gate 10 人工驗收紀錄：2026-10-02

> **TL;DR**
> - 使用者在 `45e957e` 完成 11 步操作；本機 pipeline 主流程、WIP、防護、重啟與清理通過。
> - watch／核准事件與指令有問題；修正後的自動／全新獨立驗證及事件補驗仍待完成，未 merge。
> - 下一步：agent 完成收尾驗證後，一次帶使用者補驗事件呈現，再等明確 merge 確認。

## 環境與證據來源

- 實作 branch／worktree：`feat/gate-10-pipeline`，`/Users/suzuke/AlphaCR-worktrees/AgEnD-v2-pipeline`。
- 此紀錄限使用者實際執行的 `45e957e2b095366788ef4e5704600b356f175802`，Rust CLI `agend 0.0.0`；不認證後續提交。
- 使用者逐步貼出 CLI、watch、daemon、Git 與 JSON 輸出，agent 比對；未把未貼出的 stdout 當作有看到。
- 暫存 home `/tmp/g10.UfYP`（canonical `/private/tmp/g10.UfYP`）、repo `/private/tmp/g10.UfYP-repo` 已 teardown。

## 逐步結果

| 步驟 | 結果與可核對證據 |
|---|---|
| 1 demo | 九段完成；`pipeline demo: all sections passed`、`gate 10 (pipeline): checks passed`。 |
| 2 crash recovery | 兩個 failpoint 各四個不同 PID；re-running checks、merge commits=1、無重複派工與清理、新 home negative unknown。 |
| 3 啟動 | daemon pid 94140、holders 94141／94143／94147；三個 instance ready，兩個測試 team。 |
| 4 派工／等待 | t-1 created，checks／review 後待人；JSON stages 順序正確、current_stage approve、head 3008cc24d76925f7d2a35d95bd1e7ba0a97bcd6f。原 watch 不顯示關卡，以 JSON 補查。 |
| 5 人工 merge | resolved；watch t-1 done；main e999273c58b3b703fa6e3713b2bf8df463746606，雙親 5859d0f／3008cc2，標題與 Agend-Task: t-1 正確。核准事件 unknown／重現另列缺陷。 |
| 6 清理 | branch／find 無列出殘留；binding 快照只有 version／instance／source_repo，省略 binding。 |
| 7 main 防護 | t-2 空 commit 901d177 成功；reference-transaction hook 拒絕 update main、exit128；main 仍 e999273；cancelled、worktree removed exit0。 |
| 8 WIP | t-3 done；patch t-3-1790907940911.patch 有 unfinished.txt 與 +uncommitted work；worktree removed exit0、assignee null、archive_paths 有該路徑。 |
| 9 checks 中重啟 | t-4 checks running；02:32:29 UTC SIGINT 停止、02:32:37 新 daemon pid78704 接回原三個 holder，recovered3／started0；checks 重跑20秒通過；t-4 done、main 41c1234 Merge agend/t-4/slow: slow，貼出的查詢僅一筆（wc 計數未貼）。 |
| 10 沙箱 | t-5 多次 checks 的 touch escaped 被 Operation not permitted 拒絕；ls不存在、exit1；取消 accepted，末次 fleet JSON cancelled／assignee null，未進 review／merge。 |
| 11 teardown | 使用者貼 home_removed_exit=0、repo_removed_exit=0；原 daemon／三個 holder 的四個 PID 由 agent 唯讀 ps 補核均不存在，pgrep 未另貼出非空輸出。 |

## 找到的問題與收尾

1. watch 的 task_changed 只有整體 running，文件卻要求可看各關；改讀事件內真 TaskView 的 current_stage 顯示 stage，缺欄位的舊 peer 保留原格式。
2. transition 一律清 task attention，通知型 timeout 即使留在同一 approve ticket 也先移除再重建。使用者 t-3 的 approve timed out log 與 watch 的重現符合此路徑；原始核准項目應保持。
3. 真正 Approve／RequestChanges 的原事件被提前 dismiss 成 unknown；fleet.resolve 時項目已消失。修正需先完成 store CAS，再一次發布真 action；CAS 失敗不能移除項目／發布已核准事件。
4. None binding 原本被指令描述成 null，但 serializer 省略欄位；修正文案，格式不變。
5. resolve 回覆後立即查 archive 時，t-3 JSON 還是 running／merge，故 glob無匹配、worktree exit1。其後 WIP saved／done，再查都通過；指令改等 done 才讀封存，保留這次非零輸出不算初次通過。
6. 原 reviewer assigned log 可在既有 Sent／Confirmed receipt 已略過送件後仍重複印；單靠這些 log 不能證明重複送件。遲來的 t-2／t-3／t-4 result／timeout 均以 stale_result 拒絕，沒有認作新進展。

兩個新的真程序回歸已對原 product binary 重現上述 timeout 重現與 unknown action：exit101、0 passed／2 failed。原 log `/private/tmp/g10-manual-events-before.log` 保留；測試使用真 workflow／store／daemon／Git／CLI watch，不注入合成輸出。修正後上述兩個 native tests exit0／2 passed，另有一個完整 queue 的核准 CAS 失敗回歸 exit0；fmt／workspace clippy／實際 no-std 亦 exit0（logs `/private/tmp/g10-manual-fix-logs`）。收尾的完成版 head、最新 CI 與 verifier 結果由 [PR #143](https://github.com/suzuke/AgEnD/pull/143) 固定，未以舊綠燈代替新結果。

## 下一步

補驗修正後的 watch stage 與 Approve／RequestChanges／timeout 事件；其餘主流程依上述 frozen head 記錄。使用者確認前不 merge。
