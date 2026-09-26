# 第 10 施工關：daemon：流水線（`pipeline`）

> **TL;DR**
> - pipeline、git、runner、forge local、supervisor、reconcile。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：等前一個施工關完成後開工；開工時把標「開工時細化」的步驟寫定。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**未開始**（2026-09-24）

## 範圍

- 驅動 core 狀態機
- git adapter（先記錄再建立）
- runner（`command` 關卡）
- forge local（merge-tree + CAS update-ref）
- supervisor、reconcile
- `PipelineState` 存成 task 資料列上的快照欄位，與 task 一起 CAS，不靠 replay events 重建（第 5 施工關 P4）
- 這把 D32 延伸到 `PipelineState`，要有 golden JSON 測試；第 1 施工關的測試要重跑
- 從第 6 施工關移來（[gate-06 P9](gate-06-daemon-holder.md#p9hooks-與-binding-快照依賴規則)，使用者 2026-09-26 追認）：綁定／釋放 worktree 時呼叫 `agend_shim::install_hooks`／`uninstall_hooks`；daemon 為每個 agent 寫唯讀 binding 快照 `$AGEND_HOME/bindings/<instance>.json`（見 [GLOSSARY](../GLOSSARY.md)「binding 快照」）；`check-deps` 加規則：`agend-daemon` 不能依賴 `agend-holder` 或 `agend-shim`

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-daemon` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept pipeline` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.x.y`（目前是 `agend 0.0.0`）。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

1. 暫存 repo + 假 agent：建立 task（開工時細化：暫存 repo 與假 agent 的準備指令，以及它印出的 repo 路徑 `<暫存 repo>`、home 路徑 `<home>`）。

   ```bash
   agend task create --role dev "demo"
   ```

   應該看到：接著 `agend status` 依序顯示 work → submit(local) → command → approval → merge。

   - [ ] 通過

2. 確認 merge 結果（把 `<暫存 repo>` 換成第 1 步印出的路徑）。

   ```bash
   git -C <暫存 repo> log --oneline -1 main
   ```

   應該看到：main 最新的 commit 是這個 task 的 merge。

   - [ ] 通過

3. 確認清理（同上，換掉 `<暫存 repo>`、`<home>`）。

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
