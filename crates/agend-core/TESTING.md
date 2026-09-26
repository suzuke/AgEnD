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
| `protocol::tests`、`protocol::client::tests`、`protocol::holder::tests` | major 不相容會有明確錯誤；daemon 可與仍支援的舊 holder major 協商；client hello 是 1.1、1.0 的 peer 協商到 1.0；「需要你」依 D36 排序、沒有 `attention_id` 的排最後（第 8 施工關） |
| `protocol::ask::tests` | 請示 thread 接受提供的選項或自由文字；等待追問時才接受回答；有結論後不再接受（D35） |
| `xtask/tests/protocol_compat.rs` | exact JSON wire shape、unknown tagged variants、忽略 additive fields；approval request 不接受 caller 指定 head；請示 thread、自由文字回答、context recap 的形狀，以及 D35 之前的舊訊息仍能解碼 |
| `xtask/tests/workflow_toml.rs` | workflow 存檔 TOML 格式的 golden 檔（內建四個與一個自訂），鎖住 serde 形狀（D32 的條件） |
| `pipeline::task::tests` | workflow 版本固定、reopen／supersede／關係檢查 |
| `pipeline::workflow::tests` | 四個內建 workflow（含 D34 `planned`）、repo 要求、角色、approval；佔位符不可加引號、要有來源關卡；merge 必須最後；command 與綁 head 的 approval 必須在最後的 branch work 之後；`on_fail` 只能指向前面的 work；command／approval 前面必須有 work；只有 `validated` 過的 workflow 能建 pipeline |
| `pipeline::state::tests` | code stage 轉換、失敗與要求修改都回最近的 work（返工回 task 持有者）、work／submit 期間 head 變更不改關卡、D14 核准保留、每個 approval 關卡都要覆蓋目前 head 才能 merge、`RunCommand` 帶展開後的指令與 change id、取消是獨立狀態、merge 送出後不能取消且 head 變更只記成待處理（`MergeFailed` 才套用）；竄改狀態測試（只有 crate 內能造出竄改狀態） |
| `tests/pipeline_explorer.rs`、`tests/pipeline_explorer_splitmix.rs` | 兩個獨立的事件序列探索器，見下方「狀態機探索器」 |
| `policy::assign::tests` | D33：持有 task（含審查）的 agent 不再接其他 task；全員都持有時在人數上限內開臨時 instance，否則排隊；返工回 task 持有者；task 持有者額度用盡改派另一個 backend 並交接 branch 與意見；被刪、換了 team、持有別的 task、backend 不再允許時立即改派（角色不存在則轉 ask）；臨時 instance 在 task 結束前不回收。另有 reviewer 優先跨 backend（只有同一個 backend 時仍開臨時 instance）、額度轉派、缺角色轉 ask、wait cycle |
| `policy::busy::tests` | codex steer；claude／opencode steer 退成 interrupt；queue 不變 |
| `policy::debounce::tests` | busy 立即生效，idle 穩定 5 秒，busy 會取消待定 idle |
| `policy::conflict::tests` | pairwise 重疊檔案排序與去重 |
| `policy::attention::tests` | 請示排序：放行最多工作的在前，同樣時等最久的在前，全同時以 id 決定（D36） |
| `policy::merge_gate::tests` | merge 門檻唯一實作：每個 command 關卡對目前 head 通過、每個 approval 關卡都覆蓋目前 head（不綁 head 的只需存在）、`allow_unreviewed` 只免除「至少一個綁 head 的核准」；rebase 以 patch-id 判斷保留 |
| `screen::tests` | backend-specific startup prompt 片段比對，不分類一般畫面；Claude 的片段不是完整 holder 擷取 |

## 可完成證明（存檔檢查裡）

`Workflow::validate` 在靜態規則之後呼叫 `pipeline::state::completability_witness`：用純函式 `step` 走固定的事件序列——全部成功一次、每個 command／approval 各失敗一次後再成功、產出 branch 之後每個關卡各收到一次新 commit 後再成功——每一趟都要在上限內到 done，否則回 `WorkflowError::NotCompletable { scenario, stage_id, reason }`。它是存檔規則的一部分，所以所有測試裡的 workflow 都先經過它。

## 狀態機探索器

`tests/pipeline_explorer.rs` 是不加依賴的 property test（core 的依賴與 dev-dependency allowlist 不變）：xorshift 固定種子產生事件序列，跑在 13 個 workflow 上（內建 `code`、`research`、`planned`、`epic`，epic 的 `first`／`pick` 變體，以及 7 個自訂：pick fanout 在 checks 之後再 review、沒有 merge 但有 checks 與綁 head 的 approval、兩個 approval、review 夾在兩個 command 之間、`allow_unreviewed`、command 在 submit 之前加 `count = 2` 與不綁 head 的 approval 和指向 work 的 `on_fail`、`{pr}`／`{head}` 佔位符）。

| 項目 | 數量 |
|---|---|
| 每個 workflow 的序列 | 4,000（`AGEND_EXPLORER_SEQUENCES` 可調大） |
| 每條序列最多步數 | 60；進入終止狀態後再送 3 個事件，必須全被拒絕 |
| 事件組成 | 約 6 成是目前關卡的合理事件、1.5 成 head 變更、2 成過期或偽造的結果、其餘是失敗、逾時、取消 |
| 竄改狀態測試 | 在 crate 內（`pipeline::state::tests`，外部無法偽造 state）：5 個 workflow × 3,000 個竄改狀態 × 14 個事件 = 210,000 次 |
| 事件身分 | `tests/event_identity.rs`：verifier r4 的反例寫成 `verifier_r4_*` 回歸測試（merge 送出中收到關卡失敗、未滿人數的挑選撐過 head 變更、上一輪 fanout 的完成、重複的 work 完成），以及 action 帶身分、沒有 branch 的 workflow 拒收新 commit、挑選清單不列已取消的子 task。兩個探索器都會重送先前被接受的結果、送過期或未來的 attempt，並檢查「身分不符的事件從不被接受」與「merge 送出中只接受它的結果與 head 變更」；可完成證明在每個關卡往前走之後重送同樣的結果，必須被拒絕。把 r4 的四個問題逐一放回程式，都有測試失敗。verifier r5 的反例寫成 `verifier_r5_*`（head 變更在 approval／fanout 關卡開新的 attempt、目前 attempt 的部分核准重播不改狀態、merge 送出中 branch 重設丟棄待處理變更）；兩個探索器與可完成證明另檢查「關卡內作廢一定開新 attempt」，拿掉這條規則會被抓到 |
| 可完成性 | `tests/workflow_completability.rs`：存檔檢查的可完成證明（見下方）之外的獨立檢查。verifier r2、r3 的反例寫成 `verifier_r2_*`、`verifier_r3_*` 回歸測試；隨機 workflow 產生器預設 200,000 個 workflow（`-- --ignored` 再跑 1,000,000 個，最近一次接受 54,696 個），每個被 `validate` 接受的都要能用獨立的成功事件驅動器走到 done，隨機干擾（失敗、要求修改、新 commit、main 前進、merge 失敗、逾時）之後也要。拿掉可完成證明與新文法的舊 validate 會讓它失敗 |
| 死路探索器 | `tests/pipeline_deadend_explorer.rs`（`--ignored`，建議 `--release`，約 5 分鐘）：verifier r3 寫的 PCG32 產生器與廣度優先搜尋，對每個被接受的 workflow 探索有限次 head 變更與失敗，確認每個可達狀態都還能只靠成功事件走到 done；產生 60,000 個 workflow、存檔檢查接受 6,948 個、深入探索 1,504 個、約 846 萬個狀態（1,373 個 workflow 碰到狀態上限被截斷）、0 死路、0 違反（2026-09-25）。merge 送出中 branch 重設的例外只允許「除了待處理清單之外整個狀態不變」（`PipelineState::without_pending_head_changes`），重設時連帶清掉挑選或 change id 的改動會被抓到 |
| 第二個探索器 | `tests/pipeline_explorer_splitmix.rs`：fresh-context verifier 寫的 SplitMix64 探索器，8 個 workflow × 3,000 條序列 × 最多 80 步（`AGEND_EXPLORER2_SEQUENCES` 可調大）；它的 oracle 只算「前一個 work 最近一次完成之後」的 check 與核准 |

每一步之後，用**只看被接受的事件與 `ReturnToWork`** 建立的 oracle（不讀 state 自己的紀錄；返工時忘掉退回的 work 之後所有關卡的 check 與核准，只有 D14 能延續核准）檢查：

1. merge 門檻：`Merge` action、`MergeCompleted` 與沒有 merge 的 workflow 的 `Done`，都要每個 command 關卡對目前 head 通過、每個 approval 關卡有足夠的核准覆蓋目前 head（D14：乾淨且 patch 相同的 rebase 讓核准延續到新 head）；最後一個 pick fanout 的勝出者是它目前的子 task（fanout 重跑後要重新挑）。沒有 merge 的 workflow 也成立，因為存檔檢查不允許在最後的 branch work 之後再有 work，可完成證明在 done 時也檢查這兩點。
2. 不跳過關卡：關卡只會因目前關卡自己的完成事件前進，中間只能跳過已滿足的 approval；過了 submit 之後一定收過 `Submitted`。
3. 返工不遺失：work 期間 head 變更不改關卡、不發 action；要求修改與 command 失敗會退回（預設最近的 work），不會讓 task 失敗；head 變更永遠不讓 task 前進。
4. 核准與結果只算給它所屬的 head 與關卡。
5. 終止狀態（done、failed、cancelled）不接受任何事件；merge 送出後只接受它的結果與 head 變更。
6. `step` 不 panic（包括竄改過的狀態）；竄改狀態下 `MergeCompleted` 被接受時，紀錄必定覆蓋每個 command 與 approval 關卡。
7. 事件身分：被接受的結果一定是目前關卡、目前 attempt（綁 head 的關卡還要目前 head）的；重送先前被接受的結果一律被拒；merge 送出中只接受它的結果與 head 變更。
8. 關卡內作廢一定開新 attempt：已收的部分核准或暫定挑選在原關卡被清掉、head 變更到達 approval 或 fanout 關卡、逾時改派，之後的 attempt 一定比之前大。

失敗訊息附 workflow、種子與完整事件序列，可重現。另外確認過探索器抓得到 review 找到的錯：把 N1／N2（head 變更往前跳）、N2（work 走 D14 路徑）、N3（要求修改讓 task 失敗）、N4（門檻不看不綁 head 的 approval）、B3（command 結果不綁 head）、approval 不綁 head、返工時保留核准（verifier I3）、merge 送出後允許取消逐一放回程式，至少一個探索器或竄改狀態測試會失敗（N4 由竄改狀態測試抓到）。

各 review 的 probe 情境另有具名回歸測試，名稱以 `review_` 開頭（`review_n1_…`、`review_b3_…` 等）；fresh-context verifier 的情境以 `verifier_` 開頭；分布在 `pipeline::state`、`pipeline::workflow`、`policy::assign` 的測試。

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
