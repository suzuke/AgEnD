# agend-core

> **TL;DR**
> - 純邏輯 crate：型別、協定、trait、流水線狀態機、policy、螢幕分類器。
> - 記住：**`#![no_std]` + `alloc` + `forbid(unsafe_code)`，沒有依賴、沒有 build script**；時間只經 `Clock` trait。
> - 下一步：第 1 施工關在這裡開始（見 docs/ROADMAP.md）。

## 負責

- 所有 crate 共用的型別（`model`）：backend、`general` team、`lifetime`、訊息送達狀態、branch 命名空間
- 兩套有版本的協定定義：client（protocol v1）與 holder
- 邊界 trait：`Driver`、`Forge`、`Store`、`Runtime`、`Notifier`、`Clock`（簽章第 1 施工關設計）
- 流水線狀態機（6 種關卡、task 關係與操作、workflow 存檔檢查）
- policy：忙碌策略、去抖動、衝突偵測、merge 門檻與 patch-id、角色分派
- 螢幕分類器（只認 hard gate；規則是資料）
- `config.toml` 的結構（呼叫端傳入文字）
- 安裝規則（`setup`）：只有資料與純函式；執行在 `agend` crate

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
| `policy::assign` | D18 分派規則；daemon 只提供候選成員與負載等輸入（D25） |
| `screen` | hard gate 分類器 |
| `setup` | 安裝規則（第 13 施工關）：已測的 backend 版本範圍、登入判斷、git 最低版本、launchd／systemd unit 文字；只有資料與純函式 |

## 依賴規則

- `#![no_std]` + `alloc`；沒有任何依賴、沒有 `[features]`、沒有 build script
- 不用 std：`alloc` 的 `String`、`Vec`、`format!`；雜湊表用 `BTreeMap`／`BTreeSet`；錯誤型別用 `core::error::Error`

| 保護 | 擋下什麼 | 工具 |
|---|---|---|
| 以 `--all-features` 對無 std 的 target（`thumbv7em-none-eabihf`）編譯 agend-core | 會被編譯到的 std 使用：`extern crate std` 的各種寫法、`[lib] path` 改指、`include!`、用到 std 的依賴（驗證過的案例見 `xtask/TESTING.md`） | `cargo xtask check-deps`（需要該 target） |
| 同一次編譯帶 `-F unsafe-code` | `unsafe extern "C"` 之類直接呼叫 libc 的 FFI；原始碼的 `#![forbid(unsafe_code)]` 被刪也照擋（屬性留著給 IDE 即時提示） | `cargo xtask check-deps` |
| `cargo metadata` 規則 | agend-core 有 build script、有任何 `[features]`，或有任何依賴（normal／build／dev）不在 `CORE_DEP_ALLOWLIST`（目前是空的） | `cargo xtask check-deps` |

威脅模型：這些保護擋的是意外把 I/O 帶進 core，不是刻意繞過（例如改 xtask 本身、把 allowlist 加長）；後者靠 code review。

已知缺口（接受，只有刻意才會發生）：以對無 std target 為假的 cfg 包住的程式碼，例如 `#[cfg(not(target_os = "none"))]`，在那個 target 上不會被編譯，所以擋不到。

## 入口

- `agend_core::model`、`agend_core::pipeline::stage::StageKind`、`agend_core::policy::busy::effective_level`、`agend_core::protocol::holder::ControlKey`

## 下一步

```bash
cargo test -p agend-core
cargo xtask accept core
```
