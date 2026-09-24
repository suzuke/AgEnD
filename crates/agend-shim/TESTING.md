# agend-shim 測試

> **TL;DR**
> - 目前測 argv[0] 分派；整個 binary 的分派在 `crates/agend/tests/argv0_dispatch.rs`。
> - 記住：shim 測試要在暫存 repo 裡跑，絕不碰宿主 repo。
> - 下一步：第 3 施工關用 testkit 的 git fixture 測導向、拒絕、快照還原。

## 怎麼跑

```bash
cargo test -p agend-shim
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `tests::dispatches_on_basename` | `git`、完整路徑的 `git`、`kill`、`killall`、`./pkill` 都被認出 |
| `tests::other_names_are_not_the_shim` | `agend`、`git-lfs`、`gitk`、空字串、`/` 不是 shim |

## 用到的假實作

- 目前無；第 3 施工關用 `agend_testkit::git_fixture`

## 還沒測的

- [ ] classify、protected_ref、snapshot、kill_guard、audit（第 3 施工關）
- [ ] binding 快照缺失時的行為

## 下一步

```bash
cargo test -p agend-shim
```
