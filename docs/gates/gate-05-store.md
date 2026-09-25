# 第 5 施工關：agend-daemon：store（`store`）

> **TL;DR**
> - SQLite schema、migration、保留期限、每日快照。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：等前一個施工關完成後開工；開工時把標「開工時細化」的步驟寫定。

## 狀態

**未開始**（2026-09-24）

## 範圍

- schema 與 migration
- 每張表的保留期限（依第 1 施工關 P6）
- 每日 `VACUUM INTO` 快照與份數

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-daemon` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept store` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] 重啟／持久化類的契約 case（STO-4、STO-12，見 [CONTRACTS.md](../../crates/agend-testkit/CONTRACTS.md)）對真實作、跨真的 process 重啟跑（分開的 process、真的檔案／DB）**（已追認 2026-09-25，第 2 施工關 A25）**
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 跑 demo（用暫存 DB 與假時鐘）。

   ```bash
   ~/.cargo/bin/cargo xtask accept store
   ```

   應該看到：建立暫存 DB → 跑 migration → 放入樣本資料 → 印出每張表的筆數。

   - [ ] 通過

2. 保留期限。

   操作：同一次輸出，找 `retention`

   應該看到：假時鐘往前撥之後，超過期限的訊息／事件消失，task 紀錄還在；前後兩張表的筆數對得上 P6 的規則。

   - [ ] 通過

3. 快照。

   操作：同一次輸出，找 `snapshot`

   應該看到：印出快照檔路徑與大小，檔案存在。

   - [ ] 通過

4. 故意弄壞：對已有新 schema 的 DB 跑舊版 migration。

   操作：開工時細化

   應該看到：被拒絕並說明版本不符，DB 內容不變。

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
cat docs/gates/gate-05-store.md
~/.cargo/bin/cargo xtask accept store
```
