# agend-core 測試

> **TL;DR**
> - 測純函式、no-std 型別與 workflow 狀態機；狀態機另有固定種子的事件序列探索器（property test，不加依賴）；protocol wire shape 在 xtask integration tests 驗證。
> - 記住：Codex fixture 是 PTY 擷取；Claude fixture 是 spike 紀錄中的 prompt 文字，並非完整 holder 畫面擷取。
> - 下一步：跑 `cargo xtask accept core`，比對實際狀態機 transcript。

## 怎麼跑

```bash
cargo test -p agend-core
cargo xtask accept core
```

`accept core` 會跑 workspace fmt、workspace clippy、core tests、xtask protocol compatibility tests、workflow TOML golden tests、`check-deps`，再啟動 `agend-core` 的 `core_demo` example。demo 實際呼叫 protocol hello、assignment、busy、debounce 與 pipeline 狀態機；JSON wire shape 由 `cargo test -p xtask --test protocol_compat` 驗證。

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `model::tests` | backend 名稱與 delivery 狀態轉換；branch producer／consumer 往返 |
| `protocol::tests`、`protocol::client::tests`、`protocol::holder::tests` | major 不相容會有明確錯誤；daemon 可與仍支援的舊 holder major 協商 |
| `xtask/tests/protocol_compat.rs` | exact JSON wire shape、unknown tagged variants、忽略 additive fields；approval request 不接受 caller 指定 head |
| `xtask/tests/workflow_toml.rs` | workflow 存檔 TOML 格式的 golden 檔（內建四個與一個自訂），鎖住 serde 形狀（D32 的條件） |
| `pipeline::task::tests` | workflow 版本固定、reopen／supersede／關係檢查 |
| `pipeline::workflow::tests` | 三個內建 workflow、repo 要求、角色、approval、on_fail 驗證；佔位符不可加引號、要有來源關卡；merge 前的 command 不可在最後一個 work 之前 |
| `pipeline::state::tests` | code stage 轉換、失敗與要求修改都回最近的 work（返工回原作者）、work／submit 期間 head 變更不改關卡、D14 核准保留、每個 approval 關卡都要覆蓋目前 head 才能 merge、`RunCommand` 帶展開後的指令與 change id、取消是獨立狀態 |
| `tests/pipeline_explorer.rs` | 固定種子的事件序列探索器，見下方「狀態機探索器」 |
| `policy::assign::tests` | D33：持有 task（含審查）的 agent 不再接其他 task；全員都持有時在人數上限內開臨時 instance，否則排隊；返工回持有者；持有者額度用盡改派另一個 backend 並交接 branch 與意見；持有者被刪立即改派；臨時 instance 在 task 結束前不回收。另有 reviewer 優先跨 backend（只有同一個 backend 時仍開臨時 instance）、額度轉派、缺角色轉 ask、wait cycle |
| `policy::busy::tests` | codex steer；claude／opencode steer 退成 interrupt；queue 不變 |
| `policy::debounce::tests` | busy 立即生效，idle 穩定 5 秒，busy 會取消待定 idle |
| `policy::conflict::tests` | pairwise 重疊檔案排序與去重 |
| `policy::merge_gate::tests` | merge 門檻唯一實作：每個 command 關卡對目前 head 通過、每個 approval 關卡都覆蓋目前 head（不綁 head 的只需存在）、`allow_unreviewed` 只免除「至少一個綁 head 的核准」；rebase 以 patch-id 判斷保留 |
| `screen::tests` | backend-specific startup prompt 片段比對，不分類一般畫面；Claude 的片段不是完整 holder 擷取 |

## 狀態機探索器

`tests/pipeline_explorer.rs` 是不加依賴的 property test（core 的依賴與 dev-dependency allowlist 不變）：xorshift 固定種子產生事件序列，跑在 10 個 workflow 上（內建 `code`、`research`、`epic`，epic 的 `first`／`pick` 變體，以及 5 個自訂：兩個 approval、review 夾在兩個 command 之間、`allow_unreviewed`、command 在 submit 之前加 `count = 2` 與不綁 head 的 approval 和 `on_fail`、`{pr}`／`{head}` 佔位符）。

| 項目 | 數量 |
|---|---|
| 每個 workflow 的序列 | 4,000（`AGEND_EXPLORER_SEQUENCES` 可調大） |
| 每條序列最多步數 | 60；進入終止狀態後再送 3 個事件，必須全被拒絕 |
| 事件組成 | 約 6 成是目前關卡的合理事件、1.5 成 head 變更、2 成過期或偽造的結果、其餘是失敗、逾時、取消 |
| 竄改狀態測試 | 每個 workflow 2,000 個竄改過的狀態（任意 stage index、status、head、紀錄）× 8 個事件 |

每一步之後，用**只看被接受的事件**建立的 oracle（不讀 state 自己的紀錄）檢查：

1. merge 門檻：`Merge` action、`MergeCompleted` 與沒有 merge 的 workflow 的 `Done`，都要每個 command 關卡對目前 head 通過、每個 approval 關卡有足夠的核准覆蓋目前 head（D14：乾淨且 patch 相同的 rebase 讓核准延續到新 head）。
2. 不跳關：關卡只會因目前關卡自己的完成事件前進，中間只能跳過已滿足的 approval；過了 submit 之後一定收過 `Submitted`。
3. 返工不遺失：work 期間 head 變更不改關卡、不發 action；要求修改與 command 失敗會退回（預設最近的 work），不會讓 task 失敗；head 變更永遠不讓 task 前進。
4. 核准與結果只算給它所屬的 head 與關卡。
5. 終止狀態（done、failed、cancelled）不接受任何事件。
6. `step` 不 panic（包括竄改過的狀態）；竄改狀態下 `MergeCompleted` 被接受時，紀錄必定覆蓋每個 command 與 approval 關卡。

失敗訊息附 workflow、種子與完整事件序列，可重現。另外確認過探索器抓得到 review 找到的錯：把 N1／N2（head 變更往前跳）、N2（work 走 D14 路徑）、N3（要求修改讓 task 失敗）、N4（門檻不看不綁 head 的 approval）、B3（command 結果不綁 head）、approval 不綁 head 逐一放回程式，探索器都會失敗。

各 review 的 probe 情境另有具名回歸測試，名稱以 `review_` 開頭（`review_n1_…`、`review_b3_…` 等），分布在 `pipeline::state`、`pipeline::workflow`、`policy::assign` 的測試。

## Protocol JSON Lines

core 只提供 serde 型別，不含 JSON codec。`xtask/tests/protocol_compat.rs` 用 serde_json 測已知訊息的 wire shape、未知 tag 的 payload 忽略，以及同 major 的 additive-field 規則。`ReviewApprove` 只帶 task id；daemon 依 authenticated review binding 綁定 head。

PTY bytes 在 protocol 型別中使用 `bytes_base64` 欄位；base64 實際編碼和解碼由 holder／client adapter 負責。

## Crate 邊界

- `cargo xtask check-deps` 以 no-std target 編譯 core，檢查 `unsafe-code` 與依賴 allowlist。
- core 沒有 testkit 依賴。所有單元測試都在純資料與純函式上執行。
- screen fixture 來源與證據等級見 `tests/fixtures/screens/README.md`。除 Codex 外，Claude 目前只有 spike prompt 片段；新規則需附 holder 真實畫面與 backend 版本證據。

## 下一步

```bash
cargo xtask accept core
```
