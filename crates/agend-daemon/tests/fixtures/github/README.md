# GitHub CLI 回覆捕獲

> **TL;DR**
> - 2026-10-07，以真 `gh 2.102.0` 對 GitHub 執行唯讀 GET。
> - 保留真 status line 與 CLI 混用 LF／CRLF 的 header 邊界；JSON 只摘出列明欄位。
> - 不含 token、請求 headers 或帳戶設定；不是 GitHub merge 真測。

| 檔案 | 來源 | 保留欄位 |
|---|---|---|
| `pull.http` | `gh api --include --hostname github.com repos/suzuke/AgEnD/pulls/155` | number、state、merged、merge_commit_sha、html_url；head／base 的 ref、sha、label 與 repo 的 id、full_name、html_url |
| `not-found.http` | 同上，PR number 換為 `999999999`（exit 1） | message、status |

移除其餘 headers，只保留固定 Content-Type；body 以 JSON 重新排版，欄位值不改。這些 fixtures 核 consumer 可處理真 CLI 的成功與 HTTP 錯誤輸出，不用手寫理想化 producer 格式。

SHA-256：

- `pull.http`：`33aadc0e844d4f4974100622af99fecf6671ac4dc1971fc21b1b50d308c87d75`
- `not-found.http`：`d652f3162b673d4bb369e7f1874c8011bd2574179e581d74e5a3cb2506d6bde2`

API：[GitHub pull requests](https://docs.github.com/en/rest/pulls/pulls)。合併時須傳完整 approved head 的 `sha`；head 不符時 API 回 409。checks 沿用 D29 的 command stage，見 [gh pr checks](https://cli.github.com/manual/gh_pr_checks)。

合併收據另以同版 gh 唯讀擷取既有 PR #154：`repos/suzuke/AgEnD/pulls/154` 的相同欄位保存為 `merged-pull.json`；`repos/suzuke/AgEnD/git/commits/a6cdb4c64b244ca86a23de0a2139a4eee29e5d54` 只保留 sha／parents／message 為 `merge-commit.json`。沒有重新呼叫 merge。核 approved head 必須是該 merge commit 的第二個 parent。

- `merged-pull.json`：`af1904093d4649424360363d977cec623115218e26c67f3716b8540d99f8b030`
- `merge-commit.json`：`5b5026d8d506f66f75a6a766414f4544d7f6537dfed0c8464d82e740dc5cb8e2`

`repository.json`：2026-10-07 以 gh 2.102.0 唯讀 GET `repos/suzuke/AgEnD` 捕獲，只保留 id／full_name／html_url；SHA-256 `d6b2539370d52264ee3d42790357e2c173258120ca9a2a9302f6234350c5ef8a`。提交測試沿用原 PR 捕獲，明確注入任務 marker／遺失回覆，不宣稱真 GitHub 建立 PR。
