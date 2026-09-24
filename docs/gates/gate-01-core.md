# 第 1 施工關：agend-core（`core`）

> **TL;DR**
> - 純邏輯 crate：型別、兩套協定（含對話式請示）、trait、流水線狀態機、busy policy、去抖動、衝突偵測、merge 門檻、分派、請示排序、螢幕分類器。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：fresh-context verifier 重跑並嘗試推翻；之後你親自驗收。

## 狀態

**實作中**（2026-09-25）

P1–P7 已由你在 2026-09-25 確認（記為決策 D26–D32）。實作草稿是在確認之前寫的；2026-09-24 的勾選是草稿作者自己打的，不算確認。

## 範圍

- 型別、client 與 holder 兩套協定、trait（`Driver`、`Forge`、`Store`、`Runtime`（agent runtime）、`Runner`、`Notifier`、`Clock`，依 P2／P3）
- 流水線狀態機（6 種關卡、task 關係與操作、workflow 存檔檢查）；內建 workflow `code`、`research`、`epic`，以及 2026-09-25 加入的 `planned`（D34）
- policy：busy、去抖動、衝突偵測、merge 門檻（patch-id）、分派（D25、D33）、請示排序 `policy::attention`（D36，2026-09-25 加入）
- 請示（needs-you）協定型別：選項或自由文字回答、多輪 thread（D35），以及附帶的 context recap 型別（D37）；2026-09-25 加入
- 螢幕分類器與規則資料
- `cargo xtask accept core` 的 demo

## 開工前提案

每一項都要你逐條確認（勾選）。7 項已於 2026-09-25 確認，對應決策 D26–D32（[DECISIONS](../DECISIONS.md)）。

### P1：wire format 與版本協商

- 問題：client protocol 與 holder protocol 用什麼格式、怎麼處理版本？
- 建議：兩套協定都用 unix socket 上的 JSON Lines；PTY 位元組以 base64 放在 JSON 裡。連線第一則訊息是 `hello`，交換協定版本；major 不同 → 印出清楚的錯誤；同一個 major 內只准新增欄位，未知欄位忽略。holder 協定最保守：新 daemon 必須能跟舊一個 major 的 holder 溝通。
- 理由：好除錯（`nc`／`jq` 就看得懂）；holder 很少更新，daemon 升級時舊 holder 還在跑。
- 替代方案：長度前綴的二進位 frame（較快，但難除錯）。
- [x] 使用者確認（2026-09-25）

### P2：trait 簽章

- 問題：`Driver`、`Forge`、`Store`、`Runtime`、`Notifier`、`Clock` 的方法怎麼定？
- 建議：「functional core, imperative shell」：流水線狀態機是純函式 `step(state, event) -> (state, Vec<Action>)`，不呼叫任何 trait。trait 仍放 core（D11），用 `async fn`、不含 tokio 型別（相容 no_std）。每個 trait 只放實際用到的最少方法，例如 Forge：`submit`、`head`、`merge_if_head_is`；Store：每個用途一個方法、寫入帶 CAS 版本，不做通用查詢語言。
- 理由：core 的測試不需要任何假實作；trait 保持小，假實作與契約測試才容易寫。
- 替代方案：狀態機直接呼叫 trait（每個 core 測試都要假實作）。
- [x] 使用者確認（2026-09-25）

### P3：runner 要不要 trait

- 問題：`command` 關卡的 runner 需要自己的 trait 嗎？
- 建議：要：`Runner::run(cmd, dir, timeout) -> Output`。
- 理由：D9 要求每個外部邊界都有 trait + 假實作；`command` 關卡與 git adapter 都要跑程序。
- 替代方案：不設 trait、直接用 `tokio::process`（交接測試的 agent 提出）——否決：不跑真指令就無法測試。
- [x] 使用者確認（2026-09-25）

### P4：GitHub CI 怎麼進流水線

- 問題：Checks 已是 `command` 關卡，GitHub 的 CI 結果怎麼接？
- 建議：用 `command` 關卡，例如 `run = "gh pr checks {pr} --watch --fail-fast"`；`command` 新增佔位符 `{pr}`、`{head}`、`{branch}`。forge 維持 3 個方法，local forge 不需要空實作。
- 理由：一種機制（command）涵蓋本機與 GitHub；forge 保持小。
- 替代方案：由 forge github 回報 CI 狀態（forge 變大，local forge 要有空方法）。
- [x] 使用者確認（2026-09-25）

### P5：去抖動

- 問題：idle↔busy 要穩定多久才生效？
- 建議：不對稱：轉 busy 立即生效（絕不送進忙碌中的 agent）；轉 idle 要穩定 5 秒。先用常數，第 7 施工關用真實資料重新校準。
- 理由：v1 約兩天 75 萬次轉換；送錯時機的代價在「送進忙碌 agent」那一側。
- 替代方案：對稱的 N 秒（兩邊都延遲）。
- [x] 使用者確認（2026-09-25）

### P6：保留期限

- 問題：各類資料留多久？
- 建議：task／workflow／請示紀錄（`agend ask` 建立的 needs-you 請示）永久保留（量小）；訊息 30 天；事件與狀態轉換 14 天；封存的 WIP patch 30 天；每日 DB 快照留 7 份。第 5 施工關重新校準。
- 理由：先給保守可用的預設值，實作 store 時用實際大小修正。
- 替代方案：全部永久保留（v1 home 長到 161G 的教訓）；或全部同一個期限。
- [x] 使用者確認（2026-09-25）

### P7：core 依賴 allowlist

- 問題：protocol 型別要能 derive 序列化，core 可以加依賴嗎？
- 建議：允許 `serde`（+ `serde_derive`），`default-features = false`，只開 `derive` + `alloc`，讓 protocol 型別在 no_std 下 derive；JSON 編碼本身放在 daemon／client。這會改動原本為空的 `CORE_DEP_ALLOWLIST`，所以需要明確核准。
- 理由：兩邊共用同一份型別定義（D11），又不讓 core 碰 I/O。
- 替代方案：在 daemon／client 各自手寫轉換（重複且易漂移）；或 core 保持零依賴、協定型別不 derive。
- 範圍擴充（使用者 2026-09-25 核准，併入 D32）：serde derive 用在 protocol 型別，以及 workflow 定義型別（`Workflow`、`WorkflowStage`、`Stage` 與其欄位型別、`FanoutJoin`）；條件是以 golden TOML 測試鎖住存檔格式（`xtask/tests/workflow_toml.rs`，內建四個 workflow 加一個自訂）。理由：D19 的 workflow 以 TOML 存進 DB、匯出／存回，存檔檢查（`Workflow::validate`）在 core，adapter 需要把 TOML 解成同一份型別，不另寫一份轉換。`Task`、`PipelineState`、`Candidate` 等執行期型別**不** derive：它們的持久化格式屬第 5 施工關 store，屆時再決定。
- 落實位置：`crates/agend-core/Cargo.toml`、`xtask/src/check_core.rs` 的 `CORE_DEP_ALLOWLIST`，規則寫在 AGENTS.md 與 ARCHITECTURE.md（D32）。
- [x] 使用者確認（2026-09-25）


## 待你決定

目前沒有待決定事項。

### Q1：返工時 task 持有者不能接（round-1 review I8）

- 已決定（2026-09-25），記為 [D33](../decisions/d26-d37.md#d33)，已實作在 `policy::assign`：一個 agent 同時只持有一個 task（到 done／取消為止，含等待 checks／review；審查指派是 reviewer 的那一個 task）；返工一定回到持有者；持有者額度用盡 → 改派同角色、另一個 backend 的空成員並交接 branch 與審查意見，否則在人數上限內開臨時 instance，否則排隊（`UsageLimit`）；持有者被刪 → 立即改派或開臨時 instance；臨時 instance 在 task 結束後才回收；等待 fanout 的父 task 照樣佔名額。
- 草稿原本的行為（已取代）：每個 instance 可設定 task 數；task 持有者有額度但沒空位就排隊等它（`AtCapacity`）；task 持有者不在 team 就排隊不改派（`ReworkAuthorUnavailable`）；等待 fanout 的父 task 不佔名額。
- [x] 使用者決定（2026-09-25）

## 待你追認

草稿或夜間步驟先做了、依規則要你事後確認的事項。確認前照目前的做法運作。

- `crates/agend-holder/src/pty.rs`（第 4 施工關的 crate）：隨 `ControlKey` 擴充（Enter、方向鍵、1–3、Y、N 等 11 種），`control_key_bytes` 的按鍵位元組對照與回傳型別改成 `Option`（未知的鍵回 `None`，不送任何位元組）。這是第 4 施工關的設計，改動必要且無害，但按鍵位元組與 `Option` 回傳需要你追認。
  - [ ] 使用者追認
- 夜間步驟（2026-09-25，你睡著時）決定的規則：merge、command、綁 head 的 approval 前面必須有產出 branch 的 work，且須 `requires = ["repo"]`；merge 送出後的 head 變更只記成待處理，等 forge 回報結果（`MergeFailed` 才套用）。
  - [ ] 使用者追認
- 夜間步驟 r2（2026-09-25）：存檔檢查加上**可完成證明**：用 `step` 實際走成功序列、每個 command／approval 各返工一次、每個關卡各來一次新 commit，走不到 done 就拒絕（見 [pipeline 存檔檢查清單](../architecture/pipeline.md#workflow-管理d19d21)）；文法收斂：pick fanout 後面緊接挑選的 approval；有 merge 時最後的 branch work 之後不能再有 work。另外：返工到 fanout 之後的 work 不再清掉 fanout 的子 task；merge 失敗時多筆待處理變更只為最新 head 發一次 checks；branch 出現前的 main 前進直接忽略。
  - [ ] 使用者追認
- 夜間步驟 r3（2026-09-25）：「最後的 branch work 之後不能再有 work」這條規則擴大到所有有 merge、command 或綁 head 的 approval 的 workflow（不只有 merge 的），讓沒有 merge 的 workflow 在 done 時 checks 與核准也涵蓋最後的 head；可完成證明在 done 時檢查這點與 pick 的有效性。fanout 每次重跑都要重新挑 pick，之後的核准作廢。
  - [ ] 使用者追認
- r4（2026-09-25，orchestrator 在你授權的夜間範圍內決定）：**事件身分**。每個結果事件帶要求它的 action 的 `stage_id`、`attempt`（task 每進入該關卡一次加 1），綁 head 的關卡另帶 head；`step` 開頭只有一條規則：身分不是目前的就回 `StaleResult`、狀態不變。merge 送出中只接受這次 merge 的結果與 head 變更，其他一律 `MergeInFlight`。pick 的挑選在人數湊齊時才定下，head 變更時暫定與已定的挑選一起作廢，挑選清單只列還活著的子 task。沒有產出 branch 的 work 的 workflow 拒收新 commit。client protocol 的 agent 結果命令（`done`、`result`、`review approve`／`changes`）新增選填的 `identity`，沒帶的一律當成過期（`stale_result`）；舊訊息仍可解碼。
  - [ ] 使用者追認

## 自動驗收（完成定義）

- [x] `~/.cargo/bin/cargo test --workspace` 通過（163 tests，另有 2 個 `--ignored` 深度測試）；其中 `agend-core` 105 unit tests（含 210,000 次竄改狀態）、兩個狀態機探索器（52,000 + 24,000 條事件序列）、可完成性測試 11 個（含 200,000 個隨機 workflow）、事件身分 6 tests、xtask protocol compatibility 8 tests、workflow TOML golden 2 tests（2026-09-25）
- [x] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨（2026-09-25）
- [x] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`；注入 `std::fs` 時 checker exit 1，還原後通過（2026-09-25）
- [x] `~/.cargo/bin/cargo xtask accept core` 通過，並印出下方 demo（2026-09-25）
- [x] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 跑 acceptance。它會先執行格式、workspace clippy、core tests（含兩個探索器）、protocol compatibility tests、workflow TOML golden tests、check-deps，再跑實際 core demo。

   ```bash
   ~/.cargo/bin/cargo xtask accept core
   ```

   應該看到：demo 先列出實際 protocol hello、busy 與 debounce 結果，再以 `assign::choose` 派出 task 持有者及不同 backend 的 reviewer，並以 `pipeline::state::step` 走完：command 失敗退回 task 持有者、同一個結果再送一次被拒絕（`rejected: result does not belong to the current stage, attempt and head`）、reviewer 要求修改退回 task 持有者、返工期間 main 前進仍留在 work（`stay in work`）、審查中的新 commit 重跑 checks、diff 改變的 rebase 退回 task 持有者、merge 送出後 main 前進只記成待處理（`stay in merge at H4, H5 pending until the merge result`）、forge 回報 merge 失敗後套用 H5（patch 相同，保留核准、只重跑 checks），最後 merge。完整輸出見下方；最後一行是 `gate 1 (core): checks passed`。

   - [ ] 通過

2. 看 busy 等級表（開工時細化：確切的表頭與排版）。

   操作：同一次輸出，往上找 `busy levels for steer`

   應該看到三行：Codex 的 `Steer` 保留；Claude 與 Opencode 的 `Steer` 轉為 `Interrupt`。

   - [ ] 通過

3. 看去抖動行為。

   操作：同一次輸出，找 `debounce`

   應該看到：`idle immediate=false`、`before 5s=false`、`at 5s=true`，且 `busy immediate=true`。

   - [ ] 通過

4. 故意弄壞：讓 core 用到 std，確認被擋。

   ```bash
   backup="$(mktemp)"
   cp crates/agend-core/src/lib.rs "$backup"
   printf '\npub fn leak() { let _ = std::fs::read("/etc/hosts"); }\n' >> crates/agend-core/src/lib.rs
   if ~/.cargo/bin/cargo xtask check-deps; then check_status=0; else check_status=$?; fi
   cp "$backup" crates/agend-core/src/lib.rs
   rm "$backup"
   echo "exit=$check_status"
   ~/.cargo/bin/cargo xtask check-deps
   ```

   應該看到：第一次 no-std 編譯失敗、checker 報 `agend-core does not compile for the no-std target`，`exit=1`；原檔從暫存副本還原後，最後一行回到 `check-deps: ok (… no-std build ok)`。此檢查不連結 agend-core 的 production dependency，因此即使 core 編譯失敗也能啟動 checker。

   - [ ] 通過

5. 看狀態機探索器：隨機事件序列下，merge 門檻、不跳過關卡、返工不遺失都成立。有兩個獨立的探索器。

   ```bash
   ~/.cargo/bin/cargo test -p agend-core --test pipeline_explorer -- --nocapture 2>&1 | grep -E "^explorer|test result"
   ~/.cargo/bin/cargo test -p agend-core --test pipeline_explorer_splitmix -- --nocapture 2>&1 | grep -E "merged|test result"
   ```

   應該看到：第一個指令有 13 行 `explorer <workflow>: 4000 sequences, …`，每行的 `done` 大於 0，有 merge 的 workflow（`code`、`planned`、`human-gate`、`review-between-checks`、`unreviewed`、`pick-then-review`、`checks-before-submit`、`pr-placeholders`）`merged` 大於 0，接著 `explorer total: 52000 sequences` 與 `test result: ok. 1 passed`。第二個指令有 8 行 `<workflow>: merged N reworks M`（M 都大於 0；N 除了沒有 merge 的 `checks-no-merge` 之外都大於 0）與 `test result: ok. 1 passed`。檢查的不變量列在 [crates/agend-core/TESTING.md](../../crates/agend-core/TESTING.md#狀態機探索器)。

   - [ ] 通過

### 預期 transcript

以下是 core example 的實際輸出（不含 accept 前面的檢查指令），逐字元相同；work、assignment、policy 與 pipeline action 均由 core 函式產生。

```text
client hello: supports 1.0
busy levels for steer:
  claude: Interrupt
  codex: Steer
  opencode: Interrupt
debounce: idle immediate=false, before 5s=false, at 5s=true; busy immediate=true, state=Busy
task T-1 workflow=code v1
  assignment -> dev-1
  reviewer   -> review-1
  start                     -> assign role dev (work)
  work                      -> submit via local
  submit                    -> run checks (cargo test) at H0
  command failed            -> return work to its task holder (command `checks` failed with exit code 1)
  same result again         -> rejected: result does not belong to the current stage, attempt and head; nothing changed
  work retry                -> submit via local
  submit                    -> run checks (cargo test) at H1
  command passed            -> request review approval (head-bound=true)
  changes requested         -> return work to its task holder (changes requested by review-1: rename the flag)
  rework, main advanced     -> stay in work at H1b
  rework done               -> submit via local
  submit                    -> run checks (cargo test) at H1c
  command passed            -> request review approval (head-bound=true)
  new commit                -> run checks (cargo test) at H2
  command passed            -> request review approval (head-bound=true)
  changed rebase            -> return work to its task holder (main advanced and the rebase changed the patch)
  work after changed patch  -> submit via local
  submit                    -> run checks (cargo test) at H4
  command passed            -> request review approval (head-bound=true)
  approval H4               -> merge at H4
  main advanced in flight   -> stay in merge at H4, H5 pending until the merge result
  merge failed              -> run checks (cargo test) at H5
  checks rerun              -> merge at H5
  merge completed           -> task done at Some("M1")
```


## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-25 第 1 施工關 verifier r4 推翻（02aca89）：四輪的共同根因是外部結果事件沒有完整身分，改成結構性的**事件身分**：結果帶 stage、attempt、head，`step` 開頭一條規則拒收過期結果；merge 送出中一條規則；pick 人數湊齊才定下；protocol 新增選填 `identity`（stale-by-default）。r4 反例寫成 `verifier_r4_*` 測試，兩個探索器與可完成證明加上重送／過期事件，四個問題放回程式都會被測試抓到。
- 2026-09-25 第 1 施工關 verifier r3 推翻（3e8b3a3）兩個新類型問題，已修（4b05a60）：沒有 merge 的 workflow 在最後的 branch work 之後的 work 裡換 head 仍會 done（改為存檔規則，並加入可完成證明的 done 檢查）；pick fanout 重跑後沿用舊的 pick（進入 fanout 時清掉子 task、挑選與之後的核准，`Fanout` action 帶目前 head）。兩個反例寫成 `verifier_r3_*` 測試，探索器加上對應 workflow 與 pick 不變量，verifier 的死路探索器移植為 `--ignored` 測試。
- 2026-09-25 第 1 施工關 verifier r2 推翻（843a235）：同一類問題（存檔放行、執行走不完）第二次出現，改成結構性解法（ba30886）：存檔檢查用 `step` 做可完成證明、文法收斂、三個執行期修正、隨機 workflow 產生器成為常駐測試（拿掉新規則的舊 validate 會被它抓到）。
- 2026-09-25 第 1 施工關 verifier r1 推翻（832a4dc）後修正（f458545、ebdac60）：merge、command、綁 head 的 approval 前面必須有產出 branch 的 work 且需要 repo；merge 送出後的 head 變更記成待處理、等 forge 結果（新事件 `MergeFailed`）；demo 與 transcript 改走這條路；「作者」改稱 task 持有者；pty.rs 列入「待你追認」。
- 2026-09-25 rebase 到 v2（#104 名詞表，b04c260），文件改用「施工關」等名詞（832a4dc）。
- 2026-09-25 使用者決定 D34–D37（`planned` workflow、對話式請示、請示排序、context recap）並核准 P7 範圍擴充到 workflow 定義型別（條件：golden TOML 測試）；實作與測試（7468ba0、d91865a）。
- 2026-09-25 fresh-context verifier 推翻（REFUTED）幾個窄點，已修（13c0dbc、6ae6364）：merge 必須是最後一個關卡；command 與綁 head 的 approval 必須在最後一個產出 branch 的 work 之後；`on_fail` 只能指向 work，command／approval 前面必須有 work；探索器 oracle 在返工時忘掉紀錄，並加入 verifier 的 SplitMix64 探索器；`PipelineState` 欄位私有、只能由驗證過的 workflow 建立；merge 送出後不能取消；D33 的 task 持有者要仍持有此 task、backend 仍允許、角色仍存在；程式裡 `holder` 改名 `task_holder`。verifier 情境寫成 `verifier_*` 回歸測試。
- 2026-09-25 使用者決定 Q1（記為 D33）：一個 agent 一個 task、返工回 task 持有者、額度用盡或被刪才交接；`policy::assign` 照此改寫並補測試（ba1fe59）。
- 2026-09-25 使用者確認 P1–P7（記為 D26–D32）；Q1（返工 fallback）當時仍待決定。
- 2026-09-25 修正第 2 輪 review 仍未解的項目（2a6e29b、4f78b31、e867d52）：N1／N2 head 變更在 work／submit 不改關卡、N3 `ChangesRequested` 退回最近的 work、N4 `merge_gate::evaluate` 逐個關卡的 fact、N5 狀態機探索器、N6 reviewer 同 backend fallback、N7 引號佔位符存檔擋下、N8 `RunCommand` 帶展開後指令與 change id、N9 `Cancel` 與 `Cancelled`；round-1 返工目標一致、`InvalidCommand` 訊息、移除 `ClientProtocolError`。N10：草稿另改了第 4 施工關的 `crates/agend-holder/src/pty.rs`（`control_key_bytes` 回傳 `Option`，隨 `ControlKey` 擴充）。review probe 情境都寫成 `review_*` 回歸測試。
- 2026-09-25 草稿原樣匯入 `feat/gate-01-core`（80c4de9），接手修正 review 第 2 輪仍未解的項目；草稿作者 2026-09-24 未經確認就打的 P1–P7 勾選先還原（b480311）。
- 2026-09-25 （草稿作者）修正兩份 review 的 pipeline、assignment、protocol 與 check-deps findings；workspace tests 通過（66 core tests、5 protocol compatibility tests）、workspace clippy、check-deps、accept core 通過；人工注入 std 的 check-deps 失敗路徑亦通過（工作樹，尚未提交；待 fresh-context verifier 與使用者親自驗收）。
- 2026-09-24 初次自動驗收通過；待 fresh-context verifier 與使用者親自驗收（工作樹，尚未提交）。
- 2026-09-24 草稿作者自行對照 P1–P7（不是使用者確認）：P6 保留期限由第 5 施工關 Store 落地；holder 升 major 時須保留前一 major。修正 Claude MCP fixture pattern；49 tests、fmt、clippy、check-deps 與 demo 通過（工作樹，尚未提交）。
- 2026-09-24 在 P1–P7 確認之前開始實作草稿（codex/gate-01-core 工作樹，尚未提交）；草稿作者自行勾選 P1–P7，並非使用者確認。
- 2026-09-24 狀態改為提案中；提案 P1–P7 待確認（#102）。

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-core
~/.cargo/bin/cargo xtask accept core
```
