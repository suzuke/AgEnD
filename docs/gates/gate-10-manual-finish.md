# Gate 10 人工驗收：步驟 6–11

> **TL;DR**
> - 沿用步驟 3 建立的 home，驗清理、防護、WIP、重啟與沙箱。
> - 狀態與驗證證據見 [Gate 10 入口](gate-10-pipeline.md)。
> - 勾選記 `45e957e` 的人工結果；事件收尾待複驗，見 [人工紀錄](gate-10-manual-record.md)。

先完成 [步驟 1–5](gate-10-manual-start.md)。每個新終端先跑該頁的 CLI 設定。

6. 確認清理。

   **這步在驗什麼**：task 結束後 branch、worktree、checks 目錄都不留，binding 快照也清掉（P4、P6）。錯了的話會像 v1 一樣越堆越多。

   ```bash
   export AGEND_HOME=<home>    # 步驟 3 的那個；每個新分頁都要先設
   git -C "$AGEND_HOME-repo" branch --list "agend/*"
   find "$AGEND_HOME/worktrees" "$AGEND_HOME/checks" -mindepth 1
   cat "$AGEND_HOME/bindings/g10-dev.json"
   ```

   應該看到：前兩個指令什麼都不印（daemon 開機時一定建好這兩個目錄，所以 `find` 不會報錯）；快照省略 `binding`（序列化 None 時省略；讀取時 null 也代表解除綁定），保留 instance 與 repo。

   - [x] 人工主流程通過（45e957e）

7. 故意弄壞：在 agent 的 worktree 裡改 main，然後取消那個 task。

   **這步在驗什麼**：daemon 綁定時真的裝了 hook，在 agent 的 worktree 裡誰都不能動 main（P4、第 3 施工關）；你取消 task 後它照樣被清掉（P10）。錯了的話 agent 手滑一次就能改掉你的 main，或卡住的 task 永遠佔著 agent。

   第三個分頁。`g10h` 那個 team 的假 agent 收到派工後什麼都不做，task 會停在 work 關卡，不影響 `g10`：

   ```bash
   export AGEND_HOME=<home>    # 步驟 3 的那個；每個新分頁都要先設
   agend task create --team g10h --role dev --workflow demo "held"
   AGEND_INSTANCE=g10-hold git -C "$AGEND_HOME/worktrees/<t-M>" commit --allow-empty -m "Hook probe"
   AGEND_INSTANCE=g10-hold git -C "$AGEND_HOME/worktrees/<t-M>" update-ref refs/heads/main HEAD
   echo "exit=$?"
   git --no-pager -C "$AGEND_HOME-repo" log --oneline -1 main
   agend task cancel <t-M>
   ```

   `AGEND_INSTANCE` 只套用到這兩次 git，讓 hook 用有效 binding 判斷；後續 cancel 仍用 operator 身分。

   應該看到：空 commit 成功；更新 main 的指令印出 `agend-shim: refused …` 並指出是 `(agend reference-transaction hook)`，`exit` 不是 0；main 還是步驟 5 那個 commit；`cancel` 之後 watch 出現 `<t-M>: cancelled`，`worktrees/<t-M>` 不見了。

   - [x] 人工主流程通過（45e957e）

8. 故意弄壞：task 結束時 worktree 裡留著未 commit 的變更。

   **這步在驗什麼**：WIP 不會跟著 worktree 一起消失，而是先存成 patch（P4）。錯了的話 agent 沒 commit 的東西就永遠不見了。

   在第三個分頁先讓假 worker 留 WIP，再開 task；照步驟 5 核准，**等 watch 出現該 task 的 done 後才查 archive／worktree**；resolve 成功只表示核准已接受，不代表 merge／清理已完成。

   ```bash
   export AGEND_HOME=<home>
   touch "$AGEND_HOME/workspace/g10-dev/.leave-wip"
   agend task create --team g10 --role dev --workflow demo "wip"
   ```

   ```bash
   export AGEND_HOME=<home>    # 步驟 3 的那個；每個新分頁都要先設
   ls "$AGEND_HOME/archive/"
   cat "$AGEND_HOME/archive/<t-K>-<unix-ms>.patch"
   rm "$AGEND_HOME/workspace/g10-dev/.leave-wip"
   ```

   應該看到：一個 `<t-K>-<unix-ms>.patch`，打開看得到那個沒 commit 的檔；task detail 帶 archive 路徑；daemon log 有 `<t-K>: WIP saved <path>`；worktree 照樣被刪。

   - [x] 人工主流程通過（45e957e）

9. 故意弄壞：checks 跑到一半時停掉 daemon。

   **這步在驗什麼**：daemon 停掉再起來，跑到一半的 checks 會在新目錄重跑，task 照樣走完，不會卡住也不會重複（P6、P9）。錯了的話每次重啟 daemon 都可能留下卡住的 task。

   **先讀完再操作，slow checks 只等待 20 秒；看到 checks 後立即停機，不要先貼輸出。** 第三個分頁（已設 `AGEND_HOME`）`agend task create --team g10 --role dev --workflow slow "slow"`；watch 出現 `(stage: checks)` 之後，在 daemon 的分頁按 Ctrl-C，再跑 `agend daemon`。

   應該看到：daemon 開機時 `<t-J>: re-running checks <t-J>/checks/1 after restart`；之後照步驟 5 核准，`git log --oneline main` 裡 `Merge agend/<t-J>/slow: slow` 只有一行。

   - [x] 人工主流程通過（45e957e）

10. 故意弄壞：checks 想寫到它的 worktree 外面。

    **這步在驗什麼**：checks 在寫入沙箱裡跑，agent 的程式碼改不到別的 repo（P6）。錯了的話 agent 寫的測試可以改你的 home、其他 repo 或 main。

    ```bash
    export AGEND_HOME=<home>    # 步驟 3 的那個；每個新分頁都要先設
    agend task create --team g10 --role dev --workflow escape "escape"
    ls "$AGEND_HOME-repo/escaped"; echo "exit=$?"
    agend task cancel <t-E>
    ```

    等 daemon log 出現 `<t-E>: checks … failed` 與寫入被拒絕再跑第二行。應該看到：log 那段寫著 `touch: …/escaped: Operation not permitted`（Linux 是 `Read-only file system`）；`ls` 找不到檔案、`exit` 不是 0；取消後 `<t-E>: cancelled`。

    - [x] 人工主流程通過（45e957e）

11. 收尾。

    **這步在驗什麼**：什麼都不留（第 6 施工關的孤兒巡查照舊）。

    操作：各分頁 Ctrl-C，然後在設了 `AGEND_HOME` 的分頁 `~/.cargo/bin/cargo run -q -p agend-daemon --example pipeline_probe -- teardown`。

    應該看到：`pgrep -fl "agend holder g10-"` 什麼都不印；`<home>` 與 `<home>-repo` 都不見了。

    - [x] 人工主流程通過（45e957e）

## 下一步

本次 [人工紀錄](gate-10-manual-record.md) 已保存；收尾修正經自動／全新 verifier 通過後，補驗 watch 與核准事件，再由使用者確認 merge。
