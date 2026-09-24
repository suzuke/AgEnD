# agend-shim 測試

> **TL;DR**
> - unit 測試測純決定（argv → 放行／導向／拒絕）；`tests/git_shim.rs` 在暫存 repo 裡跑真的 git。
> - 記住：shim 測試只在暫存目錄裡跑，絕不碰宿主 repo；binding 快照用 `binding::Snapshot` 寫（真的 producer，#1493）。
> - 下一步：testkit 的 `git_fixture` 有 repo builder 後，把 `tests/git_shim.rs` 的本地 `Fixture` 換掉。

## 怎麼跑

```bash
cargo test -p agend-shim
cargo test -p agend --test argv0_dispatch   # 真的 binary 以 `git` 名稱執行
cargo xtask accept shim                      # 以上 + demo
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `tests::dispatches_on_basename`、`other_names_are_not_the_shim` | argv[0] 分派 |
| `binding::tests::*` | 快照寫了讀得回來；版本、instance、相對路徑、branch 不在 `agend/<task>/`、缺 `source_repo` 都算壞掉；instance id 不能跳出目錄 |
| `classify::tests::*` | 全域選項解析；導向；讀取放行；未綁定／快照壞時拒絕寫入；`worktree`；branch 切換、建立、刪除、改名；protected ref（`update-ref`、`push .`、`push`、`branch -f`、`fetch`）；哪些操作要快照；不認得的子命令；`GIT_DIR` |
| `protected_ref::tests::*` | 內建與設定的 protected ref、glob、refspec 目的地 |
| `kill_guard::tests::*` | pkill／killall 拒絕、kill 拒絕 agend 程序與負數目標、`ps` 讀得到程序名 |
| `ctx::tests::real_tool_skips_links_to_the_shim` | 找真 git 時跳過指向 shim 的 symlink 與 hard link |
| `audit::tests::*` | audit 寫得進、讀得回；寫不進時不影響 |
| `snapshot::tests::*` | 還原命令的格式 |
| `tests/git_shim.rs` | 真 repo：從 workspace／canonical commit 會落在 task branch、main 不動；`checkout main` 拒絕並記 audit；worktree／branch 建立拒絕；未綁定、快照缺失或壞掉時拒絕寫入；protected ref 不動；`reset --hard` 快照後照訊息還原；`clean -fd`、`checkout -- .` 快照；bypass 記 audit；外部 repo 不管；hook 的 `GIT_DIR`；kill 防護對真的程序 |
| `examples/shim_demo.rs` | 驗收 demo：真的 `agend` binary 以 `git`／`kill`／`pkill` 名稱執行，每一步都檢查 |

## 用到的假實作

- `agend_testkit::tempdir::TempDir`
- `tests/git_shim.rs` 的本地 `Fixture`（canonical repo + `agend/t-1/fix` worktree + binding 快照）；之後換成 `agend_testkit::git_fixture`
- 假 holder：以 `agend` 為名的 symlink 指向 `/bin/sleep`

## 還沒測的

- [ ] Linux 上的 `ps -o comm=`（CI 的 ubuntu 會跑到；本機只驗過 macOS）
- [ ] 大型 working tree 的快照耗時
- [ ] 真 daemon 寫的快照（第 10 施工關）

## 下一步

```bash
cargo test -p agend-shim
```
