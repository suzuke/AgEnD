# 第 2 關：agend-testkit（`testkit`）

> **TL;DR**
> - 共用測試基礎設施：每個 trait 的假實作、契約測試、假 daemon、假 agent 程式。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這關才算完成。
> - 下一步：等前一關完成後開工；開工時把標「開工時細化」的步驟寫定。

## 狀態

**未開始**（2026-09-24）

## 範圍

- 每個 trait 的假實作
- 契約測試套件（同一套測試跑假實作與真實作）
- 假 daemon（protocol v1）
- 假 agent：假 codex app-server、假 opencode serve、帶 hook 的假 claude

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-testkit` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept testkit` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 跑 demo。

   ```bash
   ~/.cargo/bin/cargo xtask accept testkit
   ```

   應該看到：依序啟動三個假 agent，各印出一段範例往來（例如假 app-server 收到 `turn/start` 回 `turn/completed`），最後每個都正常結束。

   - [ ] 通過

2. 看契約測試摘要。

   操作：同一次輸出，找 `contract`

   應該看到：每個 trait 一行，假實作的契約測試全部 pass（例如 `Forge: fake 12/12`）。真實作那一側在各自的關卡才接上（例如 forge local 在第 10 關），此時只看假實作（開工時細化：確切格式）。

   - [ ] 通過

3. 故意弄壞：讓假實作偏離契約。

   操作：開工時細化：改動假 Forge 的一個回傳值

   應該看到：契約測試在假實作那一側失敗，指出是哪一條契約；還原後回到全綠。

   - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- （尚無）

## 下一步

```bash
cat docs/gates/gate-02-testkit.md
~/.cargo/bin/cargo xtask accept testkit
```
