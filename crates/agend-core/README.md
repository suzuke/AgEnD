# agend-core

> **TL;DR**
> - 純邏輯 crate：共用型別、協定、traits、workflow 狀態機、policy 與螢幕分類器。
> - 記住：**`#![no_std]` + `alloc` + `forbid(unsafe_code)`；唯一直接依賴是停用預設功能的 `serde`（只開 `derive` + `alloc`；待使用者確認 P7）**；時間只經 `Clock` trait。
> - 下一步：跑 `cargo xtask accept core`，看純邏輯 demo 與 crate 邊界檢查。

## 負責

- 所有 crate 共用型別（`model`）：backend、team、task、送達狀態、branch 命名空間
- 兩套有版本的協定定義：client 與 holder；JSON Lines hello、版本協商、未知 variant 相容、PTY bytes 的 base64 欄位
- 邊界 traits：`Driver`、`Forge`、`Store`、`Runtime`、`Runner`、`Notifier`、`Clock`
- 純函式 pipeline：六種關卡、task 關係與操作、workflow 存檔檢查、`{pr}`／`{head}`／`{branch}` 展開、`step(state, event)` 狀態機
- 純函式 policy：busy、去抖動、檔案衝突、merge 門檻、分派與 team wait-cycle 偵測
- 螢幕 hard-gate 分類器；規則資料須附版本化 prompt 證據，完整 holder 畫面逐 backend 補齊
- `config.toml` 結構與安裝規則：只有資料與純函式，I/O 由呼叫端負責

## 不負責

- 讀寫檔案、環境變數、socket、子程序，或提供 JSON codec／transport（serde 只定義資料序列化）
- 執行關卡；daemon 的 `pipeline` 依 action 執行副作用
- 讀取時鐘或自行判斷 busy／idle；時間與結構化事件由呼叫端傳入
- 自動按螢幕提示的按鍵；classifier 只回報分類，holder 只接受單一控制鍵

## 模組

| 模組 | 職責 |
|---|---|
| `model` | 共用型別與 branch／worktree 命名 |
| `protocol` | client／holder 型別、hello 與版本協商 |
| `traits` | 外部邊界契約，不含 adapter 實作 |
| `pipeline::stage` | 六種關卡與 fanout join |
| `pipeline::task` | task 關係、workflow 版本 pinning 與操作 |
| `pipeline::workflow` | typed workflow、內建 workflow、存檔檢查（D19） |
| `pipeline::state` | 純函式 `step` 與 side-effect actions；head 變更不讓 task 前進（work 中只記錄 head）；要求修改與 check 失敗退回最近的 work（返工回原作者）；取消；fanout `all`／`first`／`pick` join 和選擇 |
| `policy::busy` | `BusyLevel`、`effective_level` |
| `policy::debounce` | busy 立即生效；idle 穩定 5 秒 |
| `policy::conflict` | 檔案重疊偵測 |
| `policy::merge_gate` | merge 門檻的唯一實作（每個 command 與 approval 關卡一個 fact）與 patch-id 保留（D14） |
| `policy::assign` | D18/D25 角色分派、role instance headcount、臨時 instance 決定與等待循環 |
| `screen` | 以 fixture 支持的規則分類 hard gate |

## 依賴規則

- `#![no_std]` + `alloc`；草稿允許唯一依賴 `serde`，`default-features = false`，只開 `derive` + `alloc`；這是待使用者確認的 P7，否決則移除
- `serde` 只 derive protocol 與 workflow 定義型別（workflow 以 TOML 存 DB，D19）；`Task`、`PipelineState` 等執行期型別不 derive；不使用 `serde_json`、transport、clock 或 runtime
- 沒有 `[features]`、build script、unsafe；錯誤型別使用 `core::error::Error`
- 時間只由 `Clock` 傳入；集合用 `BTreeMap`／`BTreeSet`

| 保護 | 擋下什麼 | 工具 |
|---|---|---|
| 對無 std target 編譯 core，並帶 `-F unsafe-code` | std／I/O、FFI、以及會用 std 的依賴 | `cargo xtask check-deps`（需要 `thumbv7em-none-eabihf`） |
| `cargo metadata` 檢查 | build script、任何 crate feature、非 `serde` 直接依賴、serde 預設功能或 derive／alloc 以外的 feature | `cargo xtask check-deps` |

威脅模型：這些保護擋意外把 I/O 帶進 core；刻意改 allowlist 或 xtask 由 code review 把關。

## 入口

- `agend_core::protocol::{client, holder}`
- `agend_core::pipeline::state::{step, PipelineState, PipelineEvent}`
- `agend_core::policy::{assign, debounce, merge_gate}`
- `agend_core::screen::classify`

## 下一步

```bash
cargo test -p agend-core
cargo xtask accept core
```
