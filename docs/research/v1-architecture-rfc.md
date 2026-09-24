> 原始證據（歸檔）：v1 agend-terminal 在 2026-09 撰寫、未合併的架構簡化 RFC（原檔 `docs/ARCHITECTURE-v2.md`，未追蹤）。保留原文；它的 §0 更正記錄與 §4、§8 教訓是 v2 的方法論依據。平常不用讀。

# RFC — agend-terminal 架構簡化假說

**狀態**：**RFC／架構假說。未核准。不得據此開 issue。**
**盤點時點**：`49e303e0`（該時點的 HEAD）。**更正時點**：`bdb17869`。
**版本對照**：v0.11.3 = `b8f48965`。

> ## ⚠️ 本檔定位已撤銷重來
>
> 本檔的第一版自稱「定案的決策 + 待執行 issue 清單」，並列出 18 個可執行 issue。
> **該定位是錯的，已撤銷。** 撤銷理由有二：
>
> 1. **多個關鍵前提經查證為過期、錯誤或虛構**（§0 逐條列出，含證據）。
>    其中一項若照原文執行，會刪掉 codex 的狀態偵測。
> 2. **方法上是解法先行**：把「狀態應更一致」「持久化機制應減少」這類方向，
>    直接翻譯成 SQLite / event sourcing / single writer / 三程序 / 恰好 8 支 policy，
>    並以 LOC、鎖數、env var 數作為驗收標準。這些是**可能的選項**，
>    不是由已重現問題推導出的答案。
>
> 本檔現在的用途：保存仍然成立的診斷、保存被推翻的前提（供後人不要重踩）、
> 保存待驗證的假說。**任何提案要成為 issue，必須先通過 §4 的七道關卡。**

---

## §0 更正記錄 — 被推翻的前提

每條均經 `bdb17869` 實測。

### E1 — 基準版本標示錯誤

| 項目 | 第一版寫的 | 實況 |
|---|---|---|
| 盤點基準 | `main@49e303e0`（標示為 v0.11.3） | 49e303e0 只是盤點當時的 HEAD |
| v0.11.3 實際 commit | — | **`b8f48965`** |
| 目前 HEAD | — | `bdb17869`（盤點後又前進約 32 個 commit） |

**影響**：全文所有數據只能視為「49e303e0 時點快照」，不得當成當前事實引用。
倉庫在高速變動（近 60 天 1,265 個 commit），任何盤點數據的保鮮期以天計。

### E2 — D4 的主要理由已被修復

第一版以「修改 `fleet.yaml` 會清除使用者註解」作為引入 SQLite、
把 `fleet.yaml` 改唯讀、合併 teams 與 deployments 的主要理由。

**該缺陷已由 `fee2430e`（#3112，2026-07-27 03:11）修復**：
`src/fleet/persist.rs` +57 行、`src/teams.rs` +41 行，
新增 `preserve_yaml_comments(original, &serialized)`。

**精確描述**：修法是把註解收集後**當成 document header 重新輸出**
（原文：`comments are emitted as a document header`），
所以註解會集中到檔頭、不保留原位置 —— 症狀大幅緩解，但非完全無損。

**影響**：即便殘留「註解位置不保留」的次級問題，
其規模（既有持久化路徑上的小修）與跨模組資料庫遷移完全不成比例。
**D4 的主要正當性已消失。**

### E3 — agy 的 plane 盤點錯誤

第一版寫「agy 無 plane，待補 Transcript observer（甜點，先做）」，並據此開了 A0、A1 兩個 issue。

**實況**：agy 自 Phase D（#2448）起**已有 Hook plane** —— 最高等級的權威來源。

```
src/backend.rs:79-81   pub fn has_state_hooks(&self) -> bool {
                           matches!(self, Backend::ClaudeCode | Backend::Agy)
                       }
src/daemon/shadow/gate.rs:26-28   "(agy is NOT such a backend since Phase D, #2448 …
                                   has_state_hooks() in backend.rs covers claude + agy.)"
src/daemon/shadow/gate.rs:189     "Not agy — agy has a Hook plane since Phase D"
```

**影響**：A0／A1 **作廢**。為已有 `Hook` 的 backend 再加 `Transcript` observer，
既重複又降低權威等級。

**附帶發現（唯一通過關卡的行動項，見 §7）**：
`src/daemon/shadow/mod.rs:66` 的註解仍寫著
`(claude/codex/opencode/kiro hook-or-stream, agy screen)` ——
與 `gate.rs` 及 `has_state_hooks()` 直接矛盾。這行過期註解**實際誤導了本文第一版**。

### E4 — codex 的 plane 盤點錯誤（本文最危險的錯誤）

第一版寫 codex「僅出現在 rollout 過濾器，無專屬 plane」，
並在 A4 的動作清單寫「刪除 `shadow/rollout.rs` 的 gating 框架」。

**實況**：`src/daemon/shadow/rollout.rs` **就是 codex 的 observer**：

```
//! #2413 Phase D — codex rollout-tail observer source.
//! Codex (TUI mode) live-flushes its session to
//! `~/.codex/sessions/<Y>/<M>/<D>/rollout-<ts>-<uuid>.jsonl` DURING a turn …
//! strictly READ-ONLY tail … → Evidence (authority=Stream) …
//! claude = unix-socket hook ingest (Authority::Hook), codex = rollout tail (Authority::Stream)
```

**成因**：從檔名 `rollout.rs` 推語義為「功能推出（feature rollout）框架」，
未讀檔頭。同一輪讀了 `kiro.rs`／`opencode.rs`／`mod.rs` 的檔頭卻獨獨跳過此檔 —— 抽樣不完整。

**影響**：**照 A4 原文執行會刪除 codex 的狀態偵測。**
這是本文第一版最危險的單一錯誤，也是「不得據此開 issue」的最強理由。

### E5 — 引用了不存在的 API

第一版的 A2 寫「`Backend::evidence_sources()` 對 `Backend::Shell` 宣告 …，零新程式碼」。

**實況**：`evidence_sources` 在 `src/` 內 **0 個 hit，該函式不存在**。
語意最接近的既有 API 是 `Backend::has_state_hooks()`（`backend.rs:79`）。

**影響**：A2 的「近乎零成本」估計無依據 —— 它其實需要先設計一個新的宣告 API。
這是 `~/.claude/lessons.md` 第一條記載的錯誤模式（憑印象把名稱寫進規則），本文再犯一次。

### E6 — 鎖數誇大 3.76 倍

第一版以「353 個帶鎖 static」支持風險最高的 single-writer 重構（B3）。

**重新分類**（`bdb17869` 實測）：

| 類別 | 數量 | 是否參與鎖序 |
|---|---|---|
| `static` 宣告總數 | 422 | — |
| `Mutex` | 89 | ✅ |
| `RwLock` | 5 | ✅ |
| `OnceLock` | 89 | ❌ 初始化一次，非互斥鎖 |
| `Atomic*` | 206 | ❌ lock-free |
| **真正涉及鎖序** | **94** | |

**影響**：353 這個數字把 `OnceLock` 與 lock-free atomic 都算進去，誇大 3.76 倍。
94 個 Mutex/RwLock 仍值得關注，但不足以支撐一個 XL 級、被自評為「風險最高」的重構。

### E7 — C1 的驗收條件自相矛盾

第一版 C1 同時要求「release archive 新增獨立 `agend-tray` binary」
與「release archive 內容與 v0.11.3 逐檔比對一致」。兩者不可能同時成立。

---

## §1 仍然成立的診斷

以下部分未被上述更正推翻，且經審閱同意。

### 1.1 症狀（非解法）

1. **狀態來源不只一處**：刮螢幕、hook/stream plane、程序訊號並存，
   由 `shadow/reducer.rs` 折算。折算規則本身是一個需要維護的機制。
2. **持久化格式數量持續增加**：`fleet.yaml`、多種 JSONL、多種 JSON、
   marker 檔、lock 檔並存，每種各自實作原子寫、損毀處理、遷移與 GC。
3. **週期性工作彼此不知道對方在做什麼**：37 個 `per_tick` 模組 + 12 個 bus subscriber，
   對同一個 instance 可能同時下不同結論。
4. **保護機制以測試與文件形式存在**：鎖序靠一份人工維護的文件 + 一個 audit test；
   `block_on` 禁令靠 AST 掃描測試；subscriber 註冊完整性靠掃自己原始碼的字串比對。
   這些是「用紀律補結構」——它們有效，但每一個都是持續的維護稅。

### 1.2 三個有據可查的具體事件

| 事件 | 性質 | 狀態 |
|---|---|---|
| #1720 app 模式漏註冊 subscriber → 看門狗靜默失效 | 真實缺陷 | 已修（用原始碼掃描測試防守） |
| #1474／#1476 `block_on` 巢狀 panic（同類犯兩次） | 真實缺陷 | 已修（用 HARD RULE + AST 測試防守） |
| #919 誤判：使用者輸入含 `Error:` 被判為 agent 失敗 | 真實缺陷 | 已緩解（紅色 SGR 錨點） |

**注意**：三者**都已修復**。它們證明「這類錯誤會發生」，
不證明「必須換架構才能避免」。要主張後者，需要證明修復後的防守成本
高於重構成本 —— 本文尚未證明。

### 1.3 方向（各方同意，但不指定解法）

- 減少機制數量
- 優先使用權威證據
- 降低重複狀態來源
- 讓持久化與生命週期更容易推理
- 以 KISS 約束後續開發

---

## §2 修正後的數據

盤點於 `49e303e0`，**視為快照，非當前事實**。

| 指標 | 數值 | 附註 |
|---|---|---|
| `src/` 總行數 | 368,043 | |
| 生產碼 | ≈123,400 | 扣除 test 檔與 `#[cfg(test)]` 尾段 |
| 測試碼 | ≈244,600（另 `tests/` 23,920） | 測試:生產 ≈ 2.2 : 1 |
| 根層模組 | 84 個 `.rs` / 60+ 目錄 | |
| MCP tool / CLI 子命令 | 32 / 22 | |
| `per_tick` 模組 + bus subscriber | 37 + 12 | |
| `static` 宣告 | 422（Mutex 89、RwLock 5、OnceLock 89、Atomic 206） | **見 E6** |
| `AGEND_*` env var | 96 個名稱，67 個讀取點 | |
| `thread::spawn` / runtime 建構 | 150 / 76 | |
| 檔案 I/O 呼叫點 | 2,292 處，206 個生產檔 | |
| 子程序 | 381 處（`git` 311） | |
| invariant/guard/audit 測試 | 141 個測試檔中 41 個 | |

**這些是觀察指標，不是驗收條件。** 行數下降不等於機制減少 ——
把功能移到新 crate、新 binary 或 migration layer，可能讓行數下降而部署與除錯成本上升。

---

## §3 五項提案的現況

第一版把這五項標為「定案」。這個標籤有一半正確、一半誤導，需要分清：

- **選擇本身確實由審閱者在討論中做出**（五項各自從選項中挑選）。
- **但其中兩項的選擇建立在後來證實錯誤的前提上**，因此選擇應視為失效。
- **A/B/C 三線與 18 個 issue 從未經任何核准**，是本文第一版的單方面擴充。

| # | 提案 | 前提狀態 | 現況 |
|---|---|---|---|
| P1 | tray 抽成獨立 crate | 前提未被推翻，但「crate extraction = 簡化」的假設未經證明；C1 驗收自相矛盾（E7） | **退回，待過關卡** |
| P2 | claim_verifier 拆兩半 | 前提未被推翻；但未證明現況造成實際維護衝突 | **退回，待過關卡** |
| P3 | quickstart 移出 daemon crate | 同上 | **退回，待過關卡** |
| P4 | teams+deployments 合併、fleet.yaml 唯讀 | **主要理由已被 #3112 修復（E2）** | **失效，需重新舉證** |
| P5 | shadow 補 plane 後畢業 | **backend 盤點錯誤（E3、E4）；依賴不存在的 API（E5）** | **失效，需重新盤點** |

### P5 的盤點更正

| backend | plane | authority | 第一版判斷 |
|---|---|---|---|
| claude | unix-socket hook ingest（`shadow/mod.rs`） | `Hook` | ✅ 正確 |
| **agy** | lifecycle hooks（`has_state_hooks()`） | **`Hook`** | ❌ 誤判為「無 plane」 |
| **codex** | rollout-tail（`shadow/rollout.rs`） | **`Stream`** | ❌ 誤判為「無專屬 plane」 |
| kiro | session-tail（`shadow/kiro.rs`） | `Stream` | ✅ 正確 |
| opencode | `/event` SSE（`shadow/opencode.rs`） | `Stream` | ✅ 正確 |
| grok | 無 | `Screen` only | ✅ 正確 |
| shell | 無（程序訊號） | `Screen` only | ✅ 正確 |

**修正後的實況：7 個 backend 有 5 個已有非刮螢幕的 plane**，
只有 grok 與 shell 沒有。第一版說「只有 3 個有、4 個要補」是錯的。

---

## §4 提案晉級條件（七道關卡）

**任何提案要從本 RFC 變成 issue，必須逐條通過並附證據。**

1. **現在是否存在可重現的使用者問題？**（要有重現步驟，不是推論）
2. **問題是否尚未被修正、也未被既有 issue／任務涵蓋？**（先查 git log 與 issue tracker）
3. **最小修法是什麼？**（明確寫出，並估規模）
4. **新機制能刪除多少既有機制？**（要能列舉；刪除量必須大於新增量）
5. **若不進行完整架構遷移，是否仍可獨立改善問題？**（若可，就先做那個）
6. **驗收是否以使用者可觀察行為為準？**（不得用 LOC、檔案數、crate 數、鎖數、env var 數）
7. **是否有真實情境測試與可行的回退方式？**

**明確禁止的論證形式**：

- ❌ 先建立遷移安全網（如 golden e2e），再反過來用安全網的存在正當化尚未核准的遷移
- ❌ 以「架構完整性」或「路線圖看起來完整」為由，為已修復或未證實的問題製造工作
- ❌ 把模組邊界調整（拆 crate、拆 binary）本身當作簡化的證明

---

## §5 逐項套用關卡的裁決

第一版的 18 個 issue，逐一套用 §4：

| Issue | 關卡結果 | 裁決 |
|---|---|---|
| C1 tray 抽 crate | ①無實測建置痛點證據 ⑥驗收用 crate 數 ⑦驗收自相矛盾（E7） | **退回 RFC** |
| C2 claim_verifier 拆半 | ①無可重現問題 ④未證明刪除量 > 新增量 | **退回 RFC** |
| C3 quickstart 移出 | ①無可重現問題 | **退回 RFC** |
| A0／A1 agy observer | ①前提錯誤（agy 已有 Hook，E3） | **作廢** |
| A2 shell 宣告 | ⑤依賴不存在的 API（E5），需重新設計 | **退回 RFC** |
| A3 grok observer | ①grok 確為 Screen-only，但無可重現的誤判案例佐證 | **退回 RFC，待證據** |
| A4 shadow 畢業 | ①③④ 均未成立；**動作清單含刪除 codex observer 的致命錯誤（E4）** | **作廢，需整項重寫** |
| B0 golden e2e | 違反 §4 禁止形式第一條（先建安全網再正當化遷移） | **退回 RFC** |
| B1／B1b／B1c SQLite 遷移 | ①主要理由已修復（E2）④遷移新增 schema/migration/dual-write/回退/版本相容，未證明刪除量更大 | **退回 RFC** |
| B2 core model | ①無可重現問題，屬純設計偏好 | **退回 RFC** |
| B3 單寫者 | ①數據誇大 3.76 倍（E6）⑦自評風險最高且無回退設計 | **退回 RFC** |
| B4 37→8 policy | 「8」無行為模型依據；減少檔案數 ≠ 減少決策機制 | **退回 RFC** |
| B5 TUI 拆程序 | ①#1720 已修復；未證明現行防守成本過高 | **退回 RFC** |
| B6 env var 收斂 | ①無可重現問題 ⑥驗收用 env var 數 | **退回 RFC** |
| B7 刪護欄測試 | 相依於未核准的 B4/B5/B6 | **退回 RFC** |

**結論：18 個 issue，0 個原樣通過。2 個作廢（前提錯誤），16 個退回本 RFC 待舉證。**

---

## §6 架構假說（保存，不核准）

以下是**可能值得探索的方向**，每一項都需要先有可重現問題並通過 §4 才能推進。
保存的理由是避免重複發想，不是背書。

- **H1**：單一持久化機制是否能刪除的複雜度多於它新增的（schema、migration、
  dual-write、回退、版本相容、資料修復）？**未證明。**
- **H2**：週期性決策若收斂到單一 reconcile 點，是否真能消除衝突結論？
  需要先蒐集「多看門狗對同一 instance 下衝突結論」的實際案例。**目前無案例佐證。**
- **H3**：破壞性動作（kill／respawn／reclaim）要求權威證據，是否能降低誤判？
  方向合理，但 `shadow/gate.rs` 的 `gated_override` **已經在做類似的事**，
  需先量測現況殘留誤判率再談。
- **H4**：狀態所有權集中（單寫者）是否值得 94 個 Mutex/RwLock 的重構風險？**未證明。**
- **H5**：daemon 與 TUI 分程序是否值得？#1720 已修復，需要新的證據。

**H1–H5 均為開放問題，不是計畫。**

---

## §7 唯一通過關卡的行動項

跑完 §4 的七道關卡後，本 RFC 只能誠實提出一個行動項 ——
而它正是本文第一版犯錯的來源之一。

### X1 — 修正 `shadow/mod.rs:66` 的過期註解

- **問題**：該行寫 `(claude/codex/opencode/kiro hook-or-stream, agy screen)`，
  與 `gate.rs:26-28,189` 及 `backend.rs:79-81` 的 `has_state_hooks()` 直接矛盾。
  agy 自 Phase D（#2448）起已有 Hook plane。
- **關卡①**：可重現 —— 讀該行即得到錯誤結論。**本文第一版就是這樣被誤導的**
  （受眾是維護者而非終端使用者，此點如實標註，不誇大）。
- **關卡②**：未修復（`bdb17869` 仍在）。
- **關卡③**：最小修法 —— 改一行註解。
- **關卡④**：不新增機制。
- **關卡⑤**：無需任何架構遷移。
- **關卡⑥**：驗收 = 註解與 `has_state_hooks()` 的實際涵蓋一致。
- **關卡⑦**：零回退風險。
- **規模**：XS。

**這是本 RFC 唯一主張現在就該做的事。** 其餘一律留在假說階段。

---

## §8 方法論教訓

記錄本文第一版的錯誤模式，供後續文件與 agent 參考。

| 錯誤模式 | 本文的實例 | 防範 |
|---|---|---|
| 從檔名推語義 | 把 `shadow/rollout.rs` 讀成「feature rollout 框架」（E4） | 引用任何檔案前先讀檔頭；同族檔案要全讀，不抽樣 |
| 憑印象寫 API 名 | `Backend::evidence_sources()` 不存在（E5） | 每個 API 名在提交前 grep 驗證存在 |
| grep 分類過寬 | 把 `OnceLock`／`Atomic` 算成「帶鎖」（E6） | 分類統計必須逐類分開計數並列出 |
| 盤點數據當永久事實 | 標錯基準版本、未察覺已修復的缺陷（E1、E2） | 引用缺陷前先 `git log --grep` 確認未修 |
| 可度量但無因果的驗收標準 | LOC、鎖數、policy 數、env var 數 | 驗收一律用使用者可觀察行為 |
| 方向直接翻譯成解法 | 「減少機制」→ SQLite + event sourcing + 單寫者 + 三程序 | 方向與解法分開陳述；解法需通過 §4 |
| 用安全網正當化遷移 | B0 golden e2e 作為 B1–B7 的前提 | 安全網只能在遷移獲核准後建立 |

---

## §9 本 RFC 的下一步

1. **不執行任何 A0–B7。** 該清單已作廢。
2. X1（§7）可獨立提出。
3. 若要推進 H1–H5 任一項，起點是**蒐集可重現的問題案例**，不是設計方案。
4. 本檔任何數據在引用前需以當前 HEAD 重新驗證 —— 倉庫變動速度以天計。
