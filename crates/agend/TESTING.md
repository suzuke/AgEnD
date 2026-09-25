# agend 測試

> **TL;DR**
> - 整合測試直接執行建好的 binary。
> - 記住：argv[0] 分派用真的 symlink 驗證，不是呼叫函式。
> - 下一步：第 9 施工關加每個命令的輸出與錯誤 snapshot。
> - holder 的跨程序測試也在這裡（`tests/holder_process.rs`），因為要用真的 binary。

## 怎麼跑

```bash
cargo test -p agend
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `tests/argv0_dispatch.rs::version_prints_the_package_version` | `--version` 印出 `agend <版本>` |
| `tests/argv0_dispatch.rs::unknown_command_fails_with_usage_hint` | 未知命令 exit 2 並提示 `agend --help` |
| `tests/argv0_dispatch.rs::invoked_as_git_reaches_the_shim` | 名為 `git` 的 symlink 進入 shim，不會走 CLI |
| `tests/holder_process.rs` | `agend holder`：啟動器結束後 holder 還在、四次獨立開機看到同一個 holder、重複啟動 exit 1、agent 的 TERM／HUP／INT／QUIT 無效、安全網、路徑太長拒絕（細節見 [agend-holder TESTING](../agend-holder/TESTING.md)） |

## 用到的假實作

- `agend_testkit::tempdir::TempDir`

## 還沒測的

- [ ] 所有 CLI 命令、doctor、init（第 9 施工關）

## 下一步

```bash
cargo test -p agend
```
