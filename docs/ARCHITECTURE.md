# 架構總覽

> **TL;DR**
> - daemon 是唯一的大型 I/O 層；agent 與附屬程序由每個 instance 一個的 holder 持有，所以 daemon 可隨時重啟。
> - 記住：**crate 邊界就是架構**，由 `cargo xtask check-deps` 強制。
> - 下一步：系統圖看 [README](../README.md#系統圖)；細節看本頁底部的分頁連結。

來源：規劃 r4（§4、§5）與決策 D1–D37。後來的決策優先於規劃本文。

## 程序模型

| 程序 | 數量 | 職責 |
|---|---|---|
| daemon | 1，常駐（launchd／systemd） | protocol server、流水線、送達、監督、排程、對帳、DB |
| holder | 每個 instance 1 個 | PTY、畫面（alacritty_terminal）、附屬程序（codex app-server、opencode serve） |
| agent | 每個 instance 1 個 | codex／claude／opencode；PATH 上有 shim 與 `agend` |
| client | 任意 | TUI、CLI、未來的 GUI；都走 protocol v1 |

規則：

1. agent 側沒有 daemon 的子程序。v1 由 daemon 啟動 codex app-server 與 opencode serve，v2 改由 holder 持有。
2. daemon 重啟後：重連 holder → 直接取 holder 裡的現成畫面（不重播位元組）→ 重連 backend 補回斷線期間的事件。
3. holder 被硬殺時裡面的 agent 會一起死。這是所有方案（自有 holder、tmux、herdr）共同的上限。
4. holder 與 claude channel bridge 升級前仍跑舊 binary，兩者的協定都必須有版本且向後相容。
5. 自舉隔離：daemon、holder、shim 跑已安裝的 release 版；開發中的 AgEnD 在另一個 clone。

## Crate 地圖

切 crate 的四條準則（D10）；其餘一律是模組 + trait。

| # | 準則 | 例子 |
|---|---|---|
| 1 | 需要編譯器強制的限制 | `agend-core` 不能依賴 tokio、rusqlite、process／network crate |
| 2 | 獨立程序、有自己的穩定性要求 | `agend-daemon`、`agend-holder`、`agend-shim`、`agend-tui` |
| 3 | 對外使用 | `agend-client`（TUI、CLI、未來 Rust GUI 共用） |
| 4 | 多個 crate 共用的測試基礎設施 | `agend-testkit`（只能當 dev-dependency） |

依賴方向（實線 = 編譯期依賴）：

| crate | 依賴 |
|---|---|
| `agend-core` | `serde`（`default-features = false`，`derive` + `alloc`；JSON I/O 在 adapter；D32） |
| `agend-daemon`、`agend-holder`、`agend-shim`、`agend-client` | `agend-core` |
| `agend-tui` | `agend-core`、`agend-client` |
| `agend` | 以上全部；唯一 binary |
| `agend-testkit` | `agend-core`；只被當 dev-dependency |

邊界規則與強制工具：

| 規則 | 工具 |
|---|---|
| `agend-shim`、`agend-client` 沒有 async runtime、SQLite，也不依賴 `agend-daemon` | `cargo xtask check-deps` |
| 任何 crate 都不能把 `agend-testkit` 當一般依賴 | `cargo xtask check-deps` |

依賴清單在 `xtask/src/check_deps.rs`（說明見 [xtask/README.md](../xtask/README.md)）；shim／client 檢查 normal 與 build 依賴，不檢查 dev 依賴。

agend-core 不用 std（`#![no_std]` + `alloc`），只有 `serde` 可供型別 derive（無預設 features，只開 `derive` + `alloc`；D32）；JSON 編碼留在 adapter，時間只經 `Clock` trait。保護方式：

| 保護 | 擋下什麼 | 工具 |
|---|---|---|
| 以 `--all-features` 對無 std 的 target（`thumbv7em-none-eabihf`）編譯 agend-core | 會被編譯到的 std 使用：`extern crate std` 的各種寫法、`[lib] path` 改指、`include!`、用到 std 的依賴（驗證過的案例見 `xtask/TESTING.md`） | `cargo xtask check-deps`（需要該 target） |
| 同一次編譯帶 `-F unsafe-code` | `unsafe extern "C"` 之類直接呼叫 libc 的 FFI；原始碼的 `#![forbid(unsafe_code)]` 被刪也照擋（屬性留著給 IDE 即時提示） | `cargo xtask check-deps` |
| `cargo metadata` 規則 | agend-core 有 build script、有任何 `[features]`、serde default features 或 derive／alloc 以外 features，或有 serde 以外的依賴（normal／build／dev） | `cargo xtask check-deps` |

威脅模型：這些保護擋的是意外把 I/O 帶進 core，不是刻意繞過（例如改 xtask 本身、把 allowlist 加長）；後者靠 code review。

已知缺口（接受，只有刻意才會發生）：以對無 std target 為假的 cfg 包住的程式碼，例如 `#[cfg(not(target_os = "none"))]`，在那個 target 上不會被編譯，所以擋不到。

## daemon 分層

| 層 | 模組 | 做什麼 |
|---|---|---|
| 入口 | `server`、`handlers`、`ingest` | protocol server；命令處理（與傳輸分離）；hook／事件接收與磁碟佇列補送 |
| 領域 | `pipeline`、`delivery`、`supervisor`、`scheduler`、`reconcile`、`fleet` | 驅動 core 狀態機；送達；卡住／額度／轉派；timeout／cron；DB ↔ git 對帳；`fleet`：全貌快照與記憶體事件記錄，供 client protocol 的 `get_fleet`／訂閱讀（第 8 施工關） |
| adapter | `driver/{codex,claude,opencode}`、`runtime`、`forge/{local,github}`、`git`、`runner`、`store`、`notifier` | 對外的一切 I/O |

- 領域模組只透過 `agend_core::traits` 呼叫 adapter，所以能對 testkit 的假實作測。
- tokio runtime 為 multi-thread；SQLite 由專屬執行緒持有、經 channel 存取；git、gh、checks 指令一律 `tokio::process` + timeout。
- v1 兩天有 861 次「scanner-thread slip」，這是上一條的理由。

## core 內容

| 模組 | 內容 |
|---|---|
| `config` | `config.toml`（daemon 層級，人寫、daemon 只讀） |
| `model` | 共用型別（backend、team、訊息狀態、branch 命名空間…） |
| `protocol/{client,holder}` | 兩套有版本的協定；外部 GUI 用產生的 JSON schema |
| `traits` | `Driver`、`Forge`、`Store`、`Runtime`、`Notifier`、`Clock` |
| `pipeline/{stage,task,workflow}` | 6 種關卡狀態機、task 關係與操作、workflow 存檔檢查 |
| `policy/{busy,debounce,conflict,merge_gate,assign}` | 忙碌策略、去抖動、衝突偵測、merge 門檻與 patch-id、角色分派 |
| `screen` | hard gate 分類器；規則是資料檔 |

`policy/assign`：D18 分派規則是純邏輯，放在 core；daemon 只提供候選成員與負載等輸入（D25）。

## 細節

| 主題 | 文件 |
|---|---|
| 流水線、workflow、merge、worktree／branch 生命週期、shim | [architecture/pipeline.md](architecture/pipeline.md) |
| 訊息送達、狀態偵測、啟動提示 | [architecture/delivery.md](architecture/delivery.md) |
| TUI、設定與目錄、安裝 | [architecture/tui-and-setup.md](architecture/tui-and-setup.md) |

## 待定

第 1 施工關開工前要提案的事項只列在一個地方：[ROADMAP.md](ROADMAP.md#第-1-施工關開工前先提案經使用者確認才實作)。

另外還沒決定、不在那份清單裡的：

（無。每日 `VACUUM INTO` 快照保留份數已決定為 7 份，第 5 施工關 D31。）

## 下一步

```bash
cat docs/architecture/pipeline.md
cargo xtask check-deps
```
