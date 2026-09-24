# 第 10 關：daemon：流水線（`pipeline`）

> **TL;DR**
> - pipeline、git、runner、forge local、supervisor、reconcile。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這關才算完成。
> - 下一步：等前一關完成後開工；開工時把標「開工時細化」的步驟寫定。

## 狀態

**未開始**（2026-09-24）

## 範圍

- 驅動 core 狀態機
- git adapter（先記錄再建立）
- runner（`command` 關卡）
- forge local（merge-tree + CAS update-ref）
- supervisor、reconcile

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-daemon` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept pipeline` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 暫存 repo + 假 agent：建立 task。

   ```bash
   agend task create --role dev "demo"`（開工時細化）
   ```

   應該看到：`agend status` 依序顯示 work → submit(local) → command → approval → merge。

   - [ ] 通過

2. 確認 merge 結果。

   ```bash
   git -C <暫存 repo> log --oneline -1 main
   ```

   應該看到：main 最新的 commit 是這個 task 的 merge。

   - [ ] 通過

3. 確認清理。

   ```bash
   git -C <暫存 repo> branch --list "agend/*"; ls <home>/worktrees/
   ```

   應該看到：這個 task 的 branch 與 worktree 都不見了。

   - [ ] 通過

4. 故意弄壞：task 結束時 worktree 裡留著未 commit 的變更。

   操作：開工時細化

   應該看到：worktree 被刪除前，WIP 被存成 patch 放在 `archive/`，總覽裡看得到。

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
cat docs/gates/gate-10-pipeline.md
~/.cargo/bin/cargo xtask accept pipeline
```
