# xtask

> **TL;DR**
> - 開發者工具：`cargo xtask check-deps` 與 `cargo xtask accept <施工關>`。
> - 記住：**crate 邊界規則由 `check-deps` 強制**；規則是 `xtask/src/check_deps.rs` 裡的資料。
> - 下一步：改依賴後跑 `cargo xtask check-deps`。

## 負責

- `check-deps`：
  1. shim／client 在 `cargo tree -e normal,build --target all` 裡沒有被禁止的 crate（dev 依賴不檢查）
  2. agend-testkit 不是任何 crate 的一般依賴
  3. agend-core：`cargo metadata` 顯示沒有 build script、沒有 `[features]`、唯一直接依賴是停用 default features 且只開 `derive` + `alloc` 的 serde（D32）；而且能以 `--all-features`、`-F unsafe-code` 對無 std 的 `thumbv7em-none-eabihf` 編譯（見 `check_core.rs`）
  4. target 沒裝時印 `SKIPPED` 並失敗；`--allow-skip` 才不失敗（仍印 SKIPPED）

`SKIPPED` 代表**沒有驗證**，不是通過。

- 本機要驗證：`rustup target add thumbv7em-none-eabihf`，再跑 `~/.cargo/bin/cargo xtask check-deps`（不加 `--allow-skip`）。Homebrew 的 `cargo` 沒有額外 target，一定會 SKIPPED。
- `--allow-skip` 只在你明白這一項沒驗證時用；它仍印出 SKIPPED。
- CI 一定會跑這一項（不加 `--allow-skip`）。

- `accept core`：跑 workspace fmt、workspace clippy、core tests、protocol compatibility tests、check-deps，再執行 core example 的 protocol 與 code workflow demo。
- `accept testkit`：對 agend-testkit 跑 fmt、clippy、test（含 7 個契約 suite 與假 agent 程式測試），再跑 check-deps，然後 build 假 agent binary、執行 `testkit_demo` example（三個假 agent 各一段往來並正常結束、假 daemon 的事件身分、契約摘要）。
- `accept shim`：對 agend-shim 跑 fmt、clippy、test，加上 `agend` 的 argv[0] 分派測試與 check-deps，再 build `agend` 並執行 `agend-shim` 的 `shim_demo` example（以 `git`／`kill`／`pkill` 名稱在暫存 repo 裡跑真的 binary）
- 其他 `accept <施工關>`：對該施工關的 crate 跑 fmt、clippy、test，再跑 check-deps；demo 隨各施工關加入

## 不負責

- 擋刻意繞過（例如改 xtask、加長 allowlist）：靠 code review
- 產生 protocol JSON schema、打包 release、錄製 backend 畫面 fixture（規劃中，未實作）

## 模組

| 模組 | 職責 |
|---|---|
| `check_deps` | 規則與檢查 |
| `check_core` | agend-core 的結構檢查：`cargo metadata` 規則與無 std 編譯 |
| `accept` | 13 個施工關的 crate 對照與執行 |
| `core_demo`、`shim_demo` | 第 1、3 施工關的 demo（子程序執行 example） |

## 依賴規則

- 一般依賴：`serde_json`（metadata）；`agend-core` 與 `toml`（workflow golden 測試）只作為 xtask 測試的 dev-dependency，core acceptance demo 由子程序執行獨立 example，避免 checker 連結待檢查的 core；透過 `$CARGO` 執行 cargo，無 std 編譯時用同一個 toolchain 的 rustc
- workspace 根目錄在執行時用 `cargo locate-project --workspace` 從目前目錄找，所以在 repo 副本裡跑會檢查副本本身
- 不屬於 release binary

## 入口

- `cargo xtask check-deps`、`cargo xtask accept <1-13 或名稱>`（alias 在 `.cargo/config.toml`）

## 細節

### 施工關對照

`cargo xtask accept <編號或名稱>`：

| 編號 | 名稱 | 施工關頁 |
|---|---|---|
| 1 | `core` | [docs/gates/gate-01-core.md](../docs/gates/gate-01-core.md) |
| 2 | `testkit` | [docs/gates/gate-02-testkit.md](../docs/gates/gate-02-testkit.md) |
| 3 | `shim` | [docs/gates/gate-03-shim.md](../docs/gates/gate-03-shim.md) |
| 4 | `holder` | [docs/gates/gate-04-holder.md](../docs/gates/gate-04-holder.md) |
| 5 | `store` | [docs/gates/gate-05-store.md](../docs/gates/gate-05-store.md) |
| 6 | `daemon-holder` | [docs/gates/gate-06-daemon-holder.md](../docs/gates/gate-06-daemon-holder.md) |
| 7 | `codex` | [docs/gates/gate-07-codex.md](../docs/gates/gate-07-codex.md) |
| 8 | `client` | [docs/gates/gate-08-client.md](../docs/gates/gate-08-client.md) |
| 9 | `cli` | [docs/gates/gate-09-cli.md](../docs/gates/gate-09-cli.md) |
| 10 | `pipeline` | [docs/gates/gate-10-pipeline.md](../docs/gates/gate-10-pipeline.md) |
| 11 | `tui` | [docs/gates/gate-11-tui.md](../docs/gates/gate-11-tui.md) |
| 12 | `adapters` | [docs/gates/gate-12-adapters.md](../docs/gates/gate-12-adapters.md) |
| 13 | `install` | [docs/gates/gate-13-install.md](../docs/gates/gate-13-install.md) |

### 禁止清單

| 群組 | crate（`*` = 前綴） |
|---|---|
| async runtime | `tokio`、`tokio-*`、`async-std`、`smol`、`mio` |
| database | `rusqlite`、`libsqlite3-sys`、`sqlx`、`sqlx-*` |
| network | `hyper`、`hyper-*`、`reqwest`、`ureq`、`h2`、`socket2`、`tungstenite`、`tokio-tungstenite`、`teloxide` |
| process | `portable-pty`、`nix`、`signal-hook`、`signal-hook-*` |

| crate | 禁止 |
|---|---|
| `agend-core` | serde 以外的依賴、default features、derive／alloc 以外的 serde features，以及任何 `[features]`，由 `check_core.rs` 以 `cargo metadata` 檢查 |
| `agend-shim` | async runtime、database、`agend-daemon` |
| `agend-client` | async runtime、database、`agend-daemon` |
| 所有 crate | `agend-testkit` 當一般依賴 |

shim／client 檢查 normal 與 build 依賴，不檢查 dev 依賴。`network`、`process` 群組目前沒有規則使用，保留當文件。`libc` 不在清單內。

## 下一步

```bash
~/.cargo/bin/cargo xtask check-deps   # rustup 的 cargo；Homebrew cargo 會 SKIPPED
cargo xtask accept core
```
