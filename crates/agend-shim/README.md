# agend-shim

> **TL;DR**
> - agent PATH 上的 `git`、`kill`、`killall`、`pkill` 防護；由 `agend` binary 依 argv[0] 分派進來。
> - 記住：**啟動要輕**：不建 runtime、不讀設定、不開 DB；只讀 daemon 寫的唯讀 binding 快照。
> - 下一步：第 3 施工關實作；目前 `run` 一律拒絕並說明未實作。

## 負責

- 依 argv[0] basename 判斷是哪個工具（`Tool::from_argv0`）
- 分類 git 呼叫：放行、導向綁定的 worktree、拒絕（擋 agent 自建 worktree／branch）
- protected-ref：擋 `update-ref`、`push .`、`branch -f` 對 main 的寫入
- 破壞性操作前快照
- kill 防護、audit 記錄

## 不負責

- 開 DB、連 daemon 做決定
- 寫 binding 快照
- 安全邊界：唯讀快照只是安全帶，同 uid 可 chmod

## 模組

| 模組 | 職責 |
|---|---|
| `lib (`Tool`, `run`)` | argv[0] 判斷與入口 |
| `binding` | 讀 binding 快照（D6，無 HMAC） |
| `classify` | git 呼叫分類 |
| `protected_ref` | protected-ref 檢查 |
| `snapshot` | 快照與還原 |
| `kill_guard` | kill 類防護 |
| `audit` | audit 記錄 |

## 依賴規則

- 一般依賴：`agend-core`
- 禁止：async runtime、SQLite、`agend-daemon`（`cargo xtask check-deps`）

## 入口

- `agend_shim::Tool::from_argv0`、`agend_shim::run`

## 下一步

```bash
cargo test -p agend-shim
```
