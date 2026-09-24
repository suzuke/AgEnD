# 第 1 關：agend-core（`core`）

> **TL;DR**
> - 純邏輯 crate：型別、兩套協定、trait、流水線狀態機、busy policy、去抖動、衝突偵測、merge 門檻、螢幕分類器。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這關才算完成。
> - 下一步：逐條確認「開工前提案」。

## 狀態

**提案中**（2026-09-24）

## 範圍

- 型別、client 與 holder 兩套協定、trait（`Driver`、`Forge`、`Store`、`Runtime`、`Runner`、`Notifier`、`Clock`，依 P2／P3）
- 流水線狀態機（6 種關卡、task 關係與操作、workflow 存檔檢查）
- policy：busy、去抖動、衝突偵測、merge 門檻（patch-id）、分派（D25）
- 螢幕分類器與規則資料
- `cargo xtask accept core` 的 demo

## 開工前提案

每一項都要你逐條確認（勾選）後才開始實作。

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
- 建議：允許 `serde`（+ `serde_derive`），`default-features = false`，只開 `derive` + `alloc`，讓 protocol 型別在 no_std 下 derive；JSON 編碼本身放在 daemon／client。這會改動目前為空的 `CORE_DEP_ALLOWLIST`，所以需要明確核准。
- 理由：兩邊共用同一份型別定義（D11），又不讓 core 碰 I/O。
- 替代方案：在 daemon／client 各自手寫轉換（重複且易漂移）；或 core 保持零依賴、協定型別不 derive。
- [ ] 使用者確認


## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-core` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept core` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 跑 demo，對照下方「預期 transcript」逐段比對。

   ```bash
   ~/.cargo/bin/cargo xtask accept core
   ```

   應該看到：依序出現：派工 → work → submit → command 失敗退回 work → 修正後 command 通過 → approval 綁 head `H1` → 新 commit `H2` 使核准失效 → 重新 approval 綁 `H2` → main 前進、patch-id 不變 → 核准保留 → main 再前進、patch-id 改變 → 退回 work；最後一行 `gate 1 (core): checks passed`。

   - [ ] 通過

2. 看 busy 等級表。

   操作：同一次輸出，往上找 `busy levels`

   應該看到：一張表：codex 的 steer 仍是 steer；claude、opencode 的 steer 變成 interrupt；三者的 queue、interrupt 都不變。

   - [ ] 通過

3. 看去抖動行為。

   操作：同一次輸出，往上找 `debounce`

   應該看到：轉 busy 的那一刻立刻生效；轉 idle 要等 5 秒穩定才生效（P5 的數字；開工時細化）。

   - [ ] 通過

4. 故意弄壞：讓 core 用到 std，確認被擋。

   ```bash
   echo 'pub fn leak() { let _ = std::fs::read("/etc/hosts"); }' >> crates/agend-core/src/lib.rs
   ~/.cargo/bin/cargo xtask check-deps; echo "exit=$?"
   git checkout crates/agend-core/src/lib.rs
   ```

   應該看到：`check-deps: agend-core does not compile for the no-std target …` 與 `exit=1`；還原後重跑 `check-deps` 回到 ok。

   - [ ] 通過

### 預期 transcript（開工時細化）

以下是綱要，實作時換成 demo 的逐字輸出，之後每次驗收都拿來比對。

```text
task T-1  workflow=code v1
  work      -> assigned to dev-1 (branch agend/T-1/demo)
  submit    -> forge local
  command   -> FAIL (exit 1) -> back to work
  work      -> new commit H1
  command   -> ok
  approval  -> approved by reviewer-1, bound to head H1
  work      -> new commit H2: approval invalidated
  approval  -> approved, bound to head H2
  main advances, rebase clean, patch-id unchanged -> approval kept
  main advances, rebase clean, patch-id changed   -> back to work
```


## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-24 狀態改為提案中；提案 P1–P7 待確認（PR：docs/gate-pages）。

## 下一步

```bash
cat docs/gates/gate-01-core.md
~/.cargo/bin/cargo xtask accept core
```
