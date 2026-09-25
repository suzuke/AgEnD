# agend-shim 測試

> **TL;DR**
> - 本 crate 只有 unit 測試（純決定：argv → 放行／導向／拒絕；hook 的 stdin 行 → 放行／拒絕）。真的 repo 上的測試放在 `crates/agend/tests/shim_*.rs`：git 以獨立程序執行 hook，只有 `agend` crate 的測試拿得到真的 `agend` binary（`CARGO_BIN_EXE_agend`）。
> - 記住：真 repo 測試只用 `git init`／`git clone` 建的暫存 repo，hook 只裝在 fixture 自己的 worktree；測試的 git 不讀 `~/.gitconfig`（`GIT_CONFIG_GLOBAL=/dev/null`）、不帶 agent 的 `AGEND_*`；kill 測試只打假的 kill 記錄程式，絕不送出訊號。
> - 下一步：testkit 的 `git_fixture` 有 repo builder 後，把 `crates/agend/tests/shim_common` 的 `Fixture` 換掉。

## 怎麼跑

```bash
cargo test -p agend-shim                              # unit
cargo test -p agend                                   # 真 repo：shim、hook、argv[0] 分派
cargo test -p agend --test shim_bypass_corpus         # verifier 第 1–8 輪的繞法清單
cargo test -p agend --test shim_location_matrix       # 拼法 × cwd × 命令矩陣（約 30 秒）
cargo test -p agend --test shim_hooks                 # hook 只在 agent worktree、串接、fail closed
TMPDIR=$(mktemp -d) cargo test --workspace            # 空的暫存目錄也要全過
cargo xtask accept shim                               # 以上 + demo
```

## 測試分類（unit，`src/**/tests.rs`）

| 測試 | 證明什麼 |
|---|---|
| `tests::*` | argv[0] 分派（含 hook 名稱；`update` 這類 receive 端名稱不是） |
| `hook::tests::*` | `reference-transaction`：protected ref（更新與刪除）、自己命名空間以外的 branch、刪自己的 branch、`refs/stash`（有沒有 binding 都）拒絕；HEAD、自己的 branch、remote-tracking、tag、快照 ref 放行；讀不到 binding 時只放行 `refs/remotes/`；只看 `prepared`；看不懂的輸入行拒絕。`pre-push`：只放行推到自己的 branch，main、別的 branch、tag、刪除、審查 binding 拒絕 |
| `classify::tests::*` | 全域選項解析；導向（保留子目錄、worktree 沒有的目錄與別的 worktree 拒絕）；讀取放行；未綁定／快照壞時拒絕寫入；沒裝 hook 拒絕寫入；`core.hooksPath` 與 `push --no-verify` 拒絕；`worktree`；`checkout`／`switch` 離開 branch（含縮寫與 `-bfoo`）、`branch -c/-C/-m/-M` 各種拼法拒絕（`-fm`、`--mo`、`--cop`；`-u<值>` 不誤判）、`symbolic-ref` 寫入與 `reflog delete|expire` 拒絕（讀取放行）、`-`／`@{-N}` 回到自己的 branch、`checkout refs/heads/<自己的>` 拒絕（會 detach）；`stash` 寫入與 `clean -x|-X`（含 `-fdx`、`-fX`，`-e<值>` 不誤判）拒絕、`stash list|show` 放行；work tree／index 必須是綁定的；快照範圍（含縮寫 `reset --har`、`mv -f`）；不認得的子命令；外部 repo 與 team repo；日常命令 |
| `binding::tests::*` | 快照寫了讀得回來；版本、instance、相對路徑、branch 不在 `agend/<task>/`、缺 `source_repo` 都算壞掉；instance id 不能跳出目錄 |
| `location::tests::*` | git 的答案怎麼分類；`rev-parse` 輸出（bare、相對的 common dir、`--show-prefix`；路徑建在自己的暫存目錄） |
| `team::tests::*` | URL 正規化、insteadOf；本機路徑照 git 補 `.git`；`file://<host>/` 不看主機名 |
| `protected_ref::tests::*` | 內建與設定的 protected ref、glob |
| `kill_guard::tests::*` | pkill／killall 拒絕、kill 拒絕 agend 程序、`0`／負數／名字／job spec；pid 正規化；超過 `i32::MAX` 或 10 位數拒絕（純函式，不送訊號） |
| `ctx::tests::*`、`audit::tests::*`、`snapshot::tests::*` | 找真 git 時跳過指向 shim 的連結；audit 寫讀；還原命令的格式 |

## 測試分類（真 repo，`crates/agend/tests/`）

| 測試 | 證明什麼 |
|---|---|
| `shim_hooks.rs` | canonical checkout 沒有 `core.hooksPath`、在那裡 commit 到 main 照常成功、共用 config 只多 `extensions.worktreeConfig`；專案的 `pre-commit`、`commit-msg`、`reference-transaction`（含 stdin）、`pre-push` 從 agent worktree 照常跑，失敗的專案 hook 讓命令失敗；之前設的 `core.hooksPath`（`.githooks`）被串接、重裝不串到自己；安裝之後才設的（husky 式 `.husky`）也被串接，拿掉後回到 `.git/hooks`；`install_hooks` 拒絕 canonical、`uninstall_hooks` 之後 shim 拒絕寫入（`hooks_missing`）；沒有 binding 時 hook fail closed；agent worktree 裡 `git gc` 照常（不 pack refs），canonical 照常 `pack-refs`；`rebase --update-refs` 動不了別的 branch |
| `shim_bypass_corpus.rs` | verifier 第 1–8 輪的犯錯類案例原樣重播，每條「被 shim 或 hook 拒絕」（或快照後執行、或無害），不變量：main／master／release 本機與 origin 都沒動、還在 `agend/t-1/fix`、沒多出 branch、canonical 與 worktree 的未提交工作還在或有快照。git 自己先拒絕的兩條（fetch 進 checkout 中的 main）標明。已知限制另列（只檢查 protected ref 與資料）。另含 T5（clone、scratch push 到 team URL）、第 2 輪自己的 git dir + canonical cwd、第 4 輪 team remote 裡的 `worktree`、第 6 輪 `branch -c/-C/-m/-M`（verifier 的原指令 × 4 個位置：shim 先拒絕，ref 與 reflog 不變、沒有 `.tmp-renamed-log`、人之後的 `branch -c` 照常）、第 7 輪 `symbolic-ref <name> <target>`、`reflog delete --updateref`、`reflog expire --all`（verifier 的原指令 × 4 個位置：shim 先拒絕，ref、reflog、canonical 的 status 不變）與 `mv -f` 蓋掉未提交修改（有快照、快照裡有修改）、第 8 輪 stash 寫入、`update-ref refs/stash`、`clean -fdx|-fX|-xdf`、`checkout refs/heads/<自己的>`、T9 kill 形式（假的 kill） |
| `shim_location_matrix.rs` | 第 3 輪：7 種拼法 × 有無 `--work-tree` × 4 個 cwd × 6 個保護動作（336 案，被 shim 或 hook 拒絕，或先快照）；4 個 cwd × 7 種拼法 × 6 步正常工作（168 步全過，專案 hook 有跑；存工作用 `commit -am wip`，不用 stash） |
| `shim_everyday.rs` | 第 4 輪：正常工作清單（含 `git push`、`push -u origin <branch>`、`checkout -`、`stash list`），hook 有跑；刻意拒絕的（含 `stash push`／`pop`）附下一步；symlink 與空格路徑 8 種拼法組合 |
| `shim_route_scope.rs` | 第 5 輪：導向保留子目錄（repro E／B）、別的 worktree 拒絕、worktree 沒有的目錄拒絕；第 8 輪 `clean -x|-X` 各種拼法從 worktree、canonical、子目錄都拒絕，`.env`、`target/` 還在，`clean -fd` 照常且不動 ignored 檔 |
| `shim_push_hook.rs` | T7：推到自己 branch 的各種寫法（含 push.default 各模式、推到別的 remote）都真的推上去；git 會推到 main／master／別的 branch／tag、刪自己的 branch 時 pre-push hook 拒絕，origin 沒動；git 自己拒絕的（沒有 upstream、upstream 不同名）照 git |
| `shim_git.rs` | 導向、`checkout main` 拒絕並記 audit、worktree／branch 建立、未綁定與快照壞、protected ref（hook 拒絕、記 audit）、`reset --hard` 快照後照訊息還原、第 8 輪人在 canonical 的 stash 撐過 agent 每種 stash 寫入（shim 拒絕；繞過 shim 的真 git 由 hook 拒絕，有沒有 agent 環境都一樣）、bypass 不跳過 hook、外部 repo、hook 形式的 `GIT_DIR` |
| `argv0_dispatch.rs` | 真的 binary 以 `git` 名稱執行進入 shim |
| `crates/agend-shim/examples/shim_demo.rs` | 驗收 demo：真的 `agend` binary 以 `git`／`kill`／`pkill` 執行、lab 的 worktree 裝好 hook，每一步都檢查 |

## 用到的假實作

- `agend_testkit::tempdir::TempDir`
- `crates/agend/tests/shim_common` 的 `Fixture`：bare `origin` + canonical repo + `agend/t-1/fix` worktree（裝好 hook）+ binding 快照；harness 自己的 git 以 `-c core.hooksPath=/dev/null` 執行（像 daemon），agent 的 git 經 shim 並帶 fixture 的 `AGEND_*`
- 假 holder：以 `agend` 為名的 symlink 指向 `/bin/sleep`
- 假的 `kill`／`pkill`／`killall`：只把 argv 寫進 log 的 shell script
- 假的 `Probe`：固定表回答 team、`rev-parse`、hook 是否已裝（`classify::tests`）

## 還沒測的

- [ ] git 2.46+ 回報 symbolic-ref 更新給 `reference-transaction`（hook 不讀這種行；本機 2.39，CI 用 runner 的新版 git 跑同一組測試）
- [ ] Linux 上的 `ps -o comm=`（CI 的 ubuntu 會跑到；本機只驗過 macOS）
- [ ] 大型 working tree 的快照耗時；每次 merge／rebase／pull 都快照的累積
- [ ] 真 daemon 呼叫 `install_hooks`／`uninstall_hooks`（第 6 施工關）與寫快照（第 10 施工關）

## 下一步

```bash
cargo test -p agend-shim && cargo test -p agend
```
