# 第 3 施工關：agend-shim（`shim`）

> **TL;DR**
> - agent 的 git 與 kill 防護：protected ref 由只裝在 agent worktree 的 git hook 守（git 自己回報要改哪些 ref）；PATH 上的 shim 負責導向綁定的 worktree、破壞性操作前快照、擋離開 branch、kill 防護。只防「好意但會犯錯」的 agent（見[威脅模型](#威脅模型)）。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」、看過「待你追認」與「已知限制」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：照「你親自驗收」跑一次，再逐條看「待你追認」。

## 威脅模型

使用者已決定（2026-09-25，T19）。verifier 用這一節判斷「這算不算漏洞」。

| | 內容 |
|---|---|
| 防 | 好意但會犯錯的 agent：打錯字、習慣性 `reset --hard`／`clean -fd`、push 錯 branch、在錯的目錄跑、kill 錯 pid |
| 不防 | 故意繞過的 agent：特製的 `-c remote.x.url=…`、`-c core.hooksPath=` 配真的 git、改檔案、關掉或刪掉 hook |
| 硬保證在哪 | protected ref：agent worktree 的 `reference-transaction`／`pre-push` hook（本施工關；第 6 施工關由 daemon 在綁定時安裝）＋ forge 端 branch protection |

判斷方法：
- 正常 agent 可能不小心打出來的指令 → shim 或 hook 要擋；擋不到就是 bug。
- 要刻意組出來才會成功的 → 列進「已知限制」。

同 v1 agentic-git 的定位：安全帶，不是籠子（a seatbelt, not a cage）。

## 狀態

**實作中**（2026-09-25，draft PR #107；依使用者決定改成 hook 設計（T21），verifier 第 1–9 輪的犯錯類案例改由 shim 或 hook 擋；agent 不寫 git stash（使用者已決定，T22）、不用 autostash（使用者已決定，T23），待 fresh verifier 與你親自驗收）

## 範圍

- git hook（`agend_shim::hook`；裝在哪見 [README](../../crates/agend-shim/README.md#hook-安裝在哪)）
  - `reference-transaction`（`prepared` 階段）：拒絕寫 protected ref（main、master、binding 快照列的）、自己 `agend/<task>/` 以外的 branch（別的 agent 的、新的 `feat/x`）、刪除自己綁定的 branch、`refs/stash`（T22：canonical 與每個 worktree 共用同一個 stash 清單）；經 symbolic ref 的寫入 git 回報的是真正的目標。讀不到 binding 時只放行 `refs/remotes/`；看不懂的輸入行（未來 git 改格式）也拒絕（fail closed）
  - `pre-push`：遠端 ref 必須是自己綁定的 branch、不能刪；`git push` 沒寫目的地時由 git 自己解析，hook 看到的就是真正的目的地（T7）
  - 每個 hook 檢查完接著跑專案的同名 hook（同參數、同 stdin），專案 hook 照常跑；hook 目錄在 hook 執行時才查（共用 config 的 `core.hooksPath`，否則 `<common dir>/hooks`），綁定之後才設的（例如 husky）也串接
  - `install_hooks`／`uninstall_hooks`：`extensions.worktreeConfig`，只寫該 agent worktree 的 `config.worktree`（`core.hooksPath`、`gc.packRefs=false`）；canonical checkout、別的 checkout、`~/.gitconfig` 不動
- git shim：hook 看不到的
  - 導向：綁定時從 canonical checkout、或從 workspace 不帶 `-C`／`--git-dir`／`--work-tree`／`GIT_DIR`／`GIT_WORK_TREE` 跑的 git 改在綁定的 worktree 跑，保留子目錄；別的 worktree 裡的寫入、worktree 沒有的目錄（或在 worktree 裡是 submodule／巢狀 repo）、git dir 裡的寫入拒絕；呼叫者指定的地方 git 找不到 repo 時（`-C` 打錯字）寫入拒絕，絕不改成在整個 worktree 跑（第 10 輪）；位置問真的 git（`rev-parse`，T20）
  - 寫入只作用在綁定的 worktree：work tree、`GIT_INDEX_FILE` 要是它的；需要導向的寫入帶 `--git-dir`／`--work-tree`／`GIT_*` 就拒絕（T13）
  - 離開綁定的 branch：`checkout`／`switch` 到別的 branch、detach（含 `checkout refs/heads/<自己的 branch>`：寫完整 ref 名稱 git 會 detach）、`-b`／`-c`／`--orphan`（git 2.39 不把 HEAD 換 branch 告訴 hook）；`git worktree`（`list` 除外）；不認得的子命令（alias）
  - `branch -c/-C/-m/-M`（`--copy`／`--move`，含縮寫與 `-fm` 這類組合）一律拒絕，自己命名空間裡的也一樣：git 2.39 寫新名稱不經 ref transaction，hook 看不到；下一步給 `git branch <新> <舊>`（經 hook）再 `git branch -D <舊>`
  - `symbolic-ref` 的寫入（有目標、`-d`、`-m`）與 `reflog delete`／`reflog expire` 一律拒絕：git 2.39 改 ref 不經 ref transaction，hook 看不到（第 7 輪：`symbolic-ref refs/heads/main <自己的 branch>` 讓 main 指向 agent 的 commit、`reflog delete --updateref main@{0}` 移動 main、`reflog expire --all` 清掉整個 repo 的 reflog）；讀取照常（`symbolic-ref [-q] [--short] <name>`、`reflog`／`reflog show`）
  - 快照救不回的一律拒絕：`git stash` 的寫入（push、不帶子命令的 `stash`、save、pop、apply、drop、clear、branch、create、store；T22），下一步給 `git commit -m "wip: …"` 存在自己的 branch；`stash list`／`stash show` 照常。autostash（T23）：`pull`／`rebase`／`merge` 的 `--autostash`（含縮寫）、`-c`／`--config-env`／`GIT_CONFIG_*` 設 `rebase.autoStash`／`merge.autoStash`、config 檔設了而沒帶 `--no-autostash`，下一步一樣是 `git commit -m "wip: …"`，再帶 `--no-autostash` 重跑。`clean -x`／`-X`（含 `-fdx`、`-fX` 這類組合；ignored 檔例如 `.env` 不在快照裡）與 `clean -ff`（`-f` 兩次，含 `-ffd`、`-fdf`、`--force --force`；會刪掉未追蹤的巢狀 repo 與它只在本機的 commit，快照裡只有 gitlink），下一步給 `git clean -fd`（先快照；git 會跳過巢狀 repo）或刪指定路徑。submodule（第 9 輪）：worktree 有 `.gitmodules`、而且會 recurse（`--recurse-submodules`，或 `-c`／config 設了 `submodule.recurse` 而沒帶 `--no-recurse-submodules`）時，破壞性的 `reset`／`checkout`／`restore`／`switch` 拒絕，下一步是先在 submodule 裡 commit，或加 `--no-recurse-submodules`；`submodule foreach` 一律拒絕（git 用自己的 git 跑它的命令，不經 shim），下一步是 `git -C <path> <命令>` 逐一跑
  - 不讓 hook 被跳過：`-c`／`--config-env`／`GIT_CONFIG_*` 設 `core.hooksPath`、`push --no-verify`；綁定的 worktree 沒裝 hook 時寫入拒絕
  - 破壞性操作前快照：v1 agentic-git 的範圍（`reset --hard|--merge|--keep`、`clean`、`checkout`、`restore`（非只 `--staged`）、`switch -f|--discard-changes`、`rm -f`、`mv -f`、merge／rebase／pull／cherry-pick／revert／am）；快照存 work tree（含未追蹤、不含 ignored），不存 `refs/stash`；**submodule 與巢狀 repo 在快照裡只有 gitlink**（指到哪個 commit），裡面未提交的修改與只在本機的 commit 都不在快照裡；綁定的 worktree 裡的 submodule 或巢狀 repo（`cd mod`、`git -C mod`）原本當外部 repo 放行，現在破壞性操作前改在那個 repo 自己快照（快照 ref 在它裡面，還原那行要在它裡面跑）；canonical checkout 與別的 worktree 的 submodule 裡的寫入照「別的 worktree」拒絕；選項寬鬆比對（git 接受的任何縮寫都算，例如 `checkout --de`）
  - 沒有 hook 的 repo：team 的本機 remote 與 team repo 的 clone 不能寫、從別的 repo push 到 team repo 拒絕（T5）
- `kill`／`killall`／`pkill` 防護（T9，未改）、audit 記錄（shim 與 hook 的拒絕、bypass、快照）
- binding 來源：daemon 寫的唯讀 binding 快照（D6，無 HMAC）

## 自動驗收（完成定義）

在 repo 根目錄跑：

```bash
~/.cargo/bin/cargo fmt --all -- --check
~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
~/.cargo/bin/cargo test --workspace
TMPDIR=$(mktemp -d) ~/.cargo/bin/cargo test --workspace   # 不靠暫存目錄裡原有的東西
~/.cargo/bin/cargo xtask check-deps
~/.cargo/bin/cargo xtask accept shim
```

- [x] `cargo test -p agend-shim`：`54 passed`（unit）；`cargo test -p agend`：`4`（argv0）+ `18`（`shim_bypass_corpus`）+ `2`（`shim_everyday`）+ `14`（`shim_git`）+ `8`（`shim_hooks`）+ `6`（`shim_location_matrix`）+ `2`（`shim_push_hook`）+ `8`（`shim_route_scope`）+ `5`（`shim_submodule`）passed；`TMPDIR` 設成新的空目錄時一樣全過
- [x] clippy 乾淨；`check-deps` 最後一行 `check-deps: ok (2 rules, 8 crates checked for agend-testkit, agend-core metadata ok, no-std build ok)`
- [x] `cargo xtask accept shim` 最後兩行 `shim demo: all checks passed`、`gate 3 (shim): checks passed`
- [x] 第 1–8 輪的犯錯類案例都是回歸測試，斷言「被 shim 或 hook 拒絕，protected ref 不動」（`crates/agend/tests/shim_bypass_corpus.rs`）；把 hook 的判斷改成永遠放行時 corpus 變紅；第 9 輪的 submodule 與巢狀 repo 在 `crates/agend/tests/shim_submodule.rs`，在舊程式碼上 4 個都紅
- [x] canonical checkout 沒有 agend hook、在那裡 commit 到 main 照常成功；專案原本的 hook 從 agent worktree 照常跑；agent worktree 裡 `git gc` 照常（`shim_hooks.rs`）
- [x] 第 3 輪的位置矩陣（336 + 168）與第 4 輪的正常工作清單照常全綠
- [x] 用真的 binary 在 sandbox 裡掃 38 個常見錯誤 × 4 個 cwd（152 案）：protected ref、canonical 的未提交工作、綁定的 branch 都沒動，破壞性操作都有快照；56 案由 hook 擋、36 案由 shim 擋
- [x] `crates/agend-shim/README.md`／`TESTING.md`、`crates/agend/README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。demo 在暫存目錄建 repo，hook 只裝在那裡的 worktree，不碰你的 repo；輸出裡的暫存路徑每次不同。demo 的 `kill` 只會打到假的記錄程式（`<tmp>/fakebin/kill`），不會送出任何訊號。

1. 跑 demo。

   **這步在驗什麼**：shim 與 hook 在真的 `agend` binary、真的 git repo 上跑得起來，每個行為都有自動檢查；錯了的話後面每一步看到的輸出都不可信。

   ```bash
   ~/.cargo/bin/cargo xtask accept shim
   ```

   應該看到：fmt、clippy、`cargo test -p agend-shim`、`cargo test -p agend`、check-deps，接著 `== shim demo ==`，setup 有一行 `agend git hooks  <tmp>/home/hooks, set in the worktree's config.worktree only`，然後 7 段 `-- route`、`-- refuse`、`-- snapshot`、`-- protected ref`、`-- hooks`、`-- kill guard`、`-- audit`，每個檢查一行 `ok: …`。最後兩行：`shim demo: all checks passed`、`gate 3 (shim): checks passed`。

   - [ ] 通過

2. 導向：在綁定狀態下 commit。

   **這步在驗什麼**：agent 在 workspace 或 canonical checkout 裡下的 git 指令會落到自己的 worktree；錯了的話 agent 的 commit 會進 canonical checkout 或 main。

   操作：同一次輸出，找 `-- route`

   應該看到：`ok: commit "fix" is on agend/t-1/fix`；在 canonical checkout（`cwd: <tmp>/repo`）跑的 `git add` 印出 `agend-shim: running in your bound worktree …`；最後 `ok: canonical main did not move`、`ok: canonical checkout is clean`。

   - [ ] 通過

3. 故意弄壞：`git checkout main`、`git worktree add`。

   **這步在驗什麼**：agent 不能離開綁定的 branch，也不能自己開 worktree（這兩件事 git 2.39 不會告訴 hook，所以由 shim 擋）；錯了的話 agent 會在錯的 branch 上工作，daemon 收不到它的成果。

   操作：同一次輸出，找 `-- refuse`

   應該看到 `exit 1` 與：

   ```text
   | agend-shim: refused `git checkout main`
   | agend-shim: why: `git checkout main` would leave your branch: you are bound to agend/t-1/fix (main is protected: only the daemon changes it)
   | agend-shim: next step: stay on agend/t-1/fix in <tmp>/home/worktrees/t-1. … To restore a file instead: git checkout -- <path>  or  git restore <path>
   ```

   接著 `ok: still on agend/t-1/fix`；`git worktree add ../mine` 也被拒絕（`only the daemon creates and removes worktrees`）、`ok: no new worktree`。

   - [ ] 通過

4. `git reset --hard` 快照與還原。

   **這步在驗什麼**：破壞性指令前一定先快照，照訊息就能救回；錯了的話 agent 一個 `reset --hard` 就把沒存的工作永久弄丟。

   操作：同一次輸出，找 `-- snapshot`

   應該看到：`agend-shim: snapshot <id> saved before `git reset --hard HEAD~1`` 與 `agend-shim: to undo: git reset --keep <sha> && git restore --source=refs/agend/snapshots/dev-1/<id> -- :/`；demo 照這行執行後 `ok: fix.txt is back, including the unsaved work`、`ok: the reset commits are back`。

   - [ ] 通過

5. protected ref：hook 擋住寫 main，自己的 branch 照常 push。

   **這步在驗什麼**：不管指令怎麼寫，只要 git 要改 main（或推到 main），hook 就拒絕；錯了的話 agent 可以跳過 review 把 code 推進 main。

   操作：同一次輸出，找 `-- protected ref`

   應該看到：`git update-ref refs/heads/main HEAD`、`git push . HEAD:main`、`git branch -f master HEAD`、`git push origin HEAD:main` 各有 `agend-shim: refused `… (agend reference-transaction hook)`` 或 `(agend pre-push hook)`，理由 `it is a protected ref and only the daemon changes it`；接著 `ok: main did not move`、`ok: origin's main did not move`、`ok: pushing your own branch runs`、``ok: plain `git push` runs (git resolves it to your branch)``。

   - [ ] 通過

6. hook 只在 agent worktree、專案 hook 照跑、不能跳過。

   **這步在驗什麼**：hook 不會影響你自己的 canonical checkout，專案原本的 hook 不會因此失效，agent 也不能用 `-c core.hooksPath`／`--no-verify` 順手繞過；錯了的話你自己 commit 到 main 會被擋，或專案的 pre-commit 檢查默默不跑。

   操作：同一次輸出，找 `-- hooks`

   應該看到：`ok: the canonical checkout has no core.hooksPath`、`ok: the project's pre-commit hook ran from the agent worktree`、兩個 `ok: skipping the hooks is refused`（理由含 `would skip the agend`）、`ok: a human commit on main in the canonical checkout still works`。

   - [ ] 通過

7. 故意弄壞：`kill` 一個 holder、`pkill` 別人的程序。

   **這步在驗什麼**：agent 不能用外部的 `kill`／`pkill` 弄掉 daemon 或 holder；錯了的話一個 `pkill` 就讓整個團隊的 agent 一起掛掉。

   操作：同一次輸出，找 `-- kill guard` 與 `-- audit`

   應該看到：`kill <pid>`（假 holder）被拒絕，理由 `is an agend process`；`pkill -f sleep 300` 被拒絕，下一步 `pgrep -fl -- 'sleep 300'`；`ok: holder still alive`；`kill <自己的 pid>` 是 `exit 0`、`ok: only that call reached the (fake) real kill`。`-- audit` 段列出 10 筆 `refuse`（其中 4 筆是 hook 記的 `protected_ref`）與 3 筆 `snapshot`。

   - [ ] 通過

8. 自己動手：hook 裝在哪、binding 快照壞掉時。

   **這步在驗什麼**：你親眼看到 hook 只設在 agent worktree 自己的 `config.worktree`，而且 binding 快照壞掉時寧可拒絕也不猜；錯了的話 hook 會出現在你的 checkout，或 agent 在不知道自己綁在哪時寫錯地方。

   ```bash
   AGEND_SHIM_DEMO_KEEP=1 ~/.cargo/bin/cargo run -q -p agend-shim --example shim_demo -- target/debug/agend | tail -6
   ```

   應該看到：`kept <tmp> …` 與一行 `cd …`、一行 `export PATH=…`。把這兩行貼到終端機執行，然後（`/usr/bin/git` 是你自己的 git，不經 shim）：

   ```bash
   /usr/bin/git -C "$AGEND_HOME/worktrees/t-1" config --show-origin core.hooksPath   # file:<tmp>/repo/.git/worktrees/t-1/config.worktree  <tmp>/home/hooks
   /usr/bin/git -C ../../../repo config core.hooksPath; echo "exit=$?"              # 什麼都沒印，exit=1（你的 ~/.gitconfig 沒設 core.hooksPath 時）
   git status                                                                        # On branch agend/t-1/fix（從 workspace 導向）
   echo broken > "$AGEND_HOME/bindings/dev-1.json"
   git commit --allow-empty -m x; echo "exit=$?"
   ```

   應該看到：`agend-shim: refused `git commit --allow-empty -m x``、`why: … binding snapshot <tmp>/home/bindings/dev-1.json is malformed: expected value at line 1 column 1`、`next step: run `agend status` …`，`exit=1`。接著：

   ```bash
   git -C "$AGEND_HOME/worktrees/t-1" status    # On branch agend/t-1/fix（在 checkout 裡讀取照常）
   /usr/bin/git -C "$AGEND_HOME/worktrees/t-1" commit --allow-empty -m x; echo "exit=$?"
   ```

   最後一個指令直接用真的 git、繞過 shim：應該看到 `agend-shim: refused `update HEAD (agend reference-transaction hook)``、`why: the agend hook cannot read your binding …`、`fatal: ref updates aborted by hook`、`exit=128`（hook 讀不到 binding 就拒絕）。最後照輸出的 `remove it afterwards: rm -rf <tmp>` 刪掉暫存目錄，並開新終端機（`export` 改了 PATH）。

   - [ ] 通過

## 待你追認

owner 睡覺時我自己做的決定；都可逆。每條打勾＝同意，不同意就寫在「驗收紀錄」備註。T7、T19、T21、T22、T23 是你已經決定的；T21 之後 T3、T4、T7、T8、T10、T11、T17 改寫，T14、T15、T16 由 T21 取代。

- [x] T19 威脅模型（**使用者已決定 2026-09-25**）：只防好意但會犯錯的 agent。見[威脅模型](#威脅模型)。
- [x] T21 hook 設計（**使用者已決定 2026-09-25**）：protected ref 改由 git hook 守，git 自己回報要改哪些 ref，shim 不再猜。
  - `reference-transaction`（`prepared`）與 `pre-push` 的規則見[範圍](#範圍)；hook 由 `agend` binary 以 argv[0] 分派（`$AGEND_HOME/hooks/<名稱>` 是 symlink）；檢查完串接 repo 原本的同名 hook。
  - 只裝在 agent worktree：repo config 開 `extensions.worktreeConfig=true`（開關），該 worktree 的 `config.worktree` 設 `core.hooksPath=$AGEND_HOME/hooks` 與 `gc.packRefs=false`；worktree git dir 的空檔 `agend-hooks-installed` 是「已安裝」標記；專案的 hook 目錄在 hook 執行時才查。API：`install_hooks`／`uninstall_hooks`，第 6 施工關 daemon 在綁定／釋放 worktree 時呼叫。
  - shim 只留 hook 做不到的：導向、快照、`checkout`／`switch` 離開 branch、`worktree`、kill 防護、擋 `core.hooksPath` 覆寫與 `push --no-verify`。刪掉：選項表、config key 白名單、push 目的地解析、symbolic ref 處理、push／fetch 的 refspec 解析。
  - hook 不看 `AGEND_SHIM_BYPASS`（它是硬保證）；讀不到 binding 時 fail closed。可信的呼叫者（daemon）在 agent worktree 裡跑 git 時用 `-c core.hooksPath=/dev/null`。
  - 我的解讀，請確認：`pre-push` 對所有 remote 都只放行自己綁定的 branch（比「只管 team remote」嚴、也簡單）：推到別的 remote 的自己 branch 放行；自己命名空間的其他 branch、tag 都不能 push。
- [x] T22 agent 不寫 git stash（**使用者已決定 2026-09-25**）：`refs/stash` 是 canonical checkout 與每個 worktree 共用的一個 ref。第 8 輪：agent 的 `stash pop` 拿走人在 canonical 的 WIP，`stash clear` 把人的 stash 清掉、`gc` 後救不回（快照只存 work tree）。兩層：shim 拒絕 `stash` 的寫入形式（`list`／`show` 照常），下一步是 `git commit -m "wip: …"` 存在自己綁定的 branch（一個 agent 一個 task 一個 branch，不會弄丟）；`reference-transaction` hook 拒絕 agent worktree 對 `refs/stash` 的任何更新（讀不到 binding 時也拒絕），任何拼法都擋。autostash 在衝突時也要寫 `refs/stash`，所以 agent 也不用 autostash（T23）。
- [x] T23 agent 不用 autostash（**使用者已決定 2026-09-25**）：autostash 套回修改衝突時，git 要把它存進 `refs/stash`，T22 的 hook 拒絕：git 印 `cannot store`，branch 已經被移動，修改留在衝突標記裡。shim 在 git 執行前拒絕：`pull`／`rebase`／`merge` 的 `--autostash`（含 git 接受的縮寫，例如 `--autost`）；`-c`／`--config-env`／`GIT_CONFIG_*` 設 `rebase.autoStash`／`merge.autoStash`（key 不分大小寫；寬鬆比對，`=false` 也拒絕）。下一步：`git add -A && git commit -m "wip: …"` 存在自己綁定的 branch，再帶 `--no-autostash` 重跑；`--no-autostash` 照常。
  - config 檔（repo、global、system）設了的，我選了**拒絕**，不自動加 `--no-autostash`：比較簡單，不改寫 argv，也不用在 `--continue` 這類不接受其他選項的動作上特判（自動加的版本多約 36 行，超過行數上限）。shim 用真的 git 在綁定的 worktree 問 `git config --get --type=bool`：`rebase` 問 `rebase.autoStash`、`merge` 問 `merge.autoStash`、`pull` 兩個都問。結果是 true，而且呼叫沒帶 `--no-autostash`、也不是 `--continue`／`--abort`／`--skip`／`--quit`／`--edit-todo`／`--show-current-patch`（要完整拼出）時就拒絕。代價：在這種 repo 裡，每次 `git pull --rebase` 都要帶 `--no-autostash`（拒絕訊息有寫）。
- [ ] T1 binding 快照格式：`$AGEND_HOME/bindings/<instance>.json`，JSON `{version: 1, instance, source_repo?, protected_refs?, binding?: {kind: "work", task_id, branch, worktree} | {kind: "review", task_id, head, worktree}}`；work branch 必須是 `agend/<task_id>/<slug>`。型別暫放 `agend_shim::binding::Snapshot`，建議第 10 施工關搬到 `agend_core::model`。
- [ ] T2 環境變數：`AGEND_HOME`、`AGEND_INSTANCE`（holder 注入；shim 與 hook 都讀，git 會把環境傳給 hook）；`AGEND_SHIM_BYPASS=1` 只跳過 shim（寫 audit）；`AGEND_SHIM_DEPTH` 防 PATH 迴圈。
- [ ] T3 建 branch：`git branch agend/<自己的 task-id>/<名稱>` 可以；其他名稱由 hook 拒絕。`checkout -b`／`switch -c` 一律由 shim 拒絕（會離開綁定的 branch），自己命名空間的也一樣；`branch -c/-C/-m/-M` 也一律拒絕（第 6 輪：hook 看不到新名稱），改用 `git branch <新> <舊>`。[pipeline.md](../architecture/pipeline.md#worktree-與-branch-生命週期) 寫「擋 `branch <new>`」，兩者有出入，請你決定。
- [ ] T4 不認得的子命令（git alias、外部 `git-*`）拒絕：擋 `git co main` 這類 alias（git 2.39 看不到 HEAD 換 branch）。`filter-branch`、`fast-import`、`replace` 不再特別拒絕：它們寫的 ref 由 hook 檢查。
- [ ] T5 team repo 用「目的地」認（沒有 hook 的 repo 由 shim 守）：跟 team 無關的 scratch repo 不管；remote 指向 team remote 或 canonical checkout 的 clone、team 的本機 remote 本身：讀取放行、寫入拒絕（`worktree` 只有 `list` 算讀取）；從任何 repo push 到 team remote 或 canonical 路徑拒絕。比對前套 `insteadOf`、正規化 URL；本機路徑照 git 找 repo 的順序（`/x/origin` = `/x/origin.git`），`file://` 不看主機名。包住 `$AGEND_HOME` 的 repo 不算。
- [ ] T6 導向：綁定時在 canonical checkout 跑的讀取與寫入都導向 worktree，保留子目錄（`--show-prefix`）；在 workspace（不在任何 repo）只有**沒帶** `-C`／`--git-dir`／`--work-tree`／`GIT_DIR`／`GIT_WORK_TREE` 的呼叫導向，跑在 worktree 頂層。worktree 沒有那個子目錄、或那個子目錄在 worktree 裡是 submodule／巢狀 repo（另一個 repo），讀取與寫入都拒絕（`route_dir_missing`）；在 canonical 的 `.git` 裡的寫入拒絕；別的 agent 的 worktree 裡寫入拒絕、讀取原地跑。第 10 輪：呼叫者用上面那些指定了地方、git 在那裡找不到 repo（例如 `git -C src/sbu checkout .` 打錯字；真 git 是 `fatal: cannot change to 'src/sbu'`），shim 原本丟掉 `-C`、在整個 worktree 跑；現在寫入拒絕（`no_repo_there`，理由照 git：`cannot change to '<dir>'` 或 `not a git repository`），讀取照打的跑、由 git 自己報錯。
- [x] T7 push（**使用者已決定 2026-09-25：放行解析到自己 branch 的 push**）：現在由 `pre-push` hook 實作，git 自己解析 `git push`／`git push origin`／`-u origin <branch>`／`push origin HEAD`，hook 看到真正的遠端 ref，是自己 branch 就放行。`--all`、`--mirror`、`--tags`、`--prune` 會列出別的 ref，所以被拒絕；刪除自己的 branch 拒絕；審查 binding 不能 push。
- [ ] T8 快照：範圍對齊 v1 agentic-git（見[範圍](#範圍)），新增 merge／rebase／pull／cherry-pick／revert／am，拿掉 `read-tree -u`；`stash drop|clear` 拿掉（快照只存 work tree，救不回 stash；stash 寫入改由 T22 拒絕）；每次都做（不只 dirty 時）；含未追蹤、不含 ignored 檔，所以 `clean -x`／`-X` 改成拒絕（第 8 輪：`clean -fdx` 刪掉 `.env`，快照裡沒有）；submodule 與巢狀 repo 只存 gitlink，所以會 recurse 的破壞性操作、`clean -ff`、`submodule foreach` 拒絕，worktree 裡的 submodule／巢狀 repo 改在它自己快照（第 9 輪）；ref `refs/agend/snapshots/<instance>/<id>`，id = `<unix 秒>-<pid>`（同一程序第二次加 `-<n>`）；快照用的 git 不跑 hook；shim 不清舊快照；快照失敗就拒絕該操作。
- [ ] T9 kill（未改）：`pkill`／`killall` 拒絕；`kill` 只接受 `kill [-SIGNAL | -s SIG | -n NUM | --signal SIG] [--] <pid>...`，pid 去空白後只接受 `1..=2147483647` 且最多 10 位數；`0`、負數、名字、job spec、執行檔是 `agend` 的 pid 拒絕。
- [ ] T10 audit：`$AGEND_HOME/audit/shim.jsonl`，記 shim 與 hook 的拒絕、bypass、快照；hook 的紀錄 argv 是 hook 名稱與參數，理由寫出哪個 ref；不輪替。
- [ ] T11 exit code：shim 拒絕 1、找不到真的工具 127；hook 拒絕時由 git 決定（`reference-transaction` 通常 128，`pre-push` 1）。
- [ ] T12 v1 的 gh 父程序例外拿掉：v2 由 daemon merge。
- [ ] T13 `GIT_DIR` 指向自己 worktree 的 git dir 時照常執行（git hook 會這樣呼叫）；寫入時 git 解析出的 work tree 必須是綁定的 worktree、`GIT_INDEX_FILE` 在它的 git dir 裡；需要導向的寫入帶 `--git-dir`／`--work-tree`／`GIT_*` 拒絕，不改寫。
- [ ] T17 `checkout` 只有一個參數、沒有 `--` 後面的路徑時，問 git 它是不是 commit：是就當切換（只有自己的 branch 放行），不是就當還原路徑並快照；只在 remote 有的 branch 會被 git 建成本機 branch，由 hook 拒絕。`checkout <x> --` 當切換。`-`／`@{-N}` 用 git 解析，回到自己的 branch 放行。
- [ ] T18 shell 內建的 `kill` 攔不到（含 `kill -9 -1`）；建議交給第 4 或第 6 施工關。
- [ ] T20 位置問真的 git（未改）：`<真 git> <呼叫者的全域選項> rev-parse --absolute-git-dir --git-common-dir --show-toplevel --show-prefix`（同 cwd、同 `GIT_*`）；需要 git 2.20 以上（`config --worktree`）。代價：每次多一個 git 程序（約 +8 ms）。

## 已知限制

shim 是安全帶，不是安全邊界。照[威脅模型](#威脅模型)，以下要刻意組出來、影響低，或 PATH shim／git 2.39 本來就看不到。

- 繞過 shim（`/usr/bin/git`、`AGEND_SHIM_BYPASS=1`）跑 `symbolic-ref` 寫入或 `reflog delete --updateref`／`reflog expire`：git 2.39 不把這些更新交給 hook（hook 在任何版本也不讀 symbolic-ref 的行），**main 會動**：`symbolic-ref refs/heads/main refs/heads/<agent 的 branch>` 讓 main（與 master、release）指向 agent 的 commit，canonical checkout 裡 main 的 `status` 變成一堆 staged 變更，人在那裡 commit 會落到 agent 的 branch、`git push origin main` 會推出 agent 的工作；`reflog delete --updateref main@{0}` 把 main 退回上一個 reflog 值；`reflog expire --expire=now --all` 清掉整個 repo 每個 branch 的 reflog，之後 `gc --prune=now` 會刪掉人 reset 掉的 commit（第 7 輪發現 1、2、4）。屬刻意繞過；agent 經 shim 時在 git 執行前就被拒絕。別人建的 symbolic ref（例如 `refs/heads/agend/t-1/x` → main）之後經它的寫入，hook 看到的是 main，照樣拒絕。
- `rebase <upstream> <別的 branch>` 會先把 HEAD 換到別的 branch（git 2.39 看不到）；別的 branch 最後的更新由 hook 拒絕，protected ref 不動，HEAD 要自己 `git checkout <自己的 branch>` 切回來。
- `git config` 寫入會進共用的 repo config，影響 canonical checkout（例如 `git config core.hooksPath .githooks`、`user.email`）；agent worktree 的 `config.worktree` 優先，agend hook 不受影響，而且照樣串接到新設的 hook 目錄（hook 執行時才查）。
- 刻意關掉 hook：`git config --worktree core.hooksPath …`／`--unset`、刪 `$AGEND_HOME/hooks` 或 `agend-hooks-installed`（刪標記時 shim 會改成拒絕寫入）、`-c include.path=…`、直接改 `.git/refs`。
- 在 agent 的 scratch repo 裡用 `-c remote.<x>.url=<team>` 這類特製目的地 push 到 team remote（第 2 輪發現 1）；scratch repo 沒有 hook。forge 端 branch protection 是最後一道。
- 在 agent worktree 裡沒有 agent 環境（`AGEND_*`）跑的 git（daemon、清掉環境的工具）會被 hook 拒絕寫 branch；可信的呼叫者用 `-c core.hooksPath=/dev/null`。
- `update-ref main-worktree/HEAD …` 這類改別的 worktree HEAD 的 plumbing 不擋。
- hook 只守 branch 與 protected ref：沒列進 `protected_refs` 的 tag、`refs/remotes/*`、`refs/replace/*`、`refs/notes/*` 本機可寫（ref 是整個 repo 共用的，canonical 也看得到）；推不出去（`pre-push` 只放行自己的 branch）。要保護 tag 就把 `refs/tags/*` 加進 `protected_refs`。
- agent worktree 不 pack refs（`gc.packRefs=false`）：git 2.39 的 `pack-refs` 把每個 ref（含 main）當成寫入回報給 hook，hook 分不出來；在 agent worktree 裡明確跑 `git pack-refs` 或 `git maintenance run --task=pack-refs` 會被拒絕（沒有東西被改）；拒絕訊息會點名 git 先回報的某個 ref（例如 `delete refs/heads/agend/<task>/…` 或 `update refs/heads/<別的 branch>`），那是 git 回報 pack 的方式，ref 沒有被改。`git gc`、`git maintenance run`（預設 task）照常。
- shell 內建的 `kill` 攔不到（T18）；git 自己啟動的程序（`rebase --exec`、`!` alias）不經過 shim（git 把自己的 exec-path 放在 PATH 最前面），但它們在 agent worktree 裡寫的 ref 一樣經過 hook。`submodule foreach` 同理，所以改成拒絕（第 9 輪）。
- 沒快照：`rm --cached`、`update-index`、`submodule update --force`、`submodule deinit -f`、`git rm -rf <submodule>`（submodule 裡未提交的修改會不見，快照只有 gitlink；第 8 輪發現 4、第 9 輪；很少人打，沒 `-f` 時 git 自己拒絕）、`read-tree -u`；merge／rebase／pull 每次都快照，快照 ref 會累積（留給 daemon 清）。
- submodule 與巢狀 repo（第 9 輪）：快照只存 gitlink。已擋的見[範圍](#範圍)；還剩：
  - 從 worktree 用 `git -C mod reset --hard` 跑時，印出的還原那行要在 `mod` 裡跑；在 worktree 裡跑會失敗（找不到 ref），不改任何東西。
  - 在 submodule 或巢狀 repo **裡面**：它自己的 submodule（第二層）的 recurse、`clean -x|-X`／`-ff`、`stash`、autostash 不擋，那個 repo 是 agent 自己的；它的快照一樣沒有 ignored 檔與下一層 repo 的內容。快照 ref 寫進那個 repo，`push --mirror` 會把它推出去。
  - `-c submodule.recurse=false` 也拒絕（寬鬆比對，同 autostash）；改用 `--no-recurse-submodules`。`merge`／`rebase`／`cherry-pick`／`revert`／`pull` 在 `submodule.recurse` 下不擋：git 不改 submodule 的 work tree（`pull` 遇到有修改的 submodule 時 git 自己中止），實測修改都還在。
  - canonical checkout 裡不是 submodule 的巢狀 repo（自己 clone 進去的）仍當外部 repo，寫入放行。
- 從 workspace（不在任何 repo）沒帶 `-C`／`--git-dir`／`--work-tree`／`GIT_DIR`／`GIT_WORK_TREE` 的呼叫導向時跑在 worktree 頂層：`.` 指 worktree 頂層（真 git 會說 `not a git repository`；破壞性的先快照）。帶了而 git 在那裡找不到 repo 時不導向（T6）。
- T5 認不出用 ssh `Host` 別名或不同網址指到同一個 team remote 的 clone。
- receive 端的 hook（`pre-receive`、`update`…）與 `push-to-checkout` 不裝也不串接：沒有人 push 進 agent worktree。
- `git bisect` 會暫時 detach HEAD，放行；`git bisect reset master`（或別的 protected branch）結束時把 agent 的 worktree 留在 master 上（git 2.39 看不到 HEAD 換 branch）。master 不會動：之後的 commit／reset／merge 由 hook 拒絕，要自己 `git checkout <自己的 branch>` 切回來（第 7 輪發現 5）。
- 繞過 shim（`/usr/bin/git`、`AGEND_SHIM_BYPASS=1`）跑 `branch -c/-C/-m/-M`：git 2.39 寫新名稱不經 ref transaction，hook 看不到，protected ref 可能被蓋掉（第 6 輪發現 1）；屬刻意繞過。hook 在改名中途拒絕時 git 不回滾（舊 branch 被刪、reflog 留在 `.git/logs/refs/.tmp-renamed-log`，之後整個 repo 的 `branch -c` 失敗到有人刪掉它）；agent 經 shim 時在 git 執行前就被拒絕，不會走到這一步。
- 繞過 shim（`/usr/bin/git`、`AGEND_SHIM_BYPASS=1`）跑 `stash drop`／`pop`／`apply`：`apply`／`pop` 不寫 ref，先把 stash（可能是人的）套進 agent 的 worktree；git 2.39 刪 stash 的 reflog 項目不經 ref transaction，hook 看不到：多筆時被 drop 的那筆直接消失（rc 0），只剩一筆時 git 接著刪 `refs/stash`，hook 拒絕（rc 128），但那筆已經從 `stash list` 消失（`refs/stash` 還指著它）。屬刻意繞過；經 shim 時在 git 執行前就被拒絕，`stash`／`clear`／`store`／`update-ref refs/stash` 繞過 shim 也由 hook 拒絕。
- autostash 經 shim 時在 git 執行前就被拒絕（T23）。繞過 shim（`/usr/bin/git`、`AGEND_SHIM_BYPASS=1`），或 autoStash 設在 shim 問不到的地方（`-c include.path=…` 帶進來的檔案）時，pull／rebase／merge 套回修改如果衝突，git 要把 autostash 存進 `refs/stash`，hook 會拒絕：git 印 `error: cannot store <sha>`，修改留在 worktree 的衝突標記裡，rebase／merge／pull 前的快照也有。沒衝突時不寫 `refs/stash`，照常跑。git 2.27 以前，merge 模式的 `pull --no-autostash` 會被 git 自己拒絕（`only valid with --rebase`）。
- `uninstall_hooks` 之後留下、但無害：空的 `config.worktree`、共用 config 的 `extensions.worktreeConfig=true`、`$AGEND_HOME/hooks`（別的 agent worktree 還在用）。

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-25 verifier 第 10 輪修正（draft PR #107）：`-C`／`--git-dir`／`--work-tree`／`GIT_DIR`／`GIT_WORK_TREE` 指到 git 找不到 repo 的地方時（`git -C src/sbu checkout .` 這類打錯字），shim 原本當成「不在 repo」丟掉 `-C`、在整個綁定的 worktree 跑（LOW，T6 原本寫錯）；現在寫入拒絕（`no_repo_there`，照 git 的意思寫 `cannot change to '<dir>'`／`not a git repository`），讀取照打的跑，workspace 導向只留給沒帶這些的呼叫；另外 canonical 裡沒初始化的 submodule（空的 `mod/`）導向時會落進 worktree 裡初始化好的 submodule（另一個 repo，快照只有 gitlink，修改會不見），改為 `route_dir_missing` 拒絕；T6、範圍、已知限制改正；回歸測試 `shim_route_scope.rs`（5 個命令 × worktree／子目錄／canonical／workspace 的 `-C typo`，加 `-C src/sbu`、`-C <不是 repo 的目錄>`、canonical 的 `-C mod`、`--git-dir`、`--work-tree`、`GIT_DIR`、`GIT_WORK_TREE`）與 `shim_submodule.rs` 在舊程式碼上都紅（25 案裡 23 案真的丟了修改；`--git-dir`／`--work-tree` 原本就以 `work_tree_retarget` 拒絕）；production 3,200 → 3,219 行

- 2026-09-25 verifier 第 9 輪修正（draft PR #107）：快照只存 submodule 與巢狀 repo 的 gitlink，範圍與 T8 寫清楚；有 `.gitmodules` 時會 recurse 的破壞性 `reset`／`checkout`／`restore`／`switch` 拒絕（下一步：先在 submodule 裡 commit，或加 `--no-recurse-submodules`）；`submodule foreach` 拒絕（git 用自己的 git 跑，不經 shim）；綁定 worktree 裡的 submodule／巢狀 repo 原本當外部 repo、沒快照，現在破壞性操作前在那個 repo 快照；canonical 與別的 worktree 的 submodule 寫入拒絕；`clean -ff` 拒絕（刪巢狀 repo），與 `clean -x|-X` 共用代碼 `clean_unsnapshotted`；剩下的列入已知限制；回歸測試 `shim_submodule.rs` 在舊程式碼上 4 個都紅；production 3,161 → 3,200 行
- 2026-09-25 使用者決定 agent 不用 autostash（T23，draft PR #107）：shim 在 git 執行前拒絕 `pull`／`rebase`／`merge` 的 `--autostash`（含縮寫）與 `-c rebase|merge.autoStash`；config 檔設了而沒帶 `--no-autostash` 也拒絕（選拒絕，不改寫 argv）；下一步 `git commit -m "wip: …"`，再帶 `--no-autostash` 重跑；`Probe::rev_parse` 改成通用的唯讀 `Probe::git`；回歸測試在舊程式碼上紅（`pull --rebase --autostash` 有衝突時 branch 被移動、`cannot store`）；production 3,131 → 3,161 行
- 2026-09-25 verifier 第 8 輪修正（draft PR #107）：agent 不寫 git stash（T22，使用者已決定）：shim 拒絕 stash 寫入、hook 拒絕 `refs/stash`，人在 canonical 的 stash 不再被 agent 的 `stash pop`／`clear` 拿走或清掉；T8／範圍原本誤寫 `stash drop|clear` 可由快照還原，已改正；`clean -x`／`-X` 拒絕（ignored 檔不在快照裡）；`checkout refs/heads/<自己的 branch>` 會 detach，改為拒絕；`submodule deinit -f`、繞過 shim 的 `stash drop`、autostash 衝突列入已知限制；production 3,084 → 3,131 行

- 2026-09-25 verifier 第 7 輪修正（draft PR #107）：shim 拒絕 `symbolic-ref` 寫入與 `reflog delete|expire`（git 2.39 改 ref 不經 hook；已知限制原本誤寫「main 不會動」，已改正）；`mv -f` 快照；長選項縮寫改成 git 接受的任何前綴（`checkout --de`／`switch --de` 會 detach）；`bisect reset <protected>` 列入已知限制；production 3,042 → 3,084 行
- 2026-09-25 verifier 第 6 輪修正（draft PR #107）：shim 拒絕 `branch -c/-C/-m/-M`（含 `--copy`／`--move`、縮寫、`-fm` 組合；git 2.39 寫新名稱不經 hook，MEDIUM），因此也不會再有 hook 中途拒絕留下的 `.tmp-renamed-log`；專案 hook 目錄改在 hook 執行時查（綁定後才設的 husky 也串接），`agend-hooks-chain` 改成空標記 `agend-hooks-installed`；`maintenance run --task=pack-refs` 與 uninstall 殘留列入已知限制；production 2,998 → 3,042 行

- 2026-09-25 依使用者決定改成 hook 設計（T21，draft PR #107）：protected ref 由 agent worktree 的 `reference-transaction`／`pre-push` hook 守、串接專案 hook、`install_hooks`／`uninstall_hooks`；刪掉選項表、config 白名單、push 目的地解析、symbolic ref 處理、refspec 解析；快照範圍對齊 v1；真 repo 測試搬到 `crates/agend/tests/shim_*.rs`（需要真的 binary），corpus 改成「被 shim 或 hook 拒絕」；production 5,743 → 2,998 行（`src` 下扣掉 `tests.rs`；各模組的 inline 測試搬進 `tests.rs`，只算程式碼則 4,907 → 2,998）
- 2026-09-25 verifier 第 5 輪修正（draft PR #107）：導向保留呼叫者的子目錄（`--show-prefix`），worktree 沒有的目錄、canonical 的 `.git` 裡、別的 worktree 裡的寫入拒絕（MEDIUM，repro E／B）；`rm -f` 快照；T7 依使用者決定放行解析到自己 branch 的 push；`-c sequence.editor` 放行；`checkout -`／`switch -` 解析 `@{-1}`；新增 `tests/route_scope.rs`、`tests/implicit_push.rs`

- 2026-09-25 verifier 第 4 輪修正（draft PR #107）：`parses_rev_parse_output` 不再靠 `$TMPDIR/x`（CI 紅的原因）；測試的 git 不讀 `~/.gitconfig`、不帶 agent 的 `AGEND_*`；team remote／clone 裡 `worktree` 除 `list` 都拒絕（T5）；新增 `tests/everyday.rs`：正常工作清單、symlink 與空格路徑 8 種拼法組合
- 2026-09-25 verifier 第 3 輪修正（draft PR #107）：位置改問真的 git（T20），刪掉自己寫的 repo 探索；gitfile `--git-dir`、`GIT_DIR` 與真正的 git dir 走同一套檢查；需要導向的寫入帶 `--git-dir`／`--work-tree` 改拒絕；`file://<host>/` 認得；team 本機 remote 裡的寫入拒絕；快照壞時的訊息改正確；`tests/location_matrix.rs` 336 + 168 案
- 2026-09-25 verifier 第 2 輪修正（draft PR #107）：寫入威脅模型（T19，使用者已決定）；T9 pid 限 `1..=i32::MAX`、最多 10 位數；T5 本機路徑照 git 補 `.git`；有 git dir 沒 work tree 時 cwd 必須是綁定的 worktree；`-c` 特製目的地與執行程式的 config key 列入已知限制；三項各有回歸測試
- 2026-09-25 verifier 第 1 輪修正（draft PR #107）：選項 deny-by-default、push 要明確目的地、fetch refmap 與 config key 檢查、symbolic ref、work tree 綁定、checkout 不猜路徑、T5 用目的地認 team repo、T9 kill 參數正規化；verifier 的 bypass corpus 變成回歸測試
- 2026-09-25 第 3 施工關實作：git 導向／拒絕、protected-ref、快照與還原、kill 防護、audit，`cargo xtask accept shim` demo（draft PR，branch `feat/gate-03-shim`）

## 下一步

```bash
~/.cargo/bin/cargo xtask accept shim
cat crates/agend-shim/README.md
```
