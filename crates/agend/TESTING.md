# agend 測試

> **TL;DR**
> - 整合測試直接執行建好的 binary；shim 與 git hook 的真 repo 測試（`tests/shim_*.rs`）也在這裡，因為 git 以獨立程序執行 hook，需要真的 `agend` binary。
> - 記住：argv[0] 分派用真的 symlink 驗證，不是呼叫函式。
> - 下一步：第 9 施工關加每個命令的輸出與錯誤 snapshot。
> - holder 的跨程序測試（`tests/holder_process.rs`）與 daemon ↔ holder 的測試（`tests/holder_runtime.rs`、`tests/daemon_process.rs`，第 6 施工關）也在這裡，因為要用真的 binary。

## 怎麼跑

```bash
cargo test -p agend
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `tests/argv0_dispatch.rs::version_prints_the_package_version` | `--version` 印出 `agend <版本>` |
| `tests/argv0_dispatch.rs::unknown_command_fails_with_usage_hint` | 未知命令 exit 2 並提示 `agend --help` |
| `tests/argv0_dispatch.rs::invoked_as_git_reaches_the_shim` | 名為 `git` 的 symlink 進入 shim，不會走 CLI：`git --version` 交給真的 git；沒有 binding 時 `git worktree add` 被拒絕（exit 1、附下一步） |
| `tests/shim_*.rs` | agend-shim 的真 repo 測試（shim 導向與拒絕、git hook、快照、kill 形式）；每個檔案證明什麼見 [agend-shim/TESTING.md](../agend-shim/TESTING.md#測試分類真-repocratesagendtests) |
| `tests/holder_runtime.rs` | 第 6 施工關契約第 1、2 層：RTM-1..9 對真的 `HolderRuntime` + 真 holder；測試 binary 重新執行自己的四次開機（四個 pid、holder pid 不變、計數器變大）；反向：每次開機換新的 `AGEND_HOME` 必須在開機 2 失敗 |
| `tests/daemon_process.rs` | 第 6 施工關契約第 3 層與 P1、P3、P6：真 `agend daemon` 四次開機（開機 3 被測試 `kill -9`）、反向檢查、agent 一直死 → 3 次 `--resume` 後 `failed`、agent 環境白名單與 shim 在 PATH 最前、第二個 daemon 10 秒後被拒、孤兒 holder 下次開機被 `Shutdown`、沒有 `AGEND_HOME` exit 1。各段內容與 `daemon_probe demo` 共用（見 [agend-daemon TESTING](../agend-daemon/TESTING.md)） |
| `tests/holder_process.rs` | `agend holder`：啟動器結束後 holder 還在、四次獨立開機看到同一個 holder、重複啟動 exit 1、agent 的 TERM／HUP／INT／QUIT 無效、安全網、路徑太長拒絕（細節見 [agend-holder TESTING](../agend-holder/TESTING.md)） |

## 用到的假實作

- `agend_testkit::tempdir::TempDir`
- `tests/shim_common` 的 `Fixture`（見 agend-shim/TESTING.md）

## 還沒測的

- [ ] 所有 CLI 命令、doctor、init（第 9 施工關）

## 下一步

```bash
cargo test -p agend
```
