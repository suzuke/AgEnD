# xtask

> **TL;DR**
> - 開發者工具：`cargo xtask check-deps` 與 `cargo xtask accept <關>`。
> - 記住：**crate 邊界規則由 `check-deps` 強制**；規則是 `xtask/src/check_deps.rs` 裡的資料。
> - 下一步：改依賴後跑 `cargo xtask check-deps`。

## 負責

- `check-deps`：
  1. shim／client 在 `cargo tree -e normal,build --target all` 裡沒有被禁止的 crate（dev 依賴不檢查）
  2. agend-testkit 不是任何 crate 的一般依賴
  3. agend-core：`cargo metadata` 顯示沒有 build script、沒有 `[features]`、沒有 allowlist 以外的依賴；而且能以 `--all-features`、`-F unsafe-code` 對無 std 的 `thumbv7em-none-eabihf` 編譯（見 `check_core.rs`）
  4. target 沒裝時印 `SKIPPED` 並失敗；`--allow-skip` 才不失敗（仍印 SKIPPED）

`SKIPPED` 代表**沒有驗證**，不是通過。

- 本機要驗證：`rustup target add thumbv7em-none-eabihf`，再跑 `~/.cargo/bin/cargo xtask check-deps`（不加 `--allow-skip`）。Homebrew 的 `cargo` 沒有額外 target，一定會 SKIPPED。
- `--allow-skip` 只在你明白這一項沒驗證時用；它仍印出 SKIPPED。
- CI 一定會跑這一項（不加 `--allow-skip`）。

- `accept <關>`：對該關的 crate 跑 fmt、clippy、test，再跑 check-deps；demo 隨各關加入

## 不負責

- 擋刻意繞過（例如改 xtask、加長 allowlist）：靠 code review
- 產生 protocol JSON schema、打包 release、錄製 backend 畫面 fixture（規劃中，未實作）

## 模組

| 模組 | 職責 |
|---|---|
| `check_deps` | 規則與檢查 |
| `check_core` | agend-core 的結構檢查：`cargo metadata` 規則與無 std 編譯 |
| `accept` | 13 關的 crate 對照與執行 |

## 依賴規則

- 一般依賴：`serde_json`（解析 `cargo metadata`）；透過 `$CARGO` 執行 cargo，無 std 編譯時用同一個 toolchain 的 rustc
- workspace 根目錄在執行時用 `cargo locate-project --workspace` 從目前目錄找，所以在 repo 副本裡跑會檢查副本本身
- 不屬於 release binary

## 入口

- `cargo xtask check-deps`、`cargo xtask accept <1-13 或名稱>`（alias 在 `.cargo/config.toml`）

## 細節

### 禁止清單

| 群組 | crate（`*` = 前綴） |
|---|---|
| async runtime | `tokio`、`tokio-*`、`async-std`、`smol`、`mio` |
| database | `rusqlite`、`libsqlite3-sys`、`sqlx`、`sqlx-*` |
| network | `hyper`、`hyper-*`、`reqwest`、`ureq`、`h2`、`socket2`、`tungstenite`、`tokio-tungstenite`、`teloxide` |
| process | `portable-pty`、`nix`、`signal-hook`、`signal-hook-*` |

| crate | 禁止 |
|---|---|
| `agend-core` | 任何依賴（allowlist 為空）與任何 `[features]`，由 `check_core.rs` 以 `cargo metadata` 檢查 |
| `agend-shim` | async runtime、database、`agend-daemon` |
| `agend-client` | async runtime、database、`agend-daemon` |
| 所有 crate | `agend-testkit` 當一般依賴 |

shim／client 檢查 normal 與 build 依賴，不檢查 dev 依賴。`network`、`process` 群組目前沒有規則使用，保留當文件。`libc` 不在清單內。

## 下一步

```bash
~/.cargo/bin/cargo xtask check-deps   # rustup 的 cargo；Homebrew cargo 會 SKIPPED
cargo xtask accept core
```
