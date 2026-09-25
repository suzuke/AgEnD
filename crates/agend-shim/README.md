# agend-shim

> **TL;DR**
> - agent PATH 上的 `git`、`kill`、`killall`、`pkill` 防護；由 `agend` binary 依 argv[0] 分派進來。
> - 記住：**啟動要輕**：不建 tokio runtime、不讀設定、不開 DB、不連 daemon；只讀 daemon 寫的唯讀 binding 快照。只防好意但會犯錯的 agent（見「威脅模型」）。
> - 下一步：`cargo xtask accept shim` 看 demo；行為規則看 [第 3 施工關頁](../../docs/gates/gate-03-shim.md)。

## 威脅模型

使用者已決定（2026-09-25）；完整版在[第 3 施工關頁](../../docs/gates/gate-03-shim.md#威脅模型)。

| | 內容 |
|---|---|
| 防 | 好意但會犯錯的 agent：打錯字、習慣性 `reset --hard`／`clean -fd`、push 錯 branch、在錯的目錄跑、kill 錯 pid |
| 不防 | 故意繞過的 agent：特製的 `-c remote.x.url=…`、直接跑 `/usr/bin/git`、改檔案 |
| 硬保證在哪 | daemon 擁有的 `reference-transaction` hook（第 6 或第 10 施工關改成拒絕受保護 ref）＋ forge 端 branch protection |

## 負責

- 依 argv[0] basename 判斷是哪個工具（`Tool::from_argv0`）
- git：放行、導向綁定的 worktree、拒絕（附下一步命令）；位置問真的 git（`rev-parse`），不自己找 repo
- 選項只認完整拼寫（deny-by-default）；ref 目的地要看得到（push 要 `src:dst`、fetch refmap 含設定都檢查、不能設會改目的地的 config）；symbolic ref 先追到底再檢查
- protected-ref：`update-ref`、`push`、`push .`、`fetch`／`pull` 的目的地、`branch -f`、`tag` 寫 main／master 或快照列的 ref 一律拒絕
- 破壞性操作前快照（`refs/agend/snapshots/<instance>/<id>`）並印出還原命令
- kill 防護、audit 記錄（`$AGEND_HOME/audit/shim.jsonl`）

## 不負責

- 開 DB、連 daemon 做決定
- 寫 binding 快照（daemon 寫）、清舊快照
- 管 team repo 以外的 repo（agent 自己的 scratch repo 直接放行）；例外：team 的本機 remote 本身與 team repo 的 clone 不能寫、不能 push 到 team repo
- git 自己啟動的程序（hooks、`rebase --exec`）與 shell 內建的 `kill`：攔不到，見[已知限制](../../docs/gates/gate-03-shim.md#已知限制)
- 安全邊界：唯讀快照只是安全帶，同 uid 可 chmod；`AGEND_SHIM_BYPASS=1` 可跳過（會記 audit）
- 刻意組出來的繞法（例如 scratch repo 裡用 `-c remote.<x>.url=<team>` 特製 push 目的地）：見[已知限制](../../docs/gates/gate-03-shim.md#已知限制)

## 輸入

| 來源 | 內容 |
|---|---|
| `AGEND_HOME`、`AGEND_INSTANCE` | 找 binding 快照 `$AGEND_HOME/bindings/<instance>.json`；缺一個就當成不是 agent，寫入全拒 |
| binding 快照 | `source_repo`、`protected_refs`、`binding`（work：task、branch、worktree；review：task、head、worktree） |
| `AGEND_SHIM_BYPASS=1` | 不檢查，直接執行真的工具（記 audit） |
| `GIT_DIR`、`GIT_WORK_TREE`、`GIT_COMMON_DIR`、`GIT_INDEX_FILE`、`-C`、`--git-dir`、`--work-tree`、`-c` | 原樣交給 `git rev-parse`，由 git 回答實際作用在哪個 repo、哪個 work tree |
| `GIT_CEILING_DIRECTORIES` | 問位置時保留；從 `$AGEND_HOME` 裡面跑時再加上 `$AGEND_HOME` |
| `-c`、`--config-env`、`GIT_CONFIG_COUNT`／`GIT_CONFIG_KEY_<n>`、`GIT_CONFIG_PARAMETERS` | 這次呼叫設定的 config key；寫入類命令只允許白名單，讀取類命令不檢查 |
| `PATH` | 找真的工具：第一個不是 shim 自己（同 inode）的同名執行檔 |

## 判斷順序（git）

1. bypass → 記 audit，原樣執行
2. 解析 argv：全域選項（含 `-c` 的 key）、子命令
3. 位置：用呼叫者的全域選項、cwd、`GIT_*` 跑 `git rev-parse --absolute-git-dir --git-common-dir --show-toplevel`，照答案分成綁定的 worktree／canonical checkout／同 repo 的其他 worktree／外部 repo／不在 repo／不知道（快照壞）；`--version`、`init`、`clone` 不問
4. 外部 repo → 放行，除非是 team 的本機 remote 本身或 team repo 的 clone（寫入拒絕；`worktree` 只有 `list` 算讀取），或 push 目的地是 team repo（本機路徑照 git 補 `.git`，`file://` 不看主機名）
5. `worktree`（`list` 除外）、`filter-branch`、`fast-import`、不認得的子命令 → 拒絕
6. 結果取決於選項的子命令：用 `specs` 的表做完整拼寫解析，失敗就拒絕
7. 讀取 → 綁定且在 worktree 外就導向，否則放行
8. 寫入 → config key 白名單（`-c`、env）；`fetch` 檢查目的地後像讀取一樣導向；其餘要有效快照與綁定；在綁定的 worktree 裡：git 回答的 work tree 是綁定的 worktree、`GIT_INDEX_FILE` 在它的 git dir 裡；需要導向時：沒有 `GIT_*`、沒有 `--git-dir`／`--work-tree`（有就拒絕，不改寫）；自己的 branch 不是 symref；逐命令檢查；破壞性操作先快照
9. 導向 = 真 git 加 `-C <worktree>`、拿掉呼叫者的 `-C`／`--git-dir`／`--work-tree`

## 模組

| 模組 | 職責 |
|---|---|
| `lib`（`Tool`、`run`、`plan`、`Refusal`） | argv[0] 判斷、入口、拒絕訊息格式 |
| `ctx` | 環境輸入、找真的工具 |
| `binding` | 讀 binding 快照（D6，無 HMAC） |
| `location` | 把 git 的 `rev-parse` 答案對照 binding 分類（純函式；錨點透過 `Anchors` 問 git） |
| `classify` | git argv 解析與決定（純函式；需要 git 回答的問題走 `Probe` trait） |
| `classify::opts` | 完整拼寫的選項解析（照 git parse-options 的規則，不收縮寫） |
| `classify::specs` | 各子命令的選項表 |
| `classify::commands` | 各命令的檢查：checkout／switch、branch、tag、rebase、stash、config、remote、哪些操作要快照 |
| `classify::refs` | ref 目的地：push、fetch／pull、update-ref、symbolic-ref |
| `config_keys` | agent 可以設的 config key 白名單、讀 `GIT_CONFIG_*` |
| `team` | 用 remote URL／本機路徑認 team repo（insteadOf、URL 正規化、git `enter_repo` 的本機路徑規則） |
| `protected_ref` | protected-ref 比對 |
| `snapshot` | 快照與還原命令 |
| `git` | 串起上面各步：問 git 位置（`resolve`、`Anchors`）、真的 `Probe`（symref、config、team），產生要執行的命令 |
| `kill_guard` | kill 類防護 |
| `audit` | audit 記錄 |

## 依賴規則

- 一般依賴：`agend-core`、`serde`、`serde_json`（快照與 audit 是 JSON）
- dev 依賴：`agend-testkit`
- 禁止：async runtime、SQLite、`agend-daemon`（`cargo xtask check-deps`）

## 入口

- `agend_shim::Tool::from_argv0`、`agend_shim::run`（binary）、`agend_shim::plan`（測試）

## 細節

- 啟動成本：需要位置的呼叫多跑一次 `git rev-parse`（canonical 以外、有指定 git dir 時再問一次錨點）。macOS 經 `/usr/bin/git` 實測 guarded 呼叫約 9 ms → 16–18 ms；`git --version` 約 8 ms 不變。需要 git 2.13 以上。
- 拒絕訊息三行：`agend-shim: refused `<命令>``、`agend-shim: why: …`、`agend-shim: next step: …`；exit 1。
- 每個拒絕有固定的 code（`branch_switch`、`protected_ref`、`worktree_managed`、`no_binding`…），寫在 audit 裡。
- 決定的理由與待追認事項見 [第 3 施工關頁](../../docs/gates/gate-03-shim.md#待你追認)。

## 下一步

```bash
cargo test -p agend-shim
cargo xtask accept shim
```
