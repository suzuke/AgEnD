# agend-core

> **TL;DR**
> - 純邏輯 crate：共用型別、協定、traits、workflow 狀態機、policy 與螢幕分類器。
> - 記住：**`#![no_std]` + `alloc` + `forbid(unsafe_code)`；唯一直接依賴是停用預設功能的 `serde`（只開 `derive` + `alloc`；D32）**；時間只經 `Clock` trait。
> - 下一步：跑 `cargo xtask accept core`，看純邏輯 demo 與 crate 邊界檢查。

正式 P5 啟動按鍵與 P6 初始 idle 的實作、schema v9 與 native 驗證邊界見 [啟動處理](../../docs/gates/gate-12a-startup-runtime.md)。

2026-10-06 使用者確認 [D41](../../docs/decisions/d41.md)：Ready 唯一完整單行的 `Try "…"` 建議文字可變。
其餘完整畫面 tokens、Claude 2.1.284、100／140×24、建議列位置與 canonical workspace 仍核對；
空白／重複／未閉合／控制字元／換行／超長建議拒絕；TrustNo／TrustYes／Development 不放寬。
兩份 v5 真捕獲只遮 workspace，供 core 與 native 回歸；[來源與 SHA](tests/fixtures/screens/README.md)。
歷史 literal 補錄與 v5 FAILED 仍保留；原生通過不等於完整真模型 smoke 通過。

## 第 10 施工關（已驗收，2026-10-02）

`PipelineSnapshot` 不含 workflow，只有驗證後的 `restore` 才能回到可執行狀態；`outstanding_actions` 重建原 ticket；`TaskStatus` 含 Failed／Cancelled。binding snapshot 型別共用於 core；Store 的 `advance_task` 同交易存 task、快照、受阻理由與 event。

## 第 11 施工關 C 段（已驗收並合併 #145）

`protocol::terminal` 提供 no-std 的 cells／色彩／cursor／mode／viewport／frame 型別。holder 協定 1.1 新增 `GetTerminalFrame`、附 request id 的 `TerminalFrame` 與 `TerminalOperationError`；1.0 請求／純文字快照保持原 wire shape。holder 的 `TerminalControl` 提供 generation／owner 與實際 resize／input 完成回覆；client 1.4 新增檢視訂閱／viewport／控制、完成回覆與失去控制通知；Acquire 不接受 caller 自訂 attach id。第 11C 當時 client 提供 1.4／1.3；第 12A 已增加 1.5。注入 terminal producer 的 fake 保持 1.4。`traits::TerminalProducer` 是同步畫面／完成控制／legacy input port，server 在背景排程；core 只有介面與協定型別。端到端基礎路徑與六項 fake／真 C 契約已建立，TUI 與完整 fake U17 已有回歸，真 Codex 0.159.3 首次 U17 已核實；完整 C 段已驗收並經使用者確認合併（#145，2026-10-03）。CodexInputPolicy::approved() 只辨識 codex-cli 0.159.3；holder 身分與 thread 歸屬由 daemon 核對。

`PipelineView::replace_attention_if` 定義原子條件更新：捕捉值仍相同才 replace／publish，移除或已變更就拒絕；core 僅定義 port，Fleet 實作鎖。用於避免 Retry 與 failed-item enrichment 交錯時重建舊項目。

`policy::codex_input` 的 `approved()` 只允許精確 `codex-cli 0.159.3`；未知或其他版本拒絕人工輸入。daemon 核 holder 啟動紀錄與 thread 身分，曾允許人工輸入的 thread 永久只用自己的 clientId 對帳。診斷政策另限一個明確 instance，不改正式許可。見 [版本政策](../../docs/gates/gate-11c-codex-input.md) 與 [U17 證據](../../docs/gates/gate-11c-u17-validation.md)。

## 第 12A 持久化基礎（實作中）

`runtime_records::claude` 定義 `ClaudeDelivery`、`ClaudeAttempt`、`ClaudeAck`、`ClaudeReservation` 與 `DriverEvent` 等共用資料；`may_start`／`outcome_unknown` 保留既有四種 DeliveryState。只有新的 `Started` 可開始 transport，`Existing` 不授權重送；core 不讀時鐘或 DB。見 [store 範圍](../../docs/gates/gate-12a-store.md)。

## 第 13A home／初始設定

`setup::DEFAULT_HOME_DIRECTORY` 與 `INITIAL_CONFIG` 定義操作員預設目錄名稱和不含秘密的初始設定。環境變數解析、私有目錄建立與原子發布都由 `agend` 執行；core 不讀寫檔案。`setup::service::ServiceSpec` 產生 launchd／systemd 定義並拒絕控制字元與非絕對路徑，服務執行環境只列 HOME、AGEND_HOME 與 PATH。

## 負責

- 所有 crate 共用型別（`model`）：backend、team、task、送達狀態、branch 命名空間
- 兩套有版本的協定定義：client（1.1：全貌、「需要你」的操作、`hello` 的 `caller`、錯誤碼 `client::error_code`、事件游標規則；1.2（第 9 施工關）：`operator` 請求、`send` 的 `level` 與 `message_id`、`status` 的 `identity`、`hello` 的 daemon 版本／pid／`boot_id`、instance 的 `working_directory`、ticket `<task>/<stage>/<attempt>`、UUID v4）與 holder；JSON Lines hello、版本協商、未知 variant 相容、PTY bytes 的 base64 欄位
- pipeline 的 core ports：`PipelineStore`、`PipelineExecutor`、`PipelineView`，以及共用 `runtime_records`；Engine 可注入真 adapter 或 fake；executor 的 `clean_worktree` 必須同時檢查實際修改與會隱藏修改的 index 旗標。
- 邊界 traits：`Driver`、`Forge`、`Store`、`Runtime`、`Runner`、`Notifier`、`Clock`、`TerminalProducer`
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
| `protocol::ask` | 對話式請示 thread（選項或自由文字、追問、結論，D35）與 context recap 型別（D37） |
| `traits` | 外部邊界契約，不含 adapter 實作 |
| `pipeline::stage` | 六種關卡與 fanout join |
| `pipeline::task` | task 關係、workflow 版本 pinning 與操作 |
| `pipeline::workflow` | typed workflow、內建 workflow（`code`、`research`、`epic`、`planned`）、存檔檢查（D19）；`Workflow::validated` 產生唯一能建 pipeline 的 `ValidatedWorkflow` |
| `pipeline::state` | 純函式 `step` 與 side-effect actions；事件身分：結果必須帶目前的關卡、attempt（綁 head 的關卡另帶 head），否則 `StaleResult`；head 變更不讓 task 前進（work 中只記錄 head）；要求修改與 check 失敗退回最近的 work（返工回 task 持有者）；取消（merge 送出後不可取消）；欄位私有、只讀 accessor；fanout `all`／`first`／`pick` join 和選擇 |
| `policy::busy` | `BusyLevel`、`effective_level` |
| `policy::debounce` | busy 立即生效；idle 穩定 5 秒 |
| `policy::conflict` | 檔案重疊偵測 |
| `policy::attention` | 請示排序（D36）：放行最多工作的在前，再看等待時間，最後 id |
| `policy::merge_gate` | merge 門檻的唯一實作（每個 command 與 approval 關卡一個 fact）與 patch-id 保留（D14） |
| `policy::assign` | D18/D25/D33 角色分派：一個 agent 一個 task、返工回 task 持有者、額度用盡或被刪才交接、role headcount 內開臨時 instance、臨時 instance 回收條件、等待循環 |
| `screen` | 以 fixture 支持的規則分類 hard gate |

## 依賴規則

- `#![no_std]` + `alloc`；唯一依賴 `serde`，`default-features = false`，只開 `derive` + `alloc`（D32）
- `serde` derive protocol、workflow、binding／runtime records 與 `PipelineSnapshot` 的資料型別；workflow 以 TOML 存 DB（D19）。`Task`、`PipelineState` 本身不 derive serde；snapshot 必須經 `restore` 驗證後才可執行；不使用 `serde_json`、transport、clock、tokio runtime 或 agent runtime
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

## 第 12A protocol 1.5

`protocol::client::claude` 提供 `ClaudeRequestData`／`ClaudeOperation`、plain `ClaudeReplyData`、`ClaudeReceipt` 及 `ClaudePendingRecord`；只有資料、無 I/O 或新依賴。一般 client 1.3、完整終端 1.4 保持能力底線。精確欄位與本批限制見 [bridge 基礎](../../docs/gates/gate-12a-bridge.md)。

## Claude 2.1.284 啟動畫面證據

`screen::tests::claude_2_1_284_holder_trust_frames_remain_hard_gates_at_both_widths`
以真 daemon／holder 的 100×24、140×24 信任畫面核對既有分類器；只回 StartupMenu，
沒有 suggested key。舊 spike 片段保留；新檔來源與 SHA-256 見
[fixture 紀錄](tests/fixtures/screens/README.md)。未認證自動按鍵、初始 idle 或完整 12A。

受控蒐證另保存兩寬選到 Yes 與 development channels 的真 frame；
`claude_2_1_284_selected_trust_and_development_channels_remain_hard_gates` 核四個真 fixture
都為 Claude StartupMenu，其他 backend 不匹配、沒有 suggested key。
原規則對 development channels 回 None 的反例已保存，新規則補上已觀察標頭。
授權只完成 workspace 信任，沒有確認 development channels，不宣稱 P5／P6 完成。

## GitHub 遠端身分

`github` 提供 `GithubIdentity`、`GithubChange` 與 `GithubStore` 邊界。純狀態規則禁止更换 repository／PR、覆蓋未確認 push intent 或清除 create／cleanup attempt；完成 cleanup 後 ledger 不可改動。正式 Forge 已接線，12C 尚待真測驗收。

Workflow submit 只接受 local／github，且同一 workflow 不可混用；`forge_kind()` 供 pipeline 的後續 checks／merge／恢復選擇。

12D `config` 定義 Telegram 的 secret reference、chat／sender allowlist 與 topic；pure core 只驗值，不讀檔或持有 token。daemon 啟動讀取設定，錯誤設定在啟動 holder 前拒絕。

12D `telegram` 定義 immutable delivery、逐段 claim／receipt 邊界與完整文字分段；所有 I/O 由 daemon 實作。

Protocol 1.6 新增共用已讀收據：`mark_attention_read`、`attention_read` 事件與 fleet `read_keys`。識別沿用事項 ID＋問題次數；後續追問重新未讀。daemon 保存 SQLite，TUI 與 Telegram 共用；已讀不等於回答、核准或解除。舊 daemon 仍使用 TUI 本機已讀。

Telegram team topic 保存目前任務摘要（任務、狀態與階段）；needs-you topic 保留完整請示與操作按鈕。摘要按內容對帳，重啟不重送；未 claim 的輔助通知可恢復，in-flight 未知結果不重送。既有通知綁定原 destination，改 topic 不會自動搬移舊通知。

Telegram delivery 區分 in_flight 與 outcome_unknown，後者供本機 `telegram-delivery:<id>` 處置。操作員 Abandon 保存理由、原文及未知收據前綴，不確認送達、不重送；一般 agent 不可操作。daemon 開機在取得 DB 後恢復未確認意圖，本機處置不依賴 token；此類通知不經 Telegram 再投遞。

13C `setup::backend` 定義匯入內容 manifest 與版本名稱驗證，沒有檔案或程序 I/O；消費端與限制見[版本管理](../../docs/architecture/backend-versions.md)。

client 1.7 的 `OperatorCommand::MessageDelivery`／`MessageDeliveryData` 只表達持久化 delivery 狀態與 identity，不攜帶 body；未知狀態保留 Unknown。I/O 與權限由 daemon／client 實作。

13C 施工中的 protocol 1.7：操作員 `send_message` 固定以 `@operator` 真人身分 queue 投遞，必填 UUID v4；`driver_status` 回傳 instance 與就緒狀態，Codex 必須有連線，unknown 不代表 idle。這些 RPC 不切換 backend 版本。

holder 協定 1.3 加入 `SpawnBound`／`GetLaunchBinding` 與不含 argv／環境的 `LaunchBindingData`，供第 13 關受管啟動對帳；保留 1.2／1.1 協商。

受管啟動紀錄 `ManagedLaunchIntent` 保存不透明 UUID、匯入 artifact 與設定／實際啟動參數，供 store、supervisor 和 runtime 共用；紀錄本身不代表 holder 存活或准入完成。

13C `runtime_records::BackendSwitch` 保存明確版本切換的來源啟動意圖、目標 artifact、原生 session 與 phase；純序列化記錄，不判定 canary、程序或檔案狀態。
