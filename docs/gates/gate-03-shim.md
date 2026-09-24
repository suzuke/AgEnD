# 第 3 關：agend-shim（`shim`）

> **TL;DR**
> - agent PATH 上的 git 與 kill 防護。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這關才算完成。
> - 下一步：等前一關完成後開工；開工時把標「開工時細化」的步驟寫定。

## 狀態

**未開始**（2026-09-24）

## 範圍

- git 呼叫分類：放行、導向綁定的 worktree、拒絕
- protected-ref 檢查
- 破壞性操作前快照與還原
- `kill`／`killall`／`pkill` 防護、audit 記錄

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-shim` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept shim` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 跑 demo（會建一個暫存 repo）。

   ```bash
   ~/.cargo/bin/cargo xtask accept shim
   ```

   應該看到：每一段標出指令與結果；下面幾步逐段檢查。

   - [ ] 通過

2. 導向：在綁定狀態下 commit。

   操作：同一次輸出，找 `route`

   應該看到：commit 出現在綁定的 worktree 的 branch 上，canonical repo 沒有變。

   - [ ] 通過

3. 故意弄壞：`git checkout main`。

   操作：同一次輸出，找 `refuse`

   應該看到：被拒絕，訊息附上正確的下一步命令（例如改用 `agend task create`），exit 非 0。

   - [ ] 通過

4. `git reset --hard` 快照與還原。

   操作：同一次輸出，找 `snapshot`

   應該看到：reset 前印出快照 id；還原後被 reset 掉的檔案回來了。

   - [ ] 通過

5. 故意弄壞：`pkill` 別人的程序。

   操作：同一次輸出，找 `kill guard`

   應該看到：被拒絕，目標程序還活著。

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
cat docs/gates/gate-03-shim.md
~/.cargo/bin/cargo xtask accept shim
```
