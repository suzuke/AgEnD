# 第 1 關：agend-core（`core`）

> **TL;DR**
> - 純邏輯 crate：型別、兩套協定、trait、流水線狀態機、busy policy、去抖動、衝突偵測、merge 門檻、螢幕分類器。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這關才算完成。
> - 下一步：使用者逐條確認 P1–P7 與「待你決定」；之後才進 fresh-context verifier 與親自驗收。

## 狀態

**提案中（實作草稿完成，待使用者確認 P1–P7）**（2026-09-25）

實作草稿是在 P1–P7 確認之前寫的；若你否決或修改任何一項，草稿依你的決定調整。

## 範圍

- 型別、client 與 holder 兩套協定、trait（`Driver`、`Forge`、`Store`、`Runtime`、`Runner`、`Notifier`、`Clock`，依 P2／P3）
- 流水線狀態機（6 種關卡、task 關係與操作、workflow 存檔檢查）
- policy：busy、去抖動、衝突偵測、merge 門檻（patch-id）、分派（D25）
- 螢幕分類器與規則資料
- `cargo xtask accept core` 的 demo

## 開工前提案

每一項都要你逐條確認（勾選）。目前 7 項都**尚未確認**；草稿先照「建議」實作，確認前不算定案。

### P1：wire format 與版本協商

- 問題：client protocol 與 holder protocol 用什麼格式、怎麼處理版本？
- 建議：兩套協定都用 unix socket 上的 JSON Lines；PTY 位元組以 base64 放在 JSON 裡。連線第一則訊息是 `hello`，交換協定版本；major 不同 → 印出清楚的錯誤；同一個 major 內只准新增欄位，未知欄位忽略。holder 協定最保守：新 daemon 必須能跟舊一個 major 的 holder 溝通。
- 理由：好除錯（`nc`／`jq` 就看得懂）；holder 很少更新，daemon 升級時舊 holder 還在跑。
- 替代方案：長度前綴的二進位 frame（較快，但難除錯）。
- [ ] 使用者確認

### P2：trait 簽章

- 問題：`Driver`、`Forge`、`Store`、`Runtime`、`Notifier`、`Clock` 的方法怎麼定？
- 建議：「functional core, imperative shell」：流水線狀態機是純函式 `step(state, event) -> (state, Vec<Action>)`，不呼叫任何 trait。trait 仍放 core（D11），用 `async fn`、不含 tokio 型別（相容 no_std）。每個 trait 只放實際用到的最少方法，例如 Forge：`submit`、`head`、`merge_if_head_is`；Store：每個用途一個方法、寫入帶 CAS 版本，不做通用查詢語言。
- 理由：core 的測試不需要任何假實作；trait 保持小，假實作與契約測試才容易寫。
- 替代方案：狀態機直接呼叫 trait（每個 core 測試都要假實作）。
- [ ] 使用者確認

### P3：runner 要不要 trait

- 問題：`command` 關卡的 runner 需要自己的 trait 嗎？
- 建議：要：`Runner::run(cmd, dir, timeout) -> Output`。
- 理由：D9 要求每個外部邊界都有 trait + 假實作；`command` 關卡與 git adapter 都要跑程序。
- 替代方案：不設 trait、直接用 `tokio::process`（交接測試的 agent 提出）——否決：不跑真指令就無法測試。
- [ ] 使用者確認

### P4：GitHub CI 怎麼進流水線

- 問題：Checks 已是 `command` 關卡，GitHub 的 CI 結果怎麼接？
- 建議：用 `command` 關卡，例如 `run = "gh pr checks {pr} --watch --fail-fast"`；`command` 新增佔位符 `{pr}`、`{head}`、`{branch}`。forge 維持 3 個方法，local forge 不需要空實作。
- 理由：一種機制（command）涵蓋本機與 GitHub；forge 保持小。
- 替代方案：由 forge github 回報 CI 狀態（forge 變大，local forge 要有空方法）。
- [ ] 使用者確認

### P5：去抖動

- 問題：idle↔busy 要穩定多久才生效？
- 建議：不對稱：轉 busy 立即生效（絕不送進忙碌中的 agent）；轉 idle 要穩定 5 秒。先用常數，第 7 關用真實資料重新校準。
- 理由：v1 約兩天 75 萬次轉換；送錯時機的代價在「送進忙碌 agent」那一側。
- 替代方案：對稱的 N 秒（兩邊都延遲）。
- [ ] 使用者確認

### P6：保留期限

- 問題：各類資料留多久？
- 建議：task／workflow／decision 紀錄永久保留（量小）；訊息 30 天；事件與狀態轉換 14 天；封存的 WIP patch 30 天；每日 DB 快照留 7 份。第 5 關重新校準。
- 理由：先給保守可用的預設值，實作 store 時用實際大小修正。
- 替代方案：全部永久保留（v1 home 長到 161G 的教訓）；或全部同一個期限。
- [ ] 使用者確認

### P7：core 依賴 allowlist

- 問題：protocol 型別要能 derive 序列化，core 可以加依賴嗎？
- 建議：允許 `serde`（+ `serde_derive`），`default-features = false`，只開 `derive` + `alloc`，讓 protocol 型別在 no_std 下 derive；JSON 編碼本身放在 daemon／client。這會改動原本為空的 `CORE_DEP_ALLOWLIST`，所以需要明確核准。
- 理由：兩邊共用同一份型別定義（D11），又不讓 core 碰 I/O。
- 替代方案：在 daemon／client 各自手寫轉換（重複且易漂移）；或 core 保持零依賴、協定型別不 derive。
- 草稿的實際範圍（比建議多一項，請一併確認）：serde derive 用在 protocol 型別，以及 workflow 定義型別（`Workflow`、`WorkflowStage`、`Stage` 與其欄位型別、`FanoutJoin`）。理由：D19 的 workflow 以 TOML 存進 DB、匯出／存回，存檔檢查（`Workflow::validate`）在 core，adapter 需要把 TOML 解成同一份型別，不另寫一份轉換。`Task`、`PipelineState`、`Candidate` 等執行期型別**不** derive：它們的持久化格式屬第 5 關 store，屆時再決定。
- 草稿已照此改了 `crates/agend-core/Cargo.toml`、`xtask/src/check_core.rs` 的 `CORE_DEP_ALLOWLIST` 與 AGENTS.md／ARCHITECTURE.md 的說明，並在這些地方註明「待 P7 確認」。否決 P7 時：移除 serde 依賴、allowlist 改回空，protocol 型別的 JSON 轉換改放 adapter。
- [ ] 使用者確認


## 待你決定

草稿作者先選了行為、但沒有經你決定的事項。確認前照「草稿目前的行為」運作。

### Q1：返工時原作者不能接（round-1 review I8）

- 問題：D18 同時說「退回修改回原作者」與「額度用盡改派其他允許的 backend」；原作者不能接時兩條衝突。
- 草稿目前的行為（`policy::assign::choose`，`Purpose::Rework`）：

  | 情況 | 結果 |
  |---|---|
  | 原作者在 team、有額度、有空位 | 派回原作者 |
  | 原作者額度用盡 | 改派同角色、不同 backend、有空位的 instance；沒有就在另一個有額度的 backend 開臨時 instance（受角色人數上限）；都不行就排隊（`UsageLimit`） |
  | 原作者有額度但沒空位 | 排隊等原作者（`AtCapacity`） |
  | 原作者已不在 team | 排隊（`ReworkAuthorUnavailable`），不改派 |
  | 角色範本已刪除 | 不轉成 ask，照上面規則 |

- 要你決定：額度用盡時改派他人（目前）還是等原作者？原作者不在時要不要改派同角色的其他人？
- [ ] 使用者決定

## 自動驗收（完成定義）

- [x] `cargo test --workspace` 通過；其中 `agend-core` 66 tests、xtask protocol compatibility 5 tests（2026-09-25）
- [x] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨（2026-09-25）
- [x] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`；注入 `std::fs` 時 checker exit 1，還原後通過（2026-09-25）
- [x] `~/.cargo/bin/cargo xtask accept core` 通過，並印出下方 demo（2026-09-25）
- [x] 本關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 跑 acceptance。它會先執行格式、workspace clippy、core tests、protocol compatibility tests、check-deps，再跑實際 core demo。

   ```bash
   ~/.cargo/bin/cargo xtask accept core
   ```

   應該看到：demo 先列出實際 protocol hello、busy 與 debounce 結果，再以 `assign::choose` 派出作者及不同 backend 的 reviewer，並以 `pipeline::state::step` 完成一次 command 失敗返工、head 更新、同 patch rebase 重跑 checks、diff 改變後返工及最終 merge。完整輸出見下方；最後一行是 `gate 1 (core): checks passed`。

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

### 預期 transcript

以下是 core example 的實際輸出（不含 accept 前面的檢查指令）；work、assignment、policy 與 pipeline action 均由 core 函式產生。

```text
client hello: supports 1.0
busy levels for steer:
  claude: Interrupt
  codex: Steer
  opencode: Interrupt
debounce: idle immediate=false, before 5s=false, at 5s=true; busy immediate=true, state=Busy
task T-1  workflow=code v1
  assignment -> dev-1
  reviewer   -> review-1
  start                     -> assign role dev (work)
  work                      -> submit via local
  submit                    -> run checks (cargo test) at H0
  command failed            -> return to work, assign role dev (work)
  work retry                -> submit via local
  submit                    -> run checks (cargo test) at H1
  command passed            -> request review approval (head-bound=true)
  approval H1               -> merge at H1
  new commit                -> run checks (cargo test) at H2
  command passed            -> request review approval (head-bound=true)
  approval H2               -> merge at H2
  clean rebase              -> run checks (cargo test) at H3
  checks rerun              -> merge at H3
  changed rebase            -> return to work, assign role dev (work)
  work after changed patch  -> submit via local
  submit                    -> run checks (cargo test) at H5
  command passed            -> request review approval (head-bound=true)
  approval H5               -> merge at H5
  merge completed           -> task done at Some("M1")
```


## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-25 草稿原樣匯入 `feat/gate-01-core`（f540247），接手修正 review 第 2 輪仍未解的項目；P1–P7 勾選還原為未確認。
- 2026-09-25 （草稿作者）修正兩份 review 的 pipeline、assignment、protocol 與 check-deps findings；workspace tests 通過（66 core tests、5 protocol compatibility tests）、workspace clippy、check-deps、accept core 通過；人工注入 std 的 check-deps 失敗路徑亦通過（工作樹，尚未提交；待 fresh-context verifier 與使用者親自驗收）。
- 2026-09-24 初次自動驗收通過；待 fresh-context verifier 與使用者親自驗收（工作樹，尚未提交）。
- 2026-09-24 草稿作者自行對照 P1–P7（不是使用者確認）：P6 保留期限由第 5 關 Store 落地；holder 升 major 時須保留前一 major。修正 Claude MCP fixture pattern；49 tests、fmt、clippy、check-deps 與 demo 通過（工作樹，尚未提交）。
- 2026-09-24 在 P1–P7 確認之前開始實作草稿（codex/gate-01-core 工作樹，尚未提交）；P1–P7 仍待使用者確認。
- 2026-09-24 狀態改為提案中；提案 P1–P7 待確認（#102）。

## 下一步

```bash
cargo test -p agend-core
cargo xtask accept core
```
