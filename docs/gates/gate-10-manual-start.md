# Gate 10 人工驗收：步驟 1–5

> **TL;DR**
> - 由 agent 一次帶一步，先跑完整 demo，再觀察重啟與人工核准 merge。
> - 狀態與驗證證據見 [Gate 10 入口](gate-10-pipeline.md)。
> - 下一步：步驟 5 通過後，繼續 [步驟 6–11](gate-10-manual-finish.md)。

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。`<t-N>` 這類尖括號是會變的 id。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd /Users/suzuke/AlphaCR-worktrees/AgEnD-v2-pipeline    # 本次實作 worktree
export CARGO_TARGET_DIR=/private/tmp/AgEnD-g10-fix-target
~/.cargo/bin/cargo build -p agend -p agend-testkit --bins
export PATH="$CARGO_TARGET_DIR/debug:$PATH"
agend --version
```

應該看到 `agend 0.x.y`（目前是 `agend 0.0.0`）。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

`AGEND_HOME` 一定要設（第 13 施工關之前沒有預設值），沒設的話 `agend` 會拒絕執行。步驟 1–2 的 demo 自己建暫存 home、自己設 `AGEND_HOME`，不用你設。步驟 3 起用同一個暫存 home：每個步驟的指令第一行都是 `export AGEND_HOME=<home>`，把 `<home>` 換成步驟 3 印出的路徑（同一個分頁設過一次就好，新分頁要再設）。repo 在 `$AGEND_HOME-repo`。

1. 跑 demo。

   **這步在驗什麼**：同一套流水線在真的 git、真的 daemon 上跑完每一段：順利、checks 失敗返工、reviewer 要求修改、main 前進、留下 WIP、hook、沙箱（寫外面被擋、寫裡面可以、沒有工具就不跑）、四次開機。錯了代表後面手動看到的都不可信。

   ```bash
   cd /Users/suzuke/AlphaCR-worktrees/AgEnD-v2-pipeline
   CARGO_TARGET_DIR=/private/tmp/AgEnD-g10-fix-target \
     ~/.cargo/bin/cargo xtask accept pipeline
   ```

   應該看到：`== happy`、`== checks-fail`、`== changes`、`== wip`、`== main-advanced`、兩個 `== restart: …`、`== sandbox`、`== hooks`，倒數第二行 `pipeline demo: all sections passed`，最後一行 `gate 10 (pipeline): checks passed`。

   - [ ] 通過

2. 四次開機，兩次在危險的地方被中止。

   **這步在驗什麼**：daemon 死在 checks 中間、死在「main 已經動了、還沒記下來」的那一刻，重開後 task 照樣走完，而且只 merge 一次（P2、P7、P9）。錯了的話 daemon 當掉一次，main 上就可能多一個重複的 merge，或 task 永遠卡住。

   操作：同一次輸出，找 `== restart`。應該看到：

   ```text
   boot 1 daemon pid=<A>: <t-1>/checks/1 running; kill -9
   boot 2 daemon pid=<B>: <t-1>: re-running checks; review approved; waiting for approve
   boot 3 daemon pid=<C>: failpoint after-main-moved
   boot 4 daemon pid=<D>: <t-1>: merge found on main; not merged again
   <t-1>: four boots; after-main-moved; merge commits=1; no duplicate dispatch; cleanup complete
   negative check (new AGEND_HOME): boot 2 failed: <t-1> unknown
   ```

   | 看什麼 | 意思 |
   |---|---|
   | 四個不同 pid 與 boot 2 的 `re-running checks` | 真正重啟四個程序，checks 重新執行 |
   | `not merged again`、`merge commits=1` | merge 只做一次 |
   | `no duplicate dispatch; cleanup complete` | 每個 ticket 一次派送，worktree／branch／binding 清掉 |
   | `negative check … unknown` | 換新 home 接不起前一個 task |

   - [ ] 通過

3. 你自己動手：建暫存 repo，前景啟動 daemon。

   **這步在驗什麼**：真的 `agend daemon` 讀到兩個 team、repo、三個假 agent，而且都起來了（P3、P11）。錯了的話後面沒有東西可驗。

   在第一個分頁：

   ```bash
   export AGEND_HOME="$(mktemp -d /tmp/g10.XXXX)" && echo "export AGEND_HOME=$AGEND_HOME"
   ~/.cargo/bin/cargo run -q -p agend-daemon --example pipeline_probe -- setup
   agend daemon
   ```

   記下第一行印出的 `export AGEND_HOME=…`（之後的 `<home>` 就是它）。

   應該看到：`setup` 印出 `repo=<home>-repo`、兩個 team 與三個 agent（name `g10-dev`、`g10-rev`、`g10-hold`）；daemon 最後一行 `agend daemon ready: instances=3 …`。daemon 留在前景。

   - [ ] 通過

4. 派一個 task，看它一關一關走，停在等你核准。

   **這步在驗什麼**：pipeline 依 workflow 的順序推進，agent 審查通過後，人工核准出現在「需要你」（P1、P8）。錯了的話 task 會跳關，或卡住而你不知道。

   第二個分頁（先跑開頭那段）：

   ```bash
   export AGEND_HOME=<home>    # 步驟 3 的那個；每個新分頁都要先設
   agend debug watch
   ```

   第三個分頁（同樣先跑開頭那段），以操作者身分開 task：

   ```bash
   export AGEND_HOME=<home>    # 步驟 3 的那個；每個新分頁都要先設
   agend task create --team g10 --role dev --workflow demo "hello"
   ```

   應該看到：印出 task id `<t-N>`；watch 依序出現 `<t-N>` 的 `work` → `submit` → `checks` → `review`，最後 `attention_required approval:<t-N>/approve/1 … actions: approve, request_changes`，然後停住。

   - [ ] 通過

5. 按核准，看它 merge。

   **這步在驗什麼**：只有你的核准能讓它 merge；merge 落在 main、是一個帶 trailer 的 merge commit（P7、P8）。錯了的話不是 merge 不了，就是沒核准也 merge 了。

   第三個分頁：

   ```bash
   export AGEND_HOME=<home>    # 步驟 3 的那個；每個新分頁都要先設
   ~/.cargo/bin/cargo run -q -p agend-client --example client_probe -- resolve approval:<t-N>/approve/1 approve
   git -C "$AGEND_HOME-repo" log -1 main
   ```

   應該看到：`resolved`；watch 出現 `attention_resolved` 與 `<t-N> merge` → `done`；`git log` 的標題是 `Merge agend/<t-N>/hello: hello`，最後一行 `Agend-Task: <t-N>`。

   - [ ] 通過

## 下一步

步驟 5 通過後，繼續 [步驟 6–11](gate-10-manual-finish.md)。
