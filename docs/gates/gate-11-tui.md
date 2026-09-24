# 第 11 關：agend-tui（`tui`）

> **TL;DR**
> - attention-first TUI。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這關才算完成。
> - 下一步：等前一關完成後開工；開工時把標「開工時細化」的步驟寫定。

## 狀態

**未開始**（2026-09-24）

## 範圍

- 首頁（「需要你」+ 各 team 區塊）、team 頁、Task Detail、Agent Detail
- attach 單一 agent 的終端
- 英文／繁中切換

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-tui` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept tui` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 對有假 agent 的 daemon 開 TUI。

   ```bash
   agend app
   ```

   應該看到：首頁最上面是「需要你」，下面每個 team 一個區塊。

   - [ ] 通過

2. 處理一個決策。

   操作：在「需要你」選一項 → 選一個動作

   應該看到：該項從「需要你」消失；只是看過不會消失（已讀與已解決分開）。

   - [ ] 通過

3. 看 agent 輸出。

   操作：選一個 agent → 按 `t`

   應該看到：顯示該 agent 的終端畫面。

   - [ ] 通過

4. 導覽。

   操作：按 `←`／`→`、`/`、`L`

   應該看到：`←`／`→` 回上一層／進下一層；`/` 可搜尋；`L` 切換英文與繁中。

   - [ ] 通過

5. 故意弄壞：TUI 開著時停掉 daemon。

   操作：另一個終端停 daemon

   應該看到：TUI 顯示斷線狀態而不是當掉；daemon 回來後自動恢復。（開工時細化）

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
cat docs/gates/gate-11-tui.md
~/.cargo/bin/cargo xtask accept tui
```
