# 第 3 施工關：agend-shim（`shim`）

> **TL;DR**
> - agent PATH 上的 git 與 kill 防護：導向綁定的 worktree、拒絕危險操作並給下一步、破壞性操作前快照、kill 防護、audit。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」、看過「待你追認」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：照「你親自驗收」跑一次，再逐條看「待你追認」。

## 狀態

**實作中**（2026-09-25，draft PR；待 fresh-context verifier 與你親自驗收）

## 範圍

- git 呼叫分類：放行、導向綁定的 worktree、拒絕（附下一步命令）
  - 拒絕：`git worktree`（`list` 除外）、切到其他 branch 或 protected branch、在自己的 `agend/<task-id>/` 以外建 branch、未綁定時的寫入、未綁定時改 canonical checkout、binding 快照缺失或壞掉時的寫入
- protected-ref 檢查：`update-ref`、`push`／`push .`、`fetch <src>:<dst>`、`branch -f`、`tag` 寫 main／master 或 binding 快照列的 ref，綁定的 agent 也一樣
- 破壞性操作前快照與還原：`reset --hard|--merge|--keep`、`clean -f`、`checkout -- <paths>`／`checkout <path>`、`restore`、`switch --discard-changes`
- `kill`／`killall`／`pkill` 防護、audit 記錄（拒絕、bypass、快照）
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

- [x] `cargo test -p agend-shim` 單獨通過：`33 passed`（unit）+ `12 passed`（`tests/git_shim.rs`）
- [x] clippy 乾淨
- [x] `check-deps` 最後一行是 `check-deps: ok (2 rules, 8 crates checked for agend-testkit, agend-core metadata ok, no-std build ok)`
- [x] `cargo xtask accept shim` 最後兩行是 `shim demo: all checks passed`、`gate 3 (shim): checks passed`
- [x] `crates/agend-shim/README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。demo 在暫存目錄建 repo，不碰你的 repo；輸出裡的暫存路徑每次不同。

1. 跑 demo。

   ```bash
   ~/.cargo/bin/cargo xtask accept shim
   ```

   應該看到：先跑 fmt、clippy、測試、check-deps，接著 `== shim demo ==`，然後 6 段 `-- route`、`-- refuse`、`-- snapshot`、`-- protected ref`、`-- kill guard`、`-- audit`。每個 `$ git …` 下面是 shim 真正印出的內容（`| …`）與 `exit N`，每個檢查一行 `ok: …`。最後兩行：`shim demo: all checks passed`、`gate 3 (shim): checks passed`。

   - [ ] 通過

2. 導向：在綁定狀態下 commit。

   操作：同一次輸出，找 `-- route`

   應該看到：`$ git commit -q -m fix      (cwd: <tmp>/home/workspace/dev-1)` 之後 `ok: commit "fix" is on agend/t-1/fix`；在 canonical checkout（`cwd: <tmp>/repo`）跑的 `git add` 印出 `agend-shim: running in your bound worktree …`；最後 `ok: canonical main did not move`、`ok: canonical checkout is clean`。

   - [ ] 通過

3. 故意弄壞：`git checkout main`。

   操作：同一次輸出，找 `-- refuse`

   應該看到三行訊息與 `exit 1`：

   ```text
   | agend-shim: refused `git checkout main`
   | agend-shim: why: switching to main is refused: you are bound to agend/t-1/fix (main is protected: only the daemon changes it)
   | agend-shim: next step: stay on agend/t-1/fix in <tmp>/home/worktrees/t-1. … For other work: agend task create "<title>"
   ```

   接著 `ok: still on agend/t-1/fix`；`git worktree add ../mine` 也被拒絕（`only the daemon creates and removes worktrees`）。

   - [ ] 通過

4. `git reset --hard` 快照與還原。

   操作：同一次輸出，找 `-- snapshot`

   應該看到：reset 時印出 `agend-shim: snapshot <id> saved before `git reset --hard HEAD~1`` 與 `agend-shim: to undo: git reset --keep <sha> && git restore --source=refs/agend/snapshots/dev-1/<id> -- :/`；demo 照這行執行後 `ok: fix.txt is back, including the unsaved work`、`ok: the reset commits are back`。

   - [ ] 通過

5. protected ref：綁定的 agent 直接寫 main。

   操作：同一次輸出，找 `-- protected ref`

   應該看到：`git update-ref refs/heads/main HEAD`、`git push . HEAD:main`、`git branch -f main HEAD` 都是 `exit 1`，訊息含 `it is a protected ref and only the daemon changes it`，下一步是 `agend done`；最後 `ok: main did not move`。

   - [ ] 通過

6. 故意弄壞：`kill` 一個 holder、`pkill` 別人的程序。

   操作：同一次輸出，找 `-- kill guard`

   應該看到：`kill <pid>`（對一個名為 `agend` 的假 holder）被拒絕，理由 `is an agend process`；`pkill -f sleep 300` 被拒絕，下一步是 `pgrep -fl -- 'sleep 300'`；接著 `ok: holder still alive`；最後 `kill <自己的 pid>` 是 `exit 0`。`-- audit` 段列出 7 筆 `refuse` 與 3 筆 `snapshot`。

   - [ ] 通過

7. 故意弄壞：binding 快照壞掉時，寫入被拒絕、讀取照常。自己動手。

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

owner 睡覺時我自己做的決定；都可逆。每條打勾＝同意，不同意就寫在「驗收紀錄」備註。

- [ ] T1 binding 快照格式：`$AGEND_HOME/bindings/<instance>.json`，JSON `{version: 1, instance, source_repo?, protected_refs?, binding?: {kind: "work", task_id, branch, worktree} | {kind: "review", task_id, head, worktree}}`；work branch 必須是 `agend/<task_id>/<slug>`（用 core 的 `task_id_of_branch` 驗）。型別暫放 `agend_shim::binding::Snapshot`：core 沒有 binding 型別，照指示不改 core。建議第 10 施工關搬到 `agend_core::model`，daemon 寫、shim 讀同一個型別。
- [ ] T2 環境變數：`AGEND_HOME`、`AGEND_INSTANCE`（holder 注入）；`AGEND_SHIM_BYPASS=1` 跳過檢查（git 與 kill 共用，寫 audit）；`AGEND_SHIM_DEPTH` 防 PATH 迴圈。不設 `AGEND_INSTANCE` 等於「不是 agent」，寫入一律拒絕。
- [ ] T3 建 branch：`git branch agend/<自己的 task-id>/<名稱>` 可以（照任務說明「agend 命名空間以外不行」）；`checkout -b`／`switch -c` 一律拒絕（會離開綁定的 branch）。[pipeline.md](../architecture/pipeline.md#worktree-與-branch-生命週期) 寫「擋 `branch <new>`」，兩者有出入，請你決定。
- [ ] T4 不認得的子命令（git alias、外部 `git-*`）拒絕；`filter-branch`、`filter-repo`、`replace` 拒絕。擋的是 `git config alias.co checkout` 後 `git co main` 這類繞法。
- [ ] T5 不是 team repo 的 repo（agent 自己的 scratch repo）完全不管；`init`、`clone` 一律放行。
- [ ] T6 讀取命令在未綁定、canonical checkout 裡都放行；綁定時在 worktree 外跑的讀取與寫入都導向 worktree，在 canonical／別的 worktree 時印一行 `running in your bound worktree …`。
- [ ] T7 `push`：只能推自己的 branch、自己的命名空間與 tag；`--all`／`--branches`／`--mirror`／`--prune` 拒絕；審查 binding 不能 push。
- [ ] T8 快照：每次破壞性操作都做（不只 dirty 時），含未追蹤、不含 ignored 檔（`clean -x` 刪掉的救不回）；ref `refs/agend/snapshots/<instance>/<id>`，id = `<unix 秒>-<pid>`；shim 不清舊快照（留給 daemon）；**快照失敗就拒絕**該操作（可用 bypass）。
- [ ] T9 kill：`pkill`／`killall` 一律拒絕（只放行 `--help` 等資訊旗標，沿用 v1）；`kill` 拒絕負數目標與執行檔名是 `agend` 的 pid（daemon 與 holder 都是 `agend` binary），用 `ps -o comm=` 查；`ps` 查不到就放行。shell 內建的 `kill` 攔不到。
- [ ] T10 audit：單一檔案 `$AGEND_HOME/audit/shim.jsonl`，只記 refuse、bypass、snapshot；不輪替。
- [ ] T11 exit code：拒絕 1、找不到真的工具 127。
- [ ] T12 v1 的 gh 父程序例外（`gh pr merge --delete-branch` 後的 checkout）拿掉：v2 由 daemon merge，agent 不跑 `gh pr merge`。
- [ ] T13 `GIT_DIR` 指向自己 worktree 的 git dir 時照常執行（git hook 會這樣呼叫）；指向別處又需要導向的寫入拒絕（`git_env_retarget`）。

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-25 第 3 施工關實作：git 導向／拒絕、protected-ref、快照與還原、kill 防護、audit，`cargo xtask accept shim` demo（draft PR，branch `feat/gate-03-shim`）

## 下一步

```bash
~/.cargo/bin/cargo xtask accept shim
cat crates/agend-shim/README.md
```
