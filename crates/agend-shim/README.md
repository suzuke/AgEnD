# agend-shim

> **TL;DR**
> - agent PATH 上的 `git`、`kill`、`killall`、`pkill` 防護；由 `agend` binary 依 argv[0] 分派進來。
> - 記住：**啟動要輕**：不建 tokio runtime、不讀設定、不開 DB、不連 daemon；只讀 daemon 寫的唯讀 binding 快照。
> - 下一步：`cargo xtask accept shim` 看 demo；行為規則看 [第 3 施工關頁](../../docs/gates/gate-03-shim.md)。

## 負責

- 依 argv[0] basename 判斷是哪個工具（`Tool::from_argv0`）
- git：放行、導向綁定的 worktree、拒絕（附下一步命令）
- protected-ref：`update-ref`、`push`、`push .`、`fetch <src>:<dst>`、`branch -f`、`tag` 寫 main／master 或快照列的 ref 一律拒絕
- 破壞性操作前快照（`refs/agend/snapshots/<instance>/<id>`）並印出還原命令
- kill 防護、audit 記錄（`$AGEND_HOME/audit/shim.jsonl`）

## 不負責

- 開 DB、連 daemon 做決定
- 寫 binding 快照（daemon 寫）、清舊快照
- 管 team repo 以外的 repo（agent 自己的 scratch repo 直接放行）
- 安全邊界：唯讀快照只是安全帶，同 uid 可 chmod；`AGEND_SHIM_BYPASS=1` 可跳過（會記 audit）

## 輸入

| 來源 | 內容 |
|---|---|
| `AGEND_HOME`、`AGEND_INSTANCE` | 找 binding 快照 `$AGEND_HOME/bindings/<instance>.json`；缺一個就當成不是 agent，寫入全拒 |
| binding 快照 | `source_repo`、`protected_refs`、`binding`（work：task、branch、worktree；review：task、head、worktree） |
| `AGEND_SHIM_BYPASS=1` | 不檢查，直接執行真的工具（記 audit） |
| `GIT_DIR`、`GIT_WORK_TREE`、`GIT_COMMON_DIR`、`-C`、`--git-dir`、`--work-tree` | 判斷 git 實際作用在哪個 repo |
| `PATH` | 找真的工具：第一個不是 shim 自己（同 inode）的同名執行檔 |

## 判斷順序（git）

1. bypass → 記 audit，原樣執行
2. 解析 argv：全域選項、子命令
3. 位置：綁定的 worktree／canonical checkout／同 repo 的其他 worktree／外部 repo／不在 repo／不知道（快照壞）
4. 外部 repo → 放行；`worktree`（`list` 除外）、`filter-branch`、不認得的子命令 → 拒絕
5. 讀取 → 綁定且在 worktree 外就導向，否則放行
6. 寫入 → 需要有效快照與綁定；逐命令檢查 branch 切換、建立、protected ref；破壞性操作先快照
7. 導向 = 真 git 加 `-C <worktree>`、拿掉呼叫者的 `-C`／`--git-dir`／`--work-tree`

## 模組

| 模組 | 職責 |
|---|---|
| `lib`（`Tool`、`run`、`plan`、`Refusal`） | argv[0] 判斷、入口、拒絕訊息格式 |
| `ctx` | 環境輸入、找真的工具 |
| `binding` | 讀 binding 快照（D6，無 HMAC） |
| `location` | 只看檔案系統判斷 git 作用的位置 |
| `classify` | git argv 解析與決定（純函式） |
| `classify::commands` | 各命令的檢查：branch 切換／建立、protected ref、哪些操作要快照 |
| `protected_ref` | protected-ref 比對 |
| `snapshot` | 快照與還原命令 |
| `git` | 串起上面各步，產生要執行的命令 |
| `kill_guard` | kill 類防護 |
| `audit` | audit 記錄 |

## 依賴規則

- 一般依賴：`agend-core`、`serde`、`serde_json`（快照與 audit 是 JSON）
- dev 依賴：`agend-testkit`
- 禁止：async runtime、SQLite、`agend-daemon`（`cargo xtask check-deps`）

## 入口

- `agend_shim::Tool::from_argv0`、`agend_shim::run`（binary）、`agend_shim::plan`（測試）

## 細節

- 拒絕訊息三行：`agend-shim: refused `<命令>``、`agend-shim: why: …`、`agend-shim: next step: …`；exit 1。
- 每個拒絕有固定的 code（`branch_switch`、`protected_ref`、`worktree_managed`、`no_binding`…），寫在 audit 裡。
- 決定的理由與待追認事項見 [第 3 施工關頁](../../docs/gates/gate-03-shim.md#待你追認)。

## 下一步

```bash
cargo test -p agend-shim
cargo xtask accept shim
```
