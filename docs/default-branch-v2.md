# GitHub 預設分支改為 v2：執行安排

> **TL;DR**
> - 使用者 2026-10-12 指示：確認 Roadmap 只剩 Claude 相關後，再開始安排切換。
> - 目前仍有非 Claude 驗收與發布待辦；GitHub 預設分支維持 `main`。
> - 切換只改 GitHub 預設分支為 `v2`，保留舊 `main`，不重新命名分支。

## 已確認

- #158／#159 已依序合併至 v2；#159 的合併 tree 與通過雙平台 CI 的提交相同。
- `v2` 已設定 macOS／Ubuntu 必要 checks，禁止 force push 與刪除。現有設定為 `strict=false`、管理員不強制套用；本次不默默改動保護政策。
- 2026-10-12 查無額外 repository rulesets；兩支 PR 合併後查無其他開放 PR。切換當下須重新核對。
- `.github/workflows/ci.yml` 對所有 push／pull_request 執行，沒有 main 限定。
- `release.yml` 有 workflow_dispatch，但 push 仍只列已完成的 `feat/g13-install`。切換前須將這個舊條件改為正式分支政策並覆核，避免後續修改漏建產物。
- README 必須明列 pre-release 狀態、可執行安裝路徑、Claude 未驗範圍。沒有公開 Release 前不提供不存在的 Brew／下載命令。

## 切換前的順序

1. 依 [第 13 關現況](gates/gate-13-status.md) 逐項關閉非 Claude 待辦；若要延後驗收或發布，記錄使用者決定，不以刪除清單充當完成。
2. 合併新手 README 與現況文件；核對 CI、release 觸發條件、開放 PR 的 base，以及 branch protection／rulesets。
3. 核對 v2 固定 head 的 CI，保存 main／v2 SHA 與原 default_branch，形成切換前快照。
4. 條件成立後把 GitHub repository 的 default_branch 設為 `v2`。這不建立 tag、不發布 Release，也不刪除 main。
5. 重新查詢 default_branch、v2 保護設定、首頁 README；用新的空目錄 clone，確認預設 checkout 為 v2，完成後移除該驗證 clone。

## 回復與本機影響

需要回復時將 default_branch 設回原本的 `main`；保留兩個分支及其提交。現有 clone 不會自動切換工作分支，使用者可在自己的 clone 執行 `git remote set-head origin -a` 更新遠端預設指標。

## 下一步

先交付 README 與明確的剩餘項目表。切換條件尚未成立，這份安排不代表已執行 GitHub 設定變更。
