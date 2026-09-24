# agend-core

> **TL;DR**
> - 純邏輯 crate：型別、協定、trait、流水線狀態機、policy、螢幕分類器。
> - 記住：**`#![no_std]` + `alloc`**，編譯器保證沒有任何 I/O；時間只經 `Clock` trait。
> - 下一步：第 1 關在這裡開始（見 docs/ROADMAP.md）。

## 負責

- 所有 crate 共用的型別（`model`）：backend、`general` team、`lifetime`、訊息送達狀態、branch 命名空間
- 兩套有版本的協定定義：client（protocol v1）與 holder
- 邊界 trait：`Driver`、`Forge`、`Store`、`Runtime`、`Notifier`、`Clock`（簽章第 1 關設計）
- 流水線狀態機（6 種關卡、task 關係與操作、workflow 存檔檢查）
- policy：忙碌策略、去抖動、衝突偵測、merge 門檻與 patch-id、角色分派
- 螢幕分類器（只認 hard gate；規則是資料）
- `config.toml` 的結構（呼叫端傳入文字）

## 不負責

- 讀寫檔案、環境變數、socket、子程序
- 執行關卡（daemon 的 `pipeline` 做）
- 判斷 busy／idle（來自結構化事件）

## 模組

| 模組 | 職責 |
|---|---|
| `config` | daemon 層級 `config.toml` |
| `model` | 共用型別與 branch／worktree 命名 |
| `protocol::client` | client protocol v1 |
| `protocol::holder` | holder 協定；`ControlKey`（PTY 只能送的單一控制鍵） |
| `traits` | 邊界 trait（尚未定義） |
| `pipeline::stage` | `StageKind`（6 種）、`FanoutJoin` |
| `pipeline::task` | task 關係與操作 |
| `pipeline::workflow` | workflow 定義與存檔檢查（D19） |
| `policy::busy` | `BusyLevel`、`effective_level` |
| `policy::debounce` | 去抖動 |
| `policy::conflict` | 檔案重疊偵測 |
| `policy::merge_gate` | merge 門檻、D14 核准保留 |
| `policy::assign` | D18 分派規則（放 core 是骨架的選擇） |
| `screen` | hard gate 分類器 |

## 依賴規則

- 一般依賴：無（只有 std）
- 禁止依賴：async runtime、SQLite、network、process crate、任何其他 `agend-*`（`cargo xtask check-deps`）
- `#![no_std]` + `alloc`：沒有檔案、程序、網路、環境變數、thread、stdio、時鐘；要雜湊表時用 `BTreeMap`／`BTreeSet`，錯誤型別用 `core::error::Error`（編譯器強制）
- `cargo xtask check-deps` 確認 `#![no_std]` 這一行還在，且 `extern crate std` 只出現在 `#[cfg(test)]` 下

## 入口

- `agend_core::model`、`agend_core::pipeline::stage::StageKind`、`agend_core::policy::busy::effective_level`、`agend_core::protocol::holder::ControlKey`

## 下一步

```bash
cargo test -p agend-core
cargo xtask accept core
```
