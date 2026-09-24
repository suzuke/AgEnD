# xtask

> **TL;DR**
> - 開發者工具：`cargo xtask check-deps` 與 `cargo xtask accept <關>`。
> - 記住：**crate 邊界規則由 `check-deps` 強制**；規則是 `xtask/src/check_deps.rs` 裡的資料。
> - 下一步：改依賴後跑 `cargo xtask check-deps`。

## 負責

- `check-deps`：檢查每條規則的 crate 在 `cargo tree -e normal --target all` 裡沒有被禁止的 crate；檢查 agend-testkit 不是任何 crate 的一般依賴
- `accept <關>`：對該關的 crate 跑 fmt、clippy、test，再跑 check-deps；demo 隨各關加入

## 不負責

- 原始碼層級的規則（agend-core 不起程序、不開 socket／檔案）：由 clippy 與 `crates/agend-core/clippy.toml` 負責
- 產生 protocol JSON schema、打包 release、錄製 backend 畫面 fixture（規劃中，未實作）

## 模組

| 模組 | 職責 |
|---|---|
| `check_deps` | 規則與檢查 |
| `accept` | 12 關的 crate 對照與執行 |

## 依賴規則

- 一般依賴：無（只用 std，透過 `$CARGO` 執行 cargo）
- 不屬於 release binary

## 入口

- `cargo xtask check-deps`、`cargo xtask accept <1-12 或名稱>`（alias 在 `.cargo/config.toml`）

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
| `agend-core` | 四個群組全部 + `agend-*` |
| `agend-shim` | async runtime、database、`agend-daemon` |
| `agend-client` | async runtime、database、`agend-daemon` |
| 所有 crate | `agend-testkit` 當一般依賴 |

build-dependency 與 dev-dependency 不檢查。`libc` 不在清單內（很多純 crate 也會間接用到）。

## 下一步

```bash
cargo xtask check-deps
cargo xtask accept core
```
