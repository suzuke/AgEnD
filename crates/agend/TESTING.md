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
| `tests/daemon_process.rs` | 第 6 施工關契約第 3 層與 P1、P3、P6：真 `agend daemon` 四次開機（開機 3 被測試 `kill -9`）、反向檢查、agent 一直死 → 3 次 `--resume` 後 `failed`、daemon 死在第一次 `Spawn` 前 → 下次仍 `--session-id`（不 resume 沒建立的 session）、agent 環境白名單與 shim 在 PATH 最前、第二個 daemon 10 秒後被拒、孤兒 holder 下次開機被 `Shutdown`、沒有 `AGEND_HOME` exit 1。各段內容與 `daemon_probe demo` 共用（見 [agend-daemon TESTING](../agend-daemon/TESTING.md)） |
| `tests/client_protocol.rs` | 第 8 施工關：CLP-1..12 除 CLP-8 對真 `agend daemon`（每條一個新 home、1 個在跑 + 6 個 `failed` 的 instance）；CLP-8（2000 個事件的慢 client）對同一份 daemon server 程式在測試程序裡跑；socket 0700／0600、開機計畫做完才 bind（一個 holder 鎖被持有但不回應的 instance 讓計畫等 5 秒：socket 出現的第一刻連上，看到的全貌裡下一個 instance 已經起來）、`kill -9` 後舊 socket 被換掉、Ctrl-C 刪檔、101 bytes 的路徑拒絕且不建 `agend.db`（P1）；`retry` 依 backend 與 `session_started` 帶 `--resume`／`--session-id`／不帶，先 `Shutdown` 留著的 holder（每個 instance 只起一次 holder、沒有 start failed、沒有 restart），opencode 跑過的沒有 `retry`（codex 在第 7 施工關後可以 resume），`failed` 的終端只回最後畫面（P5、P6）；在跑的終端先畫面後 `terminal_bytes`、不存在的 instance `no_terminal`、`command` 回 `not_supported`；`agend debug ping --count 12` 跨 daemon 重啟全部 ok、有一行 `retried`，`agend debug watch` 重連後重拿全貌；對只講 1.0 的假 daemon 立刻失敗；沒有 daemon 時 10–13 秒後的訊息；`debug` 的參數與 `AGEND_HOME`。各段與 `client_demo` 共用（`agend-daemon/tests/common/client_process.rs`） |
| `tests/codex_process.rs` | 第 7 施工關：真 `agend daemon`、真 holder、`sh` 包裝，codex 是本 crate 的 example `fake_codex`。第一次啟動先建 thread 再交接、app-server 與 TUI 同一個 process group、TUI 印 `resume <thread>`；daemon 跑著時 `kill -9` holder → 清掃、`restart 1/3`、同一個 thread；codex 忽略 SIGHUP 時 holder 死掉留下的 app-server 被清掃 SIGKILL（daemon 跑著、以及 daemon 停著時下次開機）；TUI 一直死 → 3 次都 resume 同一個 thread、`failed`、沒有 codex 留下（TUI 結束時 app-server 跟著結束）；app-server 死掉 20 秒後算死亡、舊 holder 收到 `Shutdown`、新的接回同一個 thread；第一次啟動在 `thread/start` 前 daemon 被 `kill -9` → 下次開機建 thread、等著的包裝接手、不是 `failed`；schema v3 的第 6 施工關 codex 列（holder 活著）→ `failed`＋`legacy_no_thread`、holder 不動、沒有連 app-server，`new` 的照常起；`failed`、holder 還活著、`agent_pid` 有值且 argv 有 thread 標記的 instance，開機不清掃（開機改成一律清掃時這條失敗）。各段與 `codex_demo` 共用（`agend-daemon/tests/common/codex_process.rs`） |
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
