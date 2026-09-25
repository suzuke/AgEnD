# 第 7 施工關：daemon：codex driver + 送達（`codex`）

> **TL;DR**
> - codex driver、送達模型、三級忙碌策略。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：等前一個施工關完成後開工；開工時把標「開工時細化」的步驟寫定。

## 狀態

**未開始**（2026-09-24）

## 範圍

- codex driver（app-server，JSON-RPC over WebSocket over unix socket）
- 送達模型：id、`queued → sent → confirmed | failed`、單一冪等
- 三級忙碌策略：queue、steer、interrupt

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-daemon` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept codex` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 真 CLI 一致性檢查（必要；使用者已決定 2026-09-25）：`codex --version` 和 `crates/agend-testkit/transcripts/codex/` 錄製檔 header 的 `version` 相同，不同就先用錄製器重錄（[RECORDER.md](../../crates/agend-testkit/RECORDER.md#重錄cli-升版時)）；`~/.cargo/bin/cargo test -p agend-testkit --test conformance` 通過
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 對假 app-server 跑 demo。

   ```bash
   ~/.cargo/bin/cargo xtask accept codex
   ```

   應該看到：三則訊息各用一種忙碌等級，每則都印出 `queued → sent → confirmed`。

   - [ ] 通過

2. 故意弄壞：同一個訊息 id 送兩次。

   操作：同一次輸出，找 `idempotent`

   應該看到：第二次不會再送出，狀態維持第一次的結果。

   - [ ] 通過

3. 真 CLI 一致性檢查（必做；使用者已決定 2026-09-25，取代原本選做的 smoke test）。

   ```bash
   codex --version
   head -1 crates/agend-testkit/transcripts/codex/one_turn.jsonl
   ~/.cargo/bin/cargo test -p agend-testkit --test conformance
   ```

   應該看到：第一行的版本和錄製檔 header 的 `"version"` 相同；最後 `test result: ok. 4 passed`。版本不同：先重錄再跑一次（`~/.cargo/bin/cargo xtask record codex --sandbox ~/Documents/Hack/AgEnD-ops/record-sandbox.sh`，會跑真的 codex、花少量 token，細節見 [RECORDER.md](../../crates/agend-testkit/RECORDER.md)）；檢查不過就改假 codex，不改錄製檔。

   **這步在驗什麼**：driver 測試用的假 app-server 和你機器上真的 codex 講同樣形狀的協定。壞了的話，driver 對假的全綠、接上真的才出錯（v1 #1483）。

   - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-25 使用者決定：真 CLI 一致性檢查（錄製器 + `tests/conformance.rs`）列為必要完成條件，取代選做的 smoke test（`feat/backend-recorder`）。

## 下一步

```bash
cat docs/gates/gate-07-codex.md
~/.cargo/bin/cargo xtask accept codex
```
