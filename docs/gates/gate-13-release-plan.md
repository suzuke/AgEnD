# 第 13 關版本與發布交付計畫

> **TL;DR**
> - 目前為 0.0.0 驗收產物，PR #158–#161 已合併，尚未公開發布 v2 Release。
> - 非 Claude 項目先完成固定提交覆核；Claude 真 canary／跨版往返依使用者指示暫緩。
> - #158–#161 已經使用者核准合併；正式版本號與公開發布仍待確認。

## 交付順序

| 步驟 | 可檢查結果 | 執行邊界 |
|---|---|---|
| 固定提交 | fmt、clippy、check-deps、accept install、全新 context 覆核與 CI | feature branch 執行；保留失敗與成功各自範圍 |
| 使用者驗收 | 依序操作 init／doctor、首任務、需要你的處理；記錄實際輸出 | 一次帶一步，不把自動 operator 核准寫成人工驗收 |
| Claude 補驗 | 專用認證、便宜模型、固定兩版本 canary／非空歷史往返 | 目前暫緩；不碰共享 Keychain、trust 或其他 sessions |
| 合併 | 已完成：#158 → 64db11a、#159 → ea1e487、#160 → 7c67061、#161 → c23b3f6 | 這四批自有 worktree／本機分支已清理；main 為 Rust v2 |
| 版本 | 建議第一個公開候選版本 2.0.0-rc.1，更新 workspace／lockfile | 尚未採用或建立 tag；若提前提供 preview，須明列 Claude 未驗證 |
| 產物 | 該版本固定提交的四平台原生 archive、manifest、SHA256SUMS、formula，七 jobs 成功 | 不挪用不同 SHA 的舊產物；Actions 保留 14 天，正式發布須另保存 |
| 公開發布 | 使用者核准 tag／Release 文案與四平台產物後發布，再更新 tap | 實際公開 URL 下載與 Brew 安裝另驗；不可用 file URL smoke 冒充 |

## 已有安裝證據

- `cargo install --locked --path crates/agend`：固定 6ac8ac6、隔離 prefix／HOME，以預備 API-only 認證完成真 OpenCode 首任務；init 至完成 13.95 秒，不含編譯安裝。
- 下載 archive：固定 4e739122ec9362413cf679d570788dc75d2ed578 的 macOS ARM64 產物，驗 manifest／內容／雜湊後安裝；下載安裝 13.39 秒、init 至任務完成及清理 40.67 秒，共 54.06 秒。
- 兩次均為 OpenCode 1.18.34、opencode-go/gpt-6-luna；精確 committed README blob 經驗證器核准後，task done 且唯一一次雙親 local merge。不是新帳戶註冊或 GitHub merge。
- [release-artifacts 38159327386](https://github.com/suzuke/AgEnD/actions/runs/38159327386) 的四平台 archive、formula、雙平台 Brew 全部成功。Brew 使用同輪 file URL，不證明公開 Release URL 已存在。

## 更新與退回

新 binary 先安裝至新的 prefix，以隔離 AGEND_HOME 執行 init／doctor；核版本、資料目錄與服務 plan 後才安排正式 daemon 停機更新。不要覆寫執行中的 private launcher 或刪除 holder 資料。資料格式未保證向下相容：退回舊 executable 前須確認該版本能讀目前 DB；有 migration 時使用停機前完整備份，不直接覆蓋在線 DB。

backend 換版使用正式 `backend switch prepare/status/activate/rollback`，不是替換 fleet 的 program 檔案。結果不明先查 status，不自動重送。操作與持久化限制見 [版本管理](../architecture/backend-versions.md)。

## 下一步

本批獨立驗收與合併前雙平台 CI 已通過；接續核對剩餘人工／環境驗收，準備正式發布版本。安裝命令及產物驗證方式見 [安裝文件](../install-release.md)；Claude 暫緩、使用者驗收及發布核准分開追蹤。
