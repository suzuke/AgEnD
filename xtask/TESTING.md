# xtask 測試

> **TL;DR**
> - 測規則判斷的純函式，也用真的 `cargo tree` 輸出測解析。
> - 記住：無 std 編譯要 rustup 的 cargo；單元測試不依賴它，完整檢查用 `~/.cargo/bin/cargo xtask check-deps`。
> - 下一步：改禁止清單後跑 `cargo test -p xtask`，再手動加一個被禁止的依賴確認會失敗。

## 怎麼跑

```bash
cargo test -p xtask
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `check_deps::tests::parses_real_cargo_tree_output` | 解析真的 `cargo tree` 輸出（producer 產生，不手寫） |
| `check_deps::tests::denies_runtime_and_prefix_matches` | `tokio`、`tokio-*` 前綴、`agend-daemon` 會被抓；`serde`、`agend-core` 不會 |
| `check_deps::tests::a_crate_is_not_a_violation_of_its_own_rule` | 規則不會因 crate 自己的名字失敗 |
| `check_deps::tests::current_workspace_passes` | 目前 workspace 符合所有規則（以 `--allow-skip` 跑，因為 Homebrew cargo 沒有 no-std target） |
| `check_core::tests::real_metadata_of_core_passes` | 真的 `cargo metadata` 下 agend-core 沒有 build script、沒有依賴 |
| `check_core::tests::build_script_is_rejected` | 在真的 metadata 上加一個 `custom-build` target 會被抓 |
| `check_core::tests::any_dependency_kind_is_rejected` | normal、build、dev 依賴都會被抓 |
| `check_core::tests::features_are_rejected` | 在真的 metadata 上加一個 `std` feature 會被抓 |
| `check_core::tests::missing_target_is_recognised` | 「target 沒裝」與「程式用了 std」分得開 |
| `accept::tests::*` | 13 個施工關編號連續、可用編號或名稱找到、每個施工關的 crate 都存在 |

## 用到的假實作

- 無。

## 還沒測的

- [ ] 無 std 編譯本身沒有自動化反例測試。已在 repo 外的暫存副本手動驗證，以下全部讓 `check-deps` 失敗：`#[macro_use] extern crate std`、`pub extern crate std`、`[lib] path` 改指到用 std 的檔案、path 依賴 re-export `std::fs::read`、`unsafe extern "C" { fn getpid() }`（含刪掉 `#![forbid(unsafe_code)]` 之後）、build.rs 輸出 `cargo:rustc-cfg=test`、由其他 crate 啟用的選用 `std` feature。
- [ ] `accept` 真的去跑 fmt／clippy／test（要遞迴呼叫 cargo，只手動驗證過）。

## 下一步

```bash
cargo test -p xtask
~/.cargo/bin/cargo xtask check-deps
```
