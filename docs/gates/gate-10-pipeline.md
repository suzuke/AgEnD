# 第 10 施工關：daemon：流水線（`pipeline`）

> **TL;DR**
> - daemon 用 core 的狀態機把一個 task 從派工推到 merge：建 worktree 並裝 hook、跑 checks、agent 審查、你核准、在本機 repo merge；daemon 被硬殺後接著做，不重複 merge。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：完成事件收尾驗證後，由 agent 帶使用者補驗；使用者確認前不 merge。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑 [人工驗收頁開頭的設定](gate-10-manual-start.md#你親自驗收)，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**驗收中**（2026-10-02）：P1–P11 使用者已確認，提案 #130 已 merge；實作在 `feat/gate-10-pipeline` worktree。原版本的完整自動驗收、全新 verifier r12／r15、雙平台 CI 與人工主流程通過；人工找到的 watch／核准事件問題正在收尾，修正後待驗證、事件補驗及使用者確認；[draft PR #143](https://github.com/suzuke/AgEnD/pull/143) 未合併。見 [驗證證據](gate-10-verification.md) 與 [pipeline runtime](../architecture/pipeline-runtime.md)。前置施工關 #125、#132、#131、#136 及第 11 施工關 B 段 #140 已完成並 merge；本關 client protocol 1.3。

## 範圍

- 驅動 core 狀態機：daemon 裡唯一的 pipeline 迴圈推進 task，每一步先 CAS 存檔再做事（P1）
- `PipelineState` 存成 task 資料列上的快照欄位，與 task 一起 CAS，不靠 replay events 重建（第 5 施工關 P4）；D32 延伸到 `PipelineState`，golden JSON 測試；core 加一個純函式列出「還在等結果的要求」；第 1 施工關的測試要重跑（P2）
- team、角色與分派：`teams` 表、instance 的 team／角色、core `policy::assign`；supervisor 與 task 的關係（P3）
- worktree 與 binding：先記錄再建立、固定命名空間、審查 worktree、結束時清理並把 WIP 存成 patch（P4）
- 從第 6 施工關移來（[gate-06 P9](gate-06-daemon-holder.md#p9hooks-與-binding-快照依賴規則)，使用者 2026-09-26 追認）：綁定／釋放 worktree 時安裝／移除 agend hook；daemon 為每個 agent 寫唯讀 binding 快照 `$AGEND_HOME/bindings/<instance>.json`（見 [GLOSSARY](../GLOSSARY.md)「binding 快照」）；`check-deps` 規則「`agend-daemon` 不能依賴 `agend-holder` 或 `agend-shim`」照舊成立（P4）
- git adapter：daemon 自己跑的 git 全部經 `Runner`、帶 timeout、不經 shim、不跑任何 hook（P5）
- runner（`command` 關卡）：每次 checks 一個新的 detached worktree、在寫入沙箱裡跑（macOS `sandbox-exec`、Linux `bwrap`，沒有沙箱就不跑）、環境白名單、timeout（P6）
- forge local：merge-tree + CAS `update-ref`、merge 前才處理 main 前進（D14）、merge 不會做兩次（P7）
- 「需要你」的新來源：人工核准、請示、task 失敗、merge 被擋、缺角色、沒有沙箱（`sandbox-missing`）（P8）
- 開機對帳（reconcile）與四次開機的重啟契約（P9）
- 本關擁有的命令與請示（P10）
- 什麼是假的、什麼是真的（P11）

**與第 9 施工關的分工**（協調者 2026-09-26 定）：

| 誰 | 負責 |
|---|---|
| 第 10 施工關（本關） | 請示 `ask` 的 daemon 端與儲存；`done`、`result`、`review approve`／`changes`、`block`、`unblock`、`remind`、`task create` 的 daemon 端處理；`agend workflow`、`agend team` 命令（CLI 與 daemon 兩端）；新的操作者請求 `task_cancel`；`agend doctor` 的沙箱工具一列（沙箱是本關加的，所以本關自己加這一列） |
| 第 9 施工關 | 上面那些 agent 命令的 CLI 語法、client 接線、錯誤碼對應、哪些命令斷線後可以重送；ticket 格式 `<task>/<stage>/<attempt>`；`agend inbox --after`（唯讀游標）；`operator` 請求；`agend doctor` 本身（沙箱那一列由本關加） |

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子。文件已定、這裡不重問的：狀態機是純函式 `step`、trait 在 core（D27）；`command` 關卡與 git 都經 `Runner`（D28）；forge 維持 3 個方法、GitHub CI 用 `command` 關卡接、forge local 沒有 change id（D29）；workflow 用 TOML 存 DB、task 固定建立時的版本（D19、D21）；人工核准 merge＝`approval(by = "human")`（D20）；一個 agent 一個 task、返工回持有者（D33 第 1、2 點）；main 前進的保留規則（D14）；worktree 只由 daemon 建、固定命名空間、先記錄再建立、WIP 存 patch、開機與每日的單一對帳（[pipeline](../architecture/pipeline.md#worktree-與-branch-生命週期)）；hook 設計與 binding 快照格式（第 3 施工關 T1、T21）；`PipelineState` 存成快照、不重播事件（第 5 施工關 P4）；daemon 起 holder、環境白名單、重起 3 次後 `failed`（第 6 施工關 P3、P6）；「需要你」的欄位、`resolve_attention` 只收操作者（第 8 施工關 P2、P5）；請示是對話（D35）；保留期限（D31）；結果類命令帶 ticket（第 9 施工關 P2）。
### P1：誰推進 task、怎麼存檔

[完整提案與確認紀錄](gate-10-proposal-runtime.md)。

### P2：`PipelineState` 快照、怎麼防偽造、重開機後要重做什麼

[完整提案與確認紀錄](gate-10-proposal-runtime.md)。

### P3：team、角色、分派；supervisor 跟 task 的關係

[完整提案與確認紀錄](gate-10-proposal-runtime.md)。

### P4：worktree、binding、hook、binding 快照

[完整提案與確認紀錄](gate-10-proposal-worktrees.md)。

### P5：daemon 自己跑的 git

[完整提案與確認紀錄](gate-10-proposal-worktrees.md)。

### P6：`command` 關卡（runner）

[完整提案與確認紀錄](gate-10-proposal-worktrees.md)。

### P7：forge local 與 merge

[完整提案與確認紀錄](gate-10-proposal-merge.md)。

### P8：「需要你」的新來源與協定補充

[完整提案與確認紀錄](gate-10-proposal-merge.md)。

### P9：開機對帳與四次開機的重啟契約

[完整提案與確認紀錄](gate-10-proposal-commands.md)。

### P10：本關擁有的命令與請示

[完整提案與確認紀錄](gate-10-proposal-commands.md)。

### P11：什麼是假的、什麼是真的

[完整提案與確認紀錄](gate-10-proposal-commands.md)。

### 已決定（原本的待你決定）

[完整決定紀錄](gate-10-proposal-boundaries.md)。

### 本關不做（明確列出）

[範圍與支援邊界](gate-10-proposal-boundaries.md)。

### 已知風險（開工時處理）

[原提案的開工風險](gate-10-proposal-boundaries.md)。

## 自動驗收（完成定義）

- [x] `~/.cargo/bin/cargo test -p agend-core`、`-p agend-testkit`、`-p agend-shim`、`-p agend-daemon`、`-p agend` 單獨通過，包括：
  - `PipelineState` golden JSON、舊快照讀得回來、竄改的快照 `restore` 失敗不 panic（P2）
  - 探索器新不變量：`outstanding_actions` 等於送出、還沒被回應的要求（P2）
  - binding 快照型別搬家後 golden JSON 不變、shim 全部測試照過（P4）
  - STO 新規則「存檔與事件同成同敗」對 `FakeStore` 與真 store 都過，有 mutant（P1）
  - 分派：沒有空的 dev 時排隊、空出來後自動派出；沒有角色 → `no-role`，`team join` 後消失（P3、P10）
  - daemon 的 git 呼叫都有 `-c core.hooksPath=/dev/null`、都沒有 `AGEND_*`（P5）
  - checks：每次一個新目錄、跑完就刪；`--fail-checks-once` 回 work 再通過；逾時停掉整個 process group、餵 `CommandFinished{exit_code: None}`、task 回 work；沙箱：寫到 worktree 與 `TMPDIR` 的 check 通過，寫到 home、`$AGEND_HOME`、其他 repo、canonical 的 refs 的 check 失敗且檔案不存在，網路照常可用；找不到沙箱工具（測試用 debug-only 的覆寫指到不存在的路徑）→ check 不跑、`sandbox-missing` 出現、`retry` 後照跑；改 `<worktree>/.git` 或 `.git/worktrees/<run>/commondir` 失敗、daemon 清理時沒有執行任何 checks 留下的設定（`core.fsmonitor` 的回歸測試）；一次 check 寫 `CARGO_HOME/config.toml` 之後，下一次 check 看不到它；沙箱 profile 故意寫壞 → 沒有標記檔 → `sandbox-missing`、不是 checks 失敗；check 刪掉或把標記檔換成 FIFO → checks 失敗、daemon 不卡住；check 連 `run/daemon.sock`、`run/holders/*.sock`、`/private/tmp`（Linux 是 `/tmp`）下的 socket、以及經 symlink 指到它的路徑都失敗，HTTPS 照常；check 留下的背景子程序在 check 結束後不存在——**例外**是 macOS 上 `setsid` 的子程序（已知風險），測試改成斷言它連不上任何 unix socket、寫不了沙箱外（P6、P8）
  - merge：main 沒被 checkout、只被乾淨的 canonical checkout、被 dirty 的 canonical 或別的 worktree checkout 三種；空 branch 的 `done` 被拒；main 前進 → rebase、保留核准、重跑 checks；已 merge 的由 trailer 找回、`merge_intent` 當後備、手動 merge 的記成完成（`MergeCompleted`，註明 merged outside agend）（P7）
  - 「需要你」六種來源、`resolve_attention` 的 `note`；舊 attempt 的 `approval:` id 回 `unknown_attention`；重開機後 `waiting_since` 不變（P8）
  - P10 每個命令的 daemon 端；請示的一輪提問、回答、追問、結論；`remind` 跨重開機照樣送出；`task_cancel` 只收操作者
  - `delivery = inbox`：讀取不改狀態；結果命令帶同一個 ticket 時派工訊息變 `confirmed`（P11）
- [x] 四次開機（P9）通過：`crates/agend/tests/` 裡真的 `agend daemon`，開機 1 被測試 `kill -9`、開機 3 在 failpoint 中止，開機 4 的檢查全部成立；反向檢查（每次新的 `AGEND_HOME`）在開機 2 失敗
- [x] 兩個 failpoint 各一個測試：在那一點中止、重開後 task 照樣走到 done，main 上的 merge commit 剛好一個；另一個測試把 `merge_intent` 清掉（模擬斷電）後照樣由 trailer 找回
- [x] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [x] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [x] `~/.cargo/bin/cargo xtask accept pipeline` 通過，並印出下方「你親自驗收」用到的 demo
- [x] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新；想改的共用文件列在 PR 裡由你決定
- [x] 本輪 verifier 自身 daemon／holder 已停止、成功 fixture 已刪；歷史失敗資料依 manifest 保留為證據，見驗證證據頁；kill 只對自己起的、大於 1 的 pid
- [x] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

由 agent 依 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收) 一次帶一步、比對輸出後才給下一步。

- [步驟 1–5：demo、四次開機、啟動與核准 merge](gate-10-manual-start.md)
- [步驟 6–11：清理、防護、WIP、重啟、沙箱與收尾](gate-10-manual-finish.md)

## 驗收紀錄

由 agent 依使用者貼出的輸出記錄；事件呈現的修正仍待補驗，不能視為 merge 授權。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
| 2026-10-02 | 人工主流程通過；收尾待複驗 | `45e957e`；[完整人工紀錄](gate-10-manual-record.md)，11 步已操作；watch／核准事件與指令修正尚待驗證，未 merge。 |

## 進度紀錄

r12 CONFIRMED `dfe5bc6`；workspace 771 passed／2 ignored、完整 accept 713 passed／2 ignored、三組探測及四個 CI job 通過。原失敗另保留；詳見 [驗證證據](gate-10-verification.md) 與 [完整進度紀錄](gate-10-progress.md)。

## 下一步

完成收尾修正的自動／全新獨立驗證後，由 agent 帶使用者補驗 watch 與核准事件，再等明確 merge 確認。
