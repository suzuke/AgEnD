# 第 3 施工關：agend-shim（`shim`）

> **TL;DR**
> - agent PATH 上的 git 與 kill 防護：導向綁定的 worktree、拒絕危險操作並給下一步、破壞性操作前快照、kill 防護、audit。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」、看過「待你追認」與「已知限制」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：照「你親自驗收」跑一次，再逐條看「待你追認」。

## 狀態

**實作中**（2026-09-25，draft PR #107；verifier 第 1 輪的發現已修，待 verifier 重跑與你親自驗收）

## 範圍

- git 呼叫分類：放行、導向綁定的 worktree、拒絕（附下一步命令）
  - 拒絕：`git worktree`（`list` 除外）、切到其他 branch 或 protected branch、在自己的 `agend/<task-id>/` 以外建 branch、未綁定時的寫入、未綁定時改 canonical checkout、binding 快照缺失或壞掉時的寫入
- 結構性防護（verifier 第 1 輪後）：
  - 選項只認完整拼寫（deny-by-default，T14）
  - ref 目的地必須看得到：push 要明確的 `src:dst`（T7）、fetch／pull 的 refmap 含設定都要檢查（T16）、會改目的地的 config 不能設（T15）
  - symbolic ref：agent 不能建立；寫入前先追到真正的 ref 再檢查
  - 寫入只作用在綁定的 worktree：`--work-tree`、`GIT_WORK_TREE`、`GIT_INDEX_FILE` 指別處就拒絕（T13）
  - 離開綁定 branch 的旁門：DWIM checkout、`checkout <x> --`、`rebase <up> <other>`、`stash branch`、`rebase --update-refs`（T17）
  - team repo 用目的地認，不只用 cwd 認（T5）
- protected-ref 檢查：`update-ref`、`push`／`push .`、`fetch`／`pull` 的目的地、`branch -f`、`tag` 寫 main／master 或 binding 快照列的 ref，綁定的 agent 也一樣
- 破壞性操作前快照與還原：`reset --hard|--merge|--keep`、`clean`（非 dry-run）、`checkout -- <paths>`、`restore`、`switch --discard-changes`、`read-tree -u`
- `kill`／`killall`／`pkill` 防護（T9）、audit 記錄（拒絕、bypass、快照）
- binding 來源：daemon 寫的唯讀 binding 快照（D6，無 HMAC）；真 git 用 PATH 找、排除 shim 自己

## 自動驗收（完成定義）

在 repo 根目錄跑：

```bash
~/.cargo/bin/cargo fmt --all -- --check
~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
~/.cargo/bin/cargo test -p agend-shim
~/.cargo/bin/cargo test --workspace
~/.cargo/bin/cargo xtask check-deps
~/.cargo/bin/cargo xtask accept shim
```

- [x] `cargo test -p agend-shim` 通過：`47 passed`（unit）+ `11 passed`（`tests/bypass_corpus.rs`）+ `11 passed`（`tests/git_shim.rs`）
- [x] clippy 乾淨
- [x] `check-deps` 最後一行是 `check-deps: ok (2 rules, 8 crates checked for agend-testkit, agend-core metadata ok, no-std build ok)`
- [x] `cargo xtask accept shim` 最後兩行是 `shim demo: all checks passed`、`gate 3 (shim): checks passed`
- [x] `crates/agend-shim/README.md`／`TESTING.md` 已更新
- [x] verifier 第 1 輪的 bypass corpus 全部變成回歸測試（`tests/bypass_corpus.rs`），修正前全紅、修正後全綠
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。demo 在暫存目錄建 repo，不碰你的 repo；輸出裡的暫存路徑每次不同。demo 的 `kill` 只會打到假的記錄程式（`<tmp>/fakebin/kill`），不會送出任何訊號。

1. 跑 demo。

   **這步在驗什麼**：整個 shim 在真的 `agend` binary、真的 git repo 上跑得起來，每個行為都有自動檢查；錯了的話後面每一步看到的輸出都不可信。

   ```bash
   ~/.cargo/bin/cargo xtask accept shim
   ```

   應該看到：先跑 fmt、clippy、測試、check-deps，接著 `== shim demo ==`，然後 7 段 `-- route`、`-- refuse`、`-- snapshot`、`-- protected ref`、`-- structural (round 1)`、`-- kill guard`、`-- audit`。每個 `$ git …` 下面是 shim 真正印出的內容（`| …`）與 `exit N`，每個檢查一行 `ok: …`。最後兩行：`shim demo: all checks passed`、`gate 3 (shim): checks passed`。

   - [ ] 通過

2. 導向：在綁定狀態下 commit。

   **這步在驗什麼**：agent 在 workspace 或 canonical checkout 裡下的 git 指令會落到自己的 worktree；錯了的話 agent 的 commit 會進 canonical checkout 或 main。

   操作：同一次輸出，找 `-- route`

   應該看到：`$ git commit -q -m fix      (cwd: <tmp>/home/workspace/dev-1)` 之後 `ok: commit "fix" is on agend/t-1/fix`；在 canonical checkout（`cwd: <tmp>/repo`）跑的 `git add` 印出 `agend-shim: running in your bound worktree …`；最後 `ok: canonical main did not move`、`ok: canonical checkout is clean`。

   - [ ] 通過

3. 故意弄壞：`git checkout main`。

   **這步在驗什麼**：agent 不能離開綁定的 branch，也不能自己開 worktree；錯了的話 agent 會在錯的 branch 上工作，daemon 收不到它的成果。

   操作：同一次輸出，找 `-- refuse`

   應該看到三行訊息與 `exit 1`：

   ```text
   | agend-shim: refused `git checkout main`
   | agend-shim: why: switching to main is refused: you are bound to agend/t-1/fix (main is protected: only the daemon changes it)
   | agend-shim: next step: stay on agend/t-1/fix in <tmp>/home/worktrees/t-1. … To restore a file instead: git checkout -- <path>  or  git restore <path>
   ```

   接著 `ok: still on agend/t-1/fix`；`git worktree add ../mine` 也被拒絕（`only the daemon creates and removes worktrees`）。

   - [ ] 通過

4. `git reset --hard` 快照與還原。

   **這步在驗什麼**：破壞性指令前一定先快照，照訊息就能救回；錯了的話 agent 一個 `reset --hard` 就把沒存的工作永久弄丟。

   操作：同一次輸出，找 `-- snapshot`

   應該看到：reset 時印出 `agend-shim: snapshot <id> saved before `git reset --hard HEAD~1`` 與 `agend-shim: to undo: git reset --keep <sha> && git restore --source=refs/agend/snapshots/dev-1/<id> -- :/`；demo 照這行執行後 `ok: fix.txt is back, including the unsaved work`、`ok: the reset commits are back`。

   - [ ] 通過

5. protected ref：綁定的 agent 直接寫 main。

   **這步在驗什麼**：agent 不能直接改 main；錯了的話 agent 可以跳過 review 把 code 推進 main。

   操作：同一次輸出，找 `-- protected ref`

   應該看到：`git update-ref refs/heads/main HEAD`、`git push . HEAD:main`、`git branch -f main HEAD` 都是 `exit 1`，訊息含 `it is a protected ref and only the daemon changes it`，下一步是 `agend done`；最後 `ok: main did not move`。

   - [ ] 通過

6. 結構性防護：verifier 第 1 輪的四類繞法。

   **這步在驗什麼**：縮寫選項、由設定決定的推送目的地、換 work tree、symbolic ref 這四類繞法整類被擋，而不是只擋 verifier 用過的那幾個拼法；錯了的話 agent 用一個看起來無害的指令就能改到 main 或清掉 canonical checkout。

   操作：同一次輸出，找 `-- structural (round 1)`

   應該看到四個 `exit 1`：
   - `git reset --har`：`` `--har` looks like an abbreviation of `--hard` ``
   - `git push . HEAD`：`this push has no explicit destination`，下一步 `git push . HEAD:refs/heads/agend/t-1/fix`
   - `git --work-tree=<tmp>/repo clean -fd`：`is not your bound worktree`
   - `git symbolic-ref refs/heads/agend/t-1/alias refs/heads/main`：`a symbolic ref makes one ref name write another`

   最後 `ok: main did not move`、`ok: the canonical checkout's untracked file survived`。

   - [ ] 通過

7. 故意弄壞：`kill` 一個 holder、`pkill` 別人的程序。

   **這步在驗什麼**：agent 不能用外部的 `kill`／`pkill` 弄掉 daemon 或 holder；錯了的話一個 `pkill` 就讓整個團隊的 agent 一起掛掉。

   操作：同一次輸出，找 `-- kill guard`

   應該看到：`kill <pid>`（對一個名為 `agend` 的假 holder）被拒絕，理由 `is an agend process`；`pkill -f sleep 300` 被拒絕，下一步是 `pgrep -fl -- 'sleep 300'`；接著 `ok: holder still alive`；`kill <自己的 pid>` 是 `exit 0`，然後 `ok: only that call reached the (fake) real kill`。`-- audit` 段列出 11 筆 `refuse` 與 3 筆 `snapshot`。

   - [ ] 通過

8. 故意弄壞：binding 快照壞掉時，寫入被拒絕、讀取照常。自己動手。

   **這步在驗什麼**：binding 快照壞掉時 shim 寧可拒絕寫入也不猜；錯了的話 agent 會在不知道自己綁在哪裡的情況下寫錯地方。

   ```bash
   AGEND_SHIM_DEMO_KEEP=1 ~/.cargo/bin/cargo run -q -p agend-shim --example shim_demo -- target/debug/agend | tail -6
   ```

   應該看到：`kept <tmp> …` 與一行 `cd …`、一行 `export PATH=…`。把這兩行貼到終端機執行，然後：

   ```bash
   git status                                   # 應該看到 On branch agend/t-1/fix（從 workspace 導向 worktree）
   echo broken > "$AGEND_HOME/bindings/dev-1.json"
   git commit --allow-empty -m x; echo "exit=$?"
   ```

   應該看到：`agend-shim: refused `git commit --allow-empty -m x``、`why: … binding snapshot <tmp>/home/bindings/dev-1.json is malformed: expected value at line 1 column 1`、`next step: run `agend status` …`，`exit=1`。最後照輸出的 `remove it afterwards: rm -rf <tmp>` 刪掉暫存目錄，並開新終端機（`export` 改了 PATH）。

   - [ ] 通過

## 待你追認

owner 睡覺時我自己做的決定；都可逆。每條打勾＝同意，不同意就寫在「驗收紀錄」備註。T5、T7、T9、T13 在 verifier 第 1 輪後改過；T14–T18 是新的。

- [ ] T1 binding 快照格式：`$AGEND_HOME/bindings/<instance>.json`，JSON `{version: 1, instance, source_repo?, protected_refs?, binding?: {kind: "work", task_id, branch, worktree} | {kind: "review", task_id, head, worktree}}`；work branch 必須是 `agend/<task_id>/<slug>`（用 core 的 `task_id_of_branch` 驗）。型別暫放 `agend_shim::binding::Snapshot`：core 沒有 binding 型別，照指示不改 core。建議第 10 施工關搬到 `agend_core::model`，daemon 寫、shim 讀同一個型別。
- [ ] T2 環境變數：`AGEND_HOME`、`AGEND_INSTANCE`（holder 注入）；`AGEND_SHIM_BYPASS=1` 跳過檢查（git 與 kill 共用，寫 audit）；`AGEND_SHIM_DEPTH` 防 PATH 迴圈。不設 `AGEND_INSTANCE` 等於「不是 agent」，寫入一律拒絕。
- [ ] T3 建 branch：`git branch agend/<自己的 task-id>/<名稱>` 可以（照任務說明「agend 命名空間以外不行」）；`checkout -b`／`switch -c` 一律拒絕（會離開綁定的 branch）。[pipeline.md](../architecture/pipeline.md#worktree-與-branch-生命週期) 寫「擋 `branch <new>`」，兩者有出入，請你決定。
- [ ] T4 不認得的子命令（git alias、外部 `git-*`）拒絕；`filter-branch`、`filter-repo`、`replace`、`fast-import` 拒絕。擋的是 `git config alias.co checkout` 後 `git co main` 這類繞法。
- [ ] T5 team repo 用「目的地」認，不只用 cwd 認：
  - 跟 team repo 無關的 repo（agent 自己的 scratch repo）不管；`init`、`clone` 放行。
  - remote 指向 team remote 或 canonical checkout 的 repo（clone）：讀取放行，寫入一律拒絕（`team_clone`）。
  - 從任何 repo `push` 到 team remote 或 canonical 路徑：拒絕（`team_remote`）。
  - 比對前先套 `url.*.insteadOf`／`pushInsteadOf`，再正規化（`git@host:o/r.git` = `https://host/o/r`）；本機路徑比 git common dir。
  - 從 `$AGEND_HOME` 裡面往上找到、但包住 `$AGEND_HOME` 的 repo（例如 `$HOME` 的 dotfiles repo）不算；workspace 仍當「不在 repo」並導向。
- [ ] T6 讀取命令在未綁定、canonical checkout 裡都放行；綁定時在 worktree 外跑的讀取與寫入都導向 worktree，在 canonical／別的 worktree 時印一行 `running in your bound worktree …`。
- [ ] T7 `push` 只接受明確的 `src:dst`：`git push origin HEAD:refs/heads/<你的 branch>`。
  - 沒有 refspec、或 refspec 沒有 `:`（`git push`、`git push origin HEAD`）拒絕：git 會用 `remote.*.push`、`push.default`、`branch.*.merge` 決定目的地，shim 檢查不到。拒絕訊息給確切的替代指令。
  - 目的地只能是自己的 branch、自己的命名空間、不受保護的 tag；`HEAD` 當目的地拒絕。
  - `--all`、`--branches`、`--mirror`、`--prune`、`--tags`、`--follow-tags`、`--repo`、`--receive-pack`、`--exec` 拒絕；審查 binding 不能 push。
- [ ] T8 快照：每次破壞性操作都做（不只 dirty 時），含未追蹤、不含 ignored 檔（`clean -x` 刪掉的救不回）；ref `refs/agend/snapshots/<instance>/<id>`，id = `<unix 秒>-<pid>`；shim 不清舊快照（留給 daemon）；**快照失敗就拒絕**該操作（可用 bypass）。`clean` 只要不是 dry-run 都快照（`clean.requireForce=false` 時沒有 `-f` 也會刪）。
- [ ] T9 kill：
  - `pkill`／`killall` 一律拒絕（只放行 `--help` 等資訊旗標，沿用 v1）。
  - `kill` deny-by-default：只接受 `kill [-SIGNAL | -s SIG | -n NUM | --signal SIG] [--] <pid>...`。
  - pid 先去掉前後空白再判斷（`" 123"`、`+123`、`0123` 都是 123）。
  - 拒絕：`0`、負數（process group；`-1` 是自己的所有程序）、只有數字 signal 沒有 pid（`kill -1`）、名字（util-linux 的 `kill agend`）、job spec、其他選項。
  - 執行檔是 `agend` 的 pid 拒絕（daemon 與 holder 都是 `agend` binary），用 `ps -o comm=` 查；`ps` 查不到就放行。
  - shell 內建的 `kill` 攔不到，見 T18。
- [ ] T10 audit：單一檔案 `$AGEND_HOME/audit/shim.jsonl`，只記 refuse、bypass、snapshot；不輪替。
- [ ] T11 exit code：拒絕 1、找不到真的工具 127。
- [ ] T12 v1 的 gh 父程序例外（`gh pr merge --delete-branch` 後的 checkout）拿掉：v2 由 daemon merge，agent 不跑 `gh pr merge`。
- [ ] T13 `GIT_DIR` 指向自己 worktree 的 git dir 時照常執行（git hook 會這樣呼叫）；指向別處又需要導向的寫入拒絕（`git_env_retarget`）。寫入時 `--work-tree`、`GIT_WORK_TREE`、`GIT_INDEX_FILE` 必須是綁定的 worktree 自己的，否則拒絕（`work_tree_retarget`），不改成導向。`GIT_COMMON_DIR` 用來判斷位置，不能把 canonical 的 refs 偽裝成別的 repo。
- [ ] T14 選項 deny-by-default：結果取決於選項的子命令（`push`、`fetch`、`pull`、`checkout`、`switch`、`branch`、`tag`、`reset`、`clean`、`restore`、`update-ref`、`symbolic-ref`、`config`、`rebase`、`read-tree`、`remote add`）只接受表上的完整拼寫；照 git 的規則解析（`-bfoo` = `-b foo`、`-fd`、`--x=v`、`--end-of-options`）。縮寫（`--mirr`）或表上沒有的選項拒絕，訊息請 agent 寫完整選項名。代價：表是 git 2.39 的，新版 git 的新選項要加進 `specs.rs` 才能用。
- [ ] T15 config：`git config` 寫入、`-c`、`--config-env`、`GIT_CONFIG_COUNT`／`GIT_CONFIG_PARAMETERS` 只能設白名單的 key（`user.*`、`author.*`、`committer.*`、`core.editor`、`core.pager`、`pull.rebase`、`pull.ff`、`merge.conflictStyle`、`color.*`、`advice.*` 等，清單在 `config_keys.rs`）。其他（`remote.*`、`branch.*`、`push.*`、`core.worktree`、`core.hooksPath`、`alias.*`、`include.*`、`rebase.updateRefs`、`clean.requireForce`…）拒絕；改 section、`config --edit` 也拒絕。讀取不限制。
- [ ] T16 `fetch`／`pull`：命令列 refspec、`--refmap`、設定裡**所有** `remote.*.fetch` 的目的地只能是 `refs/remotes/`、自己的命名空間、或不強制的單一 tag。
  - git 在帶 refspec 時也會用設定的 refmap 順手更新，所以一律檢查設定。
  - 拒絕 `--update-head-ok`、`--prune-tags`、`--upload-pack`、`--stdin`、tag glob、強制的 tag。
  - 未綁定的 agent 可以 fetch 到 remote-tracking ref。
  - `rebase.updateRefs=true` 時，`git rebase`／`git pull` 要加 `--no-update-refs`／`--no-rebase`。
- [ ] T17 `checkout` 只有一個參數、沒有 `--` 後面的路徑時，一律當成切 branch，只有自己的 branch 放行；不再猜「這可能是檔案」。
  - 例外：`.`、`./x`、`../x` 不可能是 ref，當成還原路徑並快照。
  - `checkout <x> --` 也當成切換（git 本身就這樣解，會 DWIM 建 branch）。
  - 要還原檔案：`git checkout -- <path>` 或 `git restore <path>`；拒絕訊息會寫。
- [ ] T18 holder 防不了 agent 自己 shell 的內建 `kill`：PATH shim 攔不到 bash／zsh 內建的 `kill`，包括 `kill -9 -1`（殺掉同 uid 的所有程序；驗證時真的發生過，整個桌面當掉）。要保護 holder 需要別的層：holder 跑在不同 session／uid，或 daemon 重生 holder。建議交給第 4 施工關（holder）或第 6 施工關（daemon ↔ holder）決定；本施工關只記錄，不處理。

## 已知限制

shim 是安全帶，不是安全邊界（同 uid 的 agent 能改檔案、能直接跑 `/usr/bin/git`）。以下是明知道擋不到的：

- shell 內建的 `kill` 攔不到（T18）。
- git 自己啟動的程序不經過 shim：hooks、`rebase --exec`、`bisect run`、`submodule foreach`、`!` alias。git 會把自己的 exec-path 放在 PATH 最前面，裡面就有真的 `git`。
- 直接改檔案：`.git/config`、`~/.gitconfig`、`GIT_CONFIG_GLOBAL` 指到的檔案、`.git/refs/…`。已存在的設定只檢查 `remote.*.fetch` 與 `rebase.updateRefs`；其餘（例如 `push.followTags` 配 `--force`）不檢查。
- 沒快照：`git stash drop`、`git stash clear`（被丟掉的 stash 只能用 `git fsck` 找）、`merge --abort`、`rebase --abort`、`rm -f`、`update-index`、`submodule update --force`。
- T5 認不出用 ssh `Host` 別名或不同網址指到同一個 team remote 的 clone。
- 選項表是 git 2.39 的（T14）；新選項會被拒絕，要加進表。
- `rebase -i` 的 todo 裡手寫 `update-ref` 行，shim 看不到。
- `git bisect` 會暫時 detach HEAD（`git bisect reset` 回來）；沿用第一版，放行。

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-25 verifier 第 1 輪修正（draft PR #107）：選項 deny-by-default、push 要明確目的地、fetch refmap 與 config key 檢查、symbolic ref、work tree 綁定、checkout 不猜路徑、T5 用目的地認 team repo、T9 kill 參數正規化；verifier 的 bypass corpus 變成回歸測試
- 2026-09-25 第 3 施工關實作：git 導向／拒絕、protected-ref、快照與還原、kill 防護、audit，`cargo xtask accept shim` demo（draft PR，branch `feat/gate-03-shim`）

## 下一步

```bash
~/.cargo/bin/cargo xtask accept shim
cat crates/agend-shim/README.md
```
