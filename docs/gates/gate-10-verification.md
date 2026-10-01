# 第 10 施工關：驗證證據

> **TL;DR**
> - 本機 pipeline 已實作；產品碼固定在下列 head，雙平台 CI 通過。
> - 全新 verifier r12 CONFIRMED；兩個 deep explorer 未執行，人工驗收尚未完成，PR 未合併。
> - 下一步：由 agent 一次帶使用者驗收一步；確認後才能 merge。

## 範圍與固定版本

- [Draft PR #143](https://github.com/suzuke/AgEnD/pull/143)，branch `feat/gate-10-pipeline`，base `v2`。
- 產品 head：`dfe5bc692cc21a56c4ac9ec596da2765ddab543a`。
- Base：`85e3fa589a7208b8c440c81333dd1e0146111a47`。
- 實作 worktree：`/Users/suzuke/AlphaCR-worktrees/AgEnD-v2-pipeline`。
- Fresh verifier worktree：`/Users/suzuke/AlphaCR-worktrees/AgEnD-v2-g10-verify-r12`。

## CI 與本機回歸

上述產品 head 的四個 CI job 都成功，實際 no-std build ok，沒有 SKIPPED。

| Event | Run | Ubuntu job | macOS job |
|---|---|---|---|
| PR | [36928430310](https://github.com/suzuke/AgEnD/actions/runs/36928430310) | 110591420693 | 110591420420 |
| push | [36928423907](https://github.com/suzuke/AgEnD/actions/runs/36928423907) | 110591399105 | 110591398324 |

本機九個 `pipeline_archive*` target 共 31 個回歸通過，並通過 fmt、workspace clippy 與 check-deps。
覆蓋大 binary、merge-only 解法、index-only bytes、隱藏旗標／stat cache、顯示設定、ignored 檔案、nested metadata 的保留與一般路徑還原。
成功封存以真 git apply 比對原始 bytes；失敗須保留原 worktree／index，不發布不完整 patch。

## 獨立驗證

r12 以全新、無相關 context 的 agent 在自己的 frozen worktree 驗證；結論 CONFIRMED，限下列已驗行為與三組 macOS 探測。
完整報告 `/private/tmp/g10-r12-report.md`；exact commands／exit／env／counts、source、expected bytes 與 SHA256 manifest 在 `/private/tmp/g10-r12-logs`。

| 命令（`~/.cargo/bin/cargo`） | Exit | 證據檔 |
|---|---|---|
| `build -p agend -p agend-testkit --bins` | 0 | `01-build.*` |
| `fmt --all -- --check` | 0 | `02-fmt.*` |
| `clippy --workspace --all-targets -- -D warnings` | 0 | `03-clippy.*` |
| `test --workspace` | 0 | `07-workspace-corrected.*`，771 passed／2 ignored |
| `xtask check-deps` | 0 | `05-deps.*`，實際 no-std |
| `xtask accept pipeline` | 0 | `08-accept-corrected.*`，713 passed／2 ignored |

計數排除 signal child 重複 summary；workspace 與 accept 的重疊測試沒有相加。
accept 含兩個 crash failpoint 的四次開機、new-home negative、沙箱與 hooks，結尾兩行為 `pipeline demo: all sections passed`、`gate 10 (pipeline): checks passed`。

有限三組探測都通過：

- WIP：staged-only binary、換行檔名、隱藏旗標、ignored 檔／symlink、顯示設定、轉換 blocker；失敗保留原 index／bytes，硬重啟後 repair／wake 清理、釋放 capacity，真 git apply 還原。
- Workflow/context：實際 TOML producer 建 v2，舊 task 固定 v1；Result／output 跨重啟進入人工核准與角色交接，planner WIP 可還原，新 task 使用 v2。
- Receipt/quorum：count2 部分核准跨重啟不重複，changes 返工與舊 attempt 拒絕，再核准並在 after-main-moved crash 後恢復，只 merge 一次。

原六項中 workspace exit 101／accept exit 1 因 verifier PATH 漏掉 `/opt/homebrew/bin/npm`；原紀錄未算通過。修正 PATH 後兩項另跑完整命令，沒有以單測或 standalone demo 取代。
首次三組 probe exit 101：第 2／3 組通過，第 1 組在 TaskCreate 回應後太早讀 fleet；修正等待後同組 exit 0。原 source／log／exit／fixture 保留，未加第四組。

收尾：verifier 產品碼未改、temporary source 已移除、HEAD／status 乾淨，自身程序已停止，成功 fixture 已清理。原失敗 fixture 與 holder lock 檢查見 `fixtures-manifest.json`／`retained-fixture-locks.json`；歷史反例沒有刪除。
本頁及分頁整理屬文件修改；產品碼仍是上列 head，最後提交的 CI 狀態見 PR。

## 尚未驗證與支援邊界

- 兩個既有 ignored deep explorer 未執行：`a4r3_dead_end_explorer`、`random_accepted_workflows_always_finish_deep`。
- 此關用 fake worker 驅動真 daemon／Git／shim 與 local forge；其他 backend／GitHub forge 在第 12 施工關。
- 行尾／內容轉換、nested Git metadata／gitlink、特殊檔案與未解 index 衝突先保留原資料、回報 Failed；操作者處理後重試。
- 使用者尚未完成 [親自驗收](gate-10-pipeline.md#你親自驗收)，此頁不代表 merge 授權。

## 下一步

agent 先帶使用者執行 `cargo xtask accept pipeline`，逐項比對輸出，再給下一步。
