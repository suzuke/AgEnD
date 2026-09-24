# xtask 測試

> **TL;DR**
> - 測規則判斷的純函式，也用真的 `cargo tree` 輸出測解析。
> - 記住：`current_workspace_passes` 會對目前的 workspace 跑完整 `check-deps`。
> - 下一步：改禁止清單後跑 `cargo test -p xtask`，再手動加一個被禁止的依賴確認會失敗。

## 怎麼跑

```bash
cargo test -p xtask
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `check_deps::tests::parses_real_cargo_tree_output` | 解析真的 `cargo tree` 輸出（producer 產生，不手寫） |
| `check_deps::tests::denies_runtime_and_prefix_matches` | `tokio`、`tokio-*` 前綴、`agend-*` 都會被抓到；`serde` 不會 |
| `check_deps::tests::a_crate_is_not_a_violation_of_its_own_rule` | `agend-core` 不因自己的名字符合 `agend-*` 而失敗 |
| `check_deps::tests::no_std_attribute_is_required` | 有 `#![no_std]` 行才算；註解掉的不算 |
| `check_deps::tests::std_is_only_linked_for_tests` | `extern crate std` 不在 `#[cfg(test)]` 下（含 `as s` 別名）會被抓 |
| `check_deps::tests::current_workspace_passes` | 目前 workspace 符合所有規則 |
| `accept::tests::*` | 12 關編號連續、可用編號或名稱找到、每關的 crate 都存在 |

## 用到的假實作

- 無。

## 還沒測的

- [ ] 「移除 `#![no_std]` 會讓 check-deps 失敗」只在 repo 外的暫存副本手動驗證過（純函式 `declares_no_std`、`std_only_for_tests` 有單元測試）。
- [ ] `accept` 真的去跑 fmt／clippy／test（要遞迴呼叫 cargo，只手動驗證過）。
- [ ] 違規時的完整 exit code 路徑：手動驗證過（把 tokio 加進 agend-core → exit 1），沒有自動化測試。

## 下一步

```bash
cargo test -p xtask
cargo xtask check-deps
```
