# agend-shim 測試

> **TL;DR**
> - unit 測試測純決定（argv → 放行／導向／拒絕）；`tests/` 在暫存 repo 裡跑真的 git；`tests/bypass_corpus.rs` 是 verifier 第 1 輪的繞法清單；`tests/location_matrix.rs` 是第 3 輪的拼法 × cwd × 命令矩陣。
> - 記住：git 測試只用 `git init`／`git clone` 建的暫存 repo（不複製 worktree），並設 `GIT_CEILING_DIRECTORIES`；kill 測試只打假的 kill 記錄程式，絕不送出訊號。
> - 下一步：testkit 的 `git_fixture` 有 repo builder 後，把 `tests/git_shim.rs` 的本地 `Fixture` 換掉。

## 怎麼跑

```bash
cargo test -p agend-shim
cargo test -p agend-shim --test bypass_corpus   # verifier 的繞法清單
cargo test -p agend-shim --test location_matrix # 拼法 × cwd × 命令矩陣（約 30 秒）
cargo test -p agend --test argv0_dispatch   # 真的 binary 以 `git` 名稱執行
cargo xtask accept shim                      # 以上 + demo
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `tests::dispatches_on_basename`、`other_names_are_not_the_shim` | argv[0] 分派 |
| `binding::tests::*` | 快照寫了讀得回來；版本、instance、相對路徑、branch 不在 `agend/<task>/`、缺 `source_repo` 都算壞掉；instance id 不能跳出目錄 |
| `location::tests::*` | git 的答案怎麼分類：綁定的 worktree（toplevel，或指定時比 git dir 與 common dir）、canonical、其他 worktree、`GIT_COMMON_DIR` 改標籤、外部 repo；`rev-parse` 輸出（bare 只有兩行、相對的 common dir） |
| `classify::tests::*` | 全域選項解析；綁定的 worktree 裡 git 回答的 work tree 必須是它（任何拼法）、index 在它的 git dir 裡；需要導向的寫入帶 `--git-dir`／`--work-tree` 拒絕、只帶 `-C` 導向；導向；讀取放行；未綁定／快照壞時拒絕寫入；`worktree`；branch 切換、建立、刪除、改名；protected ref；每一類繞法：縮寫選項、隱含的 push 目的地、fetch refmap、config key、symbolic ref、work tree／index、DWIM 與旁門、外部 repo 與 team repo（用假的 `Probe`） |
| `classify::opts::tests::*` | 選項解析照 git 的規則，縮寫與沒列的選項是錯誤 |
| `classify::specs::tests::*` | 選項表格式正確、沒有重複 |
| `config_keys::tests::*` | key 白名單；`GIT_CONFIG_PARAMETERS`／`GIT_CONFIG_COUNT` 兩種格式 |
| `team::tests::*` | URL 正規化（scp、ssh、https、埠號、`.git`）、insteadOf；本機路徑照 git 補 `.git`（`/x/origin` = `/x/origin.git`）；`file://localhost/`、`file://127.0.0.1/` 不看主機名 |
| `protected_ref::tests::*` | 內建與設定的 protected ref、glob、refspec 目的地 |
| `kill_guard::tests::*` | pkill／killall 拒絕、kill 拒絕 agend 程序、`0`／負數／名字／job spec；pid 去空白、`+`、前導 0 後再判斷；pid 超過 `i32::MAX`（`4294967295` 在 BSD `kill` 會變 `-1`）或超過 10 位數拒絕；`ps` 讀得到程序名（純函式，不送訊號） |
| `ctx::tests::real_tool_skips_links_to_the_shim` | 找真 git 時跳過指向 shim 的 symlink 與 hard link |
| `audit::tests::*` | audit 寫得進、讀得回；寫不進時不影響 |
| `snapshot::tests::*` | 還原命令的格式 |
| `tests/git_shim.rs` | 真 repo：從 workspace／canonical commit 會落在 task branch、main 不動；`checkout main` 拒絕並記 audit；worktree／branch 建立拒絕；未綁定、快照缺失或壞掉時拒絕寫入；protected ref 不動；`reset --hard` 快照後照訊息還原；`clean -fd`、`checkout -- .` 快照；bypass 記 audit；外部 repo 不管；hook 的 `GIT_DIR` |
| `tests/bypass_corpus.rs` | verifier 第 1 輪的指令原樣重播（有 bare `origin`、只在 remote 的 branch、先用真 git 做好的設定／symref），每條檢查不變量：main／master／release 本機與 origin 都沒動、還在 `agend/t-1/fix`、沒多出 branch、canonical 的未提交工作還在、worktree 的未提交工作還在或有快照；T5（clone、push 到 team URL 含不帶 `.git` 的路徑與 `file://`、workspace 在別的 repo 裡）；第 2 輪：自己的 git dir + canonical cwd 的 `clean`／`reset --hard`；T9 kill 形式（只打假的 kill） |
| `tests/location_matrix.rs` | 第 3 輪：7 種拼法（直接跑、`-C <worktree>/sub`、`-C <worktree>`、`--git-dir=<worktree>/.git`、`--git-dir=<真正的 git dir>`、`GIT_DIR` 兩種）× 有無 `--work-tree` × 4 個 cwd（worktree、canonical、workspace、worktree 子目錄）× 6 個保護動作（push main、update-ref main、刪 release、`checkout -b`、髒的 `reset --hard`、`clean -fd`）＝ 336 案：每案被拒絕，或破壞性操作先快照；main／master／release 本機與 origin 不動、canonical 與 workspace 的檔案都在、不被當成 team clone。另 4 個 cwd × 7 種指向綁定 worktree 的拼法 × 6 步正常工作（commit、`-c core.editor=true commit --amend`、push 自己的 branch、`rebase origin/main`、stash、stash pop）＝ 168 步全過、hooks 有跑；hook 形式的 `GIT_DIR` + `GIT_INDEX_FILE` 呼叫放行 |
| `examples/shim_demo.rs` | 驗收 demo：真的 `agend` binary 以 `git`／`kill`／`pkill` 名稱執行，每一步都檢查 |

## 用到的假實作

- `agend_testkit::tempdir::TempDir`
- `tests/common` 的 `Fixture`（bare `origin` + canonical repo + `agend/t-1/fix` worktree + binding 快照；全部 `git init`／`git clone`）；之後換成 `agend_testkit::git_fixture`
- 假 holder：以 `agend` 為名的 symlink 指向 `/bin/sleep`，只有測試自己 spawn 的
- 假的 `kill`／`pkill`／`killall`：只把 argv 寫進 log 的 shell script；shim 的「真工具」解析到它（`tests/bypass_corpus.rs`、demo）
- 假的 `Probe`：固定表回答 symref、config、team 問題（`classify::tests`）

## 還沒測的

- [ ] Linux 上的 `ps -o comm=`（CI 的 ubuntu 會跑到；本機只驗過 macOS）
- [ ] 大型 working tree 的快照耗時
- [ ] 真 daemon 寫的快照（第 10 施工關）
- [ ] 新版 git（2.46+ 的 `git config set` 等）的真機測試；選項表來自 2.39
- [ ] Linux util-linux 的 `kill <name>`：只有純函式測試，沒有在 Linux 真機上跑
- [ ] git 2.13–2.30 真機（位置用 `rev-parse --absolute-git-dir --git-common-dir`；本機 2.39、CI 用 runner 的新版 git）

## 下一步

```bash
cargo test -p agend-shim
```
