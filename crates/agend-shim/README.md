# agend-shim

> **TL;DR**
> - agent PATH 上的 `git`、`kill`、`killall`、`pkill` 防護，加上只裝在 agent worktree 的 git hooks；都由 `agend` binary 依 argv[0] 分派進來。
> - 記住：**protected ref 由 hook 守**（git 自己回報要改哪些 ref，不猜）；shim 只做 hook 做不到的：導向、快照、擋離開 branch、kill 防護。只防好意但會犯錯的 agent（見「威脅模型」）。
> - 下一步：`cargo xtask accept shim` 看 demo；行為規則看 [第 3 施工關頁](../../docs/gates/gate-03-shim.md)。

## 威脅模型

使用者已決定（2026-09-25）；完整版在[第 3 施工關頁](../../docs/gates/gate-03-shim.md#威脅模型)。

| | 內容 |
|---|---|
| 防 | 好意但會犯錯的 agent：打錯字、習慣性 `reset --hard`／`clean -fd`、push 錯 branch、在錯的目錄跑、kill 錯 pid |
| 不防 | 故意繞過的 agent：特製的 `-c remote.x.url=…`、直接跑 `/usr/bin/git`、改檔案、刻意關掉 hook |
| 硬保證在哪 | agent worktree 的 `reference-transaction`／`pre-push` hook（本 crate）＋ forge 端 branch protection |

## 負責

- 依 argv[0] basename 判斷是哪個工具或哪個 hook（`Tool::from_argv0`）
- git hook（`hook`）：
  - `reference-transaction` 的 `prepared` 階段拒絕：protected ref（main、master、快照列的）、自己 `agend/<task>/` 以外的 branch（別的 agent 的、新的 `feat/x`）、刪除自己綁定的 branch、`refs/stash`（canonical 與每個 worktree 共用）；讀不到 binding 時只放行 `refs/remotes/`，看不懂的輸入行也拒絕（fail closed）
  - `pre-push` 拒絕：遠端 ref 不是自己綁定的 branch（或是刪除它），不管推到哪個 remote
  - 每個 hook 檢查完都接著跑專案的同名 hook（同樣的參數與 stdin），所以專案的 hook 照常跑；hook 目錄在 hook 執行時才查：共用 config 的 `core.hooksPath`（綁定之後才設的也算，例如 husky），否則 `<common dir>/hooks`
  - `install_hooks`／`uninstall_hooks`：只裝在一個 agent worktree（見「hook 安裝在哪」）
- git shim：
  - 放行、導向綁定的 worktree、拒絕（附下一步命令）；位置問真的 git（`rev-parse`），不自己找 repo
  - 擋 hook 看不到的：`checkout`／`switch` 離開綁定的 branch、`branch -c/-C/-m/-M`（`--copy`／`--move`，含縮寫與 `-fm` 這類組合；git 2.39 寫新名稱不經 ref transaction）、`symbolic-ref` 寫入與 `reflog delete|expire`（同樣不經 ref transaction）、`git worktree`（`list` 除外）、不認得的子命令（alias）
  - 擋會跳過 hook 的：`-c`／`--config-env`／`GIT_CONFIG_*` 設 `core.hooksPath`、`push --no-verify`；綁定的 worktree 沒裝 hook 時拒絕寫入
  - 破壞性操作前快照（v1 agentic-git 的範圍；`refs/agend/snapshots/<instance>/<id>`）並印出還原命令；快照救不回的拒絕：`stash` 寫入與 autostash（`pull`／`rebase`／`merge` 的 `--autostash`、`-c rebase|merge.autoStash`，或 config 檔設了而沒帶 `--no-autostash`；都改用 `git commit -m "wip: …"`）、`clean -x|-X`
  - 沒有 hook 的 repo：team 的本機 remote 與 team repo 的 clone 不能寫、不能從別的 repo push 到 team repo（T5）
- kill 防護、audit 記錄（`$AGEND_HOME/audit/shim.jsonl`；hook 的拒絕也記）

## hook 安裝在哪

| 位置 | 內容 |
|---|---|
| `$AGEND_HOME/hooks/<hook 名稱>` | 每個 hook 名稱一個 symlink，指向 `agend` binary（`hook::NAMES`：client 端 hook；不含 receive 端與 `push-to-checkout`） |
| repo 的 `config`（共用） | 只有 `extensions.worktreeConfig=true`：開關，本身不改任何行為 |
| agent worktree 的 `config.worktree` | `core.hooksPath=$AGEND_HOME/hooks`，只有這個 worktree 生效；`gc.packRefs=false`（git 2.39 的 `pack-refs` 把每個 ref 都當成寫入回報給 hook，分不出真的寫入；ref 是共用的，由 canonical 那邊 pack） |
| agent worktree 的 git dir 裡 `agend-hooks-installed` | 空檔，「已安裝」的標記（沒有它 shim 拒絕寫入） |
| canonical checkout、其他 checkout、`~/.gitconfig` | 不動 |

daemon 在綁定 worktree 時呼叫 `install_hooks`、釋放時呼叫 `uninstall_hooks`（第 6 施工關）；第 3 施工關由測試與 demo 在暫存的 lab worktree 上呼叫。`uninstall_hooks` 之後留下、但無害的：空的 `config.worktree`、共用 config 的 `extensions.worktreeConfig=true`、`$AGEND_HOME/hooks`（別的 agent worktree 還在用）。

## 不負責

- 開 DB、連 daemon 做決定
- 寫 binding 快照（daemon 寫）、清舊快照
- 管 team repo 以外的 repo（agent 自己的 scratch repo 直接放行）
- git 自己啟動的程序（`rebase --exec`、`!` alias）與 shell 內建的 `kill`：攔不到，見[已知限制](../../docs/gates/gate-03-shim.md#已知限制)
- 安全邊界：唯讀快照只是安全帶，同 uid 可 chmod；`AGEND_SHIM_BYPASS=1` 跳過 shim（會記 audit），但不跳過 hook

## 輸入

| 來源 | 內容 |
|---|---|
| `AGEND_HOME`、`AGEND_INSTANCE` | 找 binding 快照 `$AGEND_HOME/bindings/<instance>.json`；shim 與 hook 都讀（git 把環境傳給 hook）。缺一個就當成不是 agent：shim 拒絕寫入，hook 拒絕 branch 與 protected ref |
| binding 快照 | `source_repo`、`protected_refs`、`binding`（work：task、branch、worktree；review：task、head、worktree） |
| `AGEND_SHIM_BYPASS=1` | shim 不檢查，直接執行真的工具（記 audit）；hook 照常 |
| `GIT_DIR`、`GIT_WORK_TREE`、`GIT_COMMON_DIR`、`GIT_INDEX_FILE`、`-C`、`--git-dir`、`--work-tree` | 原樣交給 `git rev-parse`，由 git 回答實際作用在哪個 repo、哪個 work tree |
| `GIT_CEILING_DIRECTORIES` | 問位置時保留；從 `$AGEND_HOME` 裡面跑時再加上 `$AGEND_HOME` |
| `-c`、`--config-env`、`GIT_CONFIG_KEY_<n>`、`GIT_CONFIG_PARAMETERS` | 只檢查有沒有設 `core.hooksPath` |
| `PATH` | 找真的工具：第一個不是 shim 自己（同 inode）的同名執行檔 |

## 判斷順序（git shim）

1. bypass → 記 audit，原樣執行
2. 解析全域選項與子命令
3. 位置：用呼叫者的全域選項、cwd、`GIT_*` 跑 `git rev-parse --absolute-git-dir --git-common-dir --show-toplevel --show-prefix`，分成綁定的 worktree／canonical checkout／同 repo 的其他 worktree／外部 repo／不在 repo／不知道（快照壞）；`--version`、`init`、`clone` 不問
4. 外部 repo → 放行，除非是 team 的本機 remote 或 team repo 的 clone（寫入拒絕），或 push 目的地是 team repo
5. 設 `core.hooksPath` 或 `push --no-verify` → 拒絕
6. `worktree`（`list` 除外）、不認得的子命令 → 拒絕
7. 讀取 → 綁定且在 workspace 或 canonical checkout 就導向，否則原地放行
8. 寫入 → 要有效快照與綁定；在綁定的 worktree 裡：git 回答的 work tree 是它、`GIT_INDEX_FILE` 在它的 git dir 裡；需要導向時：不是從別的 worktree、不在 git dir 裡、沒有 `GIT_*`、沒有 `--git-dir`／`--work-tree`、同一個子目錄在 worktree 裡存在；worktree 有 hook；`checkout`／`switch` 不離開綁定的 branch；不是 `branch` 複製或改名、`symbolic-ref` 寫入、`reflog delete|expire`、`stash` 寫入、autostash、`clean -x|-X`；破壞性操作先快照
9. 導向 = 真 git 加 `-C <worktree>/<prefix>`、拿掉呼叫者的 `-C`／`--git-dir`／`--work-tree`；之後 ref 的變更由 hook 檢查

## 模組

| 模組 | 職責 |
|---|---|
| `lib`（`Tool`、`run`、`plan`、`Refusal`） | argv[0] 判斷、入口、拒絕訊息格式 |
| `hook` | hook 的判斷、串接原本的 hook、`install_hooks`／`uninstall_hooks` |
| `ctx` | 環境輸入、找真的工具 |
| `binding` | 讀 binding 快照（D6，無 HMAC） |
| `location` | 把 git 的 `rev-parse` 答案對照 binding 分類 |
| `classify` | git argv 的決定（純函式；需要 git 回答的問題走 `Probe` trait） |
| `team` | 用 remote URL／本機路徑認 team repo |
| `protected_ref` | protected-ref 比對 |
| `snapshot` | 快照與還原命令 |
| `git` | 串起上面各步：問 git 位置、真的 `Probe`，產生要執行的命令 |
| `kill_guard` | kill 類防護 |
| `audit` | audit 記錄 |

## 依賴規則

- 一般依賴：`agend-core`、`serde`、`serde_json`（快照與 audit 是 JSON）
- dev 依賴：`agend-testkit`
- 禁止：async runtime、SQLite、`agend-daemon`（`cargo xtask check-deps`）

## 入口

- `agend_shim::Tool::from_argv0`、`agend_shim::run`（binary）、`agend_shim::plan`（測試）
- `agend_shim::install_hooks`、`agend_shim::uninstall_hooks`、`agend_shim::hooks_dir`（daemon、測試、demo）

## 細節

- 啟動成本：需要位置的呼叫多跑一次 `git rev-parse`；綁定 worktree 外的寫入再問一次錨點。hook 判斷時不跑 git（只讀快照）；串接前跑一次 `git config --get core.hooksPath`（`GIT_DIR` 設成 common dir）找專案的 hook 目錄。需要 git 2.20 以上（`config --worktree`）。
- 拒絕訊息三行：`agend-shim: refused `<命令>``、`agend-shim: why: …`、`agend-shim: next step: …`；shim 拒絕 exit 1，hook 拒絕時 git 失敗（`ref updates aborted by hook`／`failed to push some refs`）。
- 每個拒絕有固定的 code（`branch_switch`、`branch_copy`、`protected_ref`、`ref_not_yours`、`push_not_yours`、`hooks_skipped`、`no_binding`…），寫在 audit 裡。
- daemon 或人要在 agent worktree 裡跑 git 又沒有 agent 的環境時，hook 會拒絕 branch 寫入；可信的呼叫者用 `git -c core.hooksPath=/dev/null …`（shim 不在它們的 PATH 上）。
- 決定的理由與待追認事項見 [第 3 施工關頁](../../docs/gates/gate-03-shim.md#待你追認)。

## 下一步

```bash
cargo test -p agend-shim
cargo test -p agend
cargo xtask accept shim
```
