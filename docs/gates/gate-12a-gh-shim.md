# 第 12A：所有 backend 共用 gh 防護

> **TL;DR**
> - D40 P4 選 A：agent 的 PATH 加入 gh shim，避免自行 merge、批准 PR 或印出 token。
> - 拒絕在真正 gh 執行前完成；其餘命令保留原 argv、cwd 與 exit status。
> - 下一步：本批待全新 verifier／固定 head CI 與使用者確認合併，之後接完整 Claude Driver。

## 狀態與範圍

2026-10-04，`feat/gate-12a-gh-shim`，基線 #149 merge `6dd552e`，[draft PR #150](https://github.com/suzuke/AgEnD/pull/150)，首個實作提交 `d4853ad`。這批只實作 [D40 P4](../decisions/d40.md) 的共用防護，沒有啟動真 Claude／Codex／OpenCode、讀取憑證或呼叫 GitHub 的寫入 API。第 12A 仍未完成。

daemon 每次 boot 維護 `$AGEND_HOME/bin/{git,gh,kill,killall,pkill}` symlink，指向當前 agend binary；所有 backend 的 holder 都先找到此目錄。一般操作者的 PATH 不加 shim。既有 `AGEND_SHIM_BYPASS=1` 保留，bypass 寫 audit；daemon 的 agent 環境白名單不傳入此變數。

## 拒絕規則

| 呼叫 | 行為 |
|---|---|
| `gh pr merge`，含 `--auto` | 拒絕，merge 經 agend pipeline |
| `gh pr review --approve`／`-a` | 拒絕；short flag 組合亦檢查；明確 `--approve=false` 不視為批准 |
| `gh auth token` | 拒絕，避免印出登入 token |
| `gh api repos/<owner>/<repo>/pulls/<id>/merge`／`reviews[/…]` | 拒絕所有方法，含讀取；需要讀 PR 時使用 `gh pr view` |
| `gh api repos/<owner>/<repo>/merges` | 拒絕直接合併 branch |
| `gh api graphql` inline query | 拒絕 `mergePullRequest`、`enablePullRequestAutoMerge`、`enqueuePullRequest`、`mergeBranch`、`addPullRequestReview`、`submitPullRequestReview` 識別字；alias 不隱藏原 mutation 名稱 |
| GraphQL `--input` 或 `-F query=@…` | 拒絕：不讀檔案／stdin，不能在執行前核對操作；改用可核對的 inline query 或請操作者處理 |
| 其他呼叫 | 原樣交真正 gh；例如 `pr view`／`create`／`checks`、明確 comment／changes review、issue comments、一般 REST 與 inline GraphQL read query |

解析保留 `-R`／`--repo` 與選項值，`--body '--approve'` 不算批准。API 可在 endpoint 前帶 `-XPUT`、`-f`／`-F`／`-H`；處理 URL、enterprise `/api/v3` prefix、query 與 percent-encoded path。沒有安裝真正 gh 時，禁用命令仍回拒絕；放行命令回 cannot find／exit 127。PATH 內指回 agend 的 symlink／hard link 不算真工具。

旗標依 [gh pr review 手冊](https://cli.github.com/manual/gh_pr_review)；API 的 fields／method／input 形式依 [gh api 手冊](https://cli.github.com/manual/gh_api)。mutation 名稱見 [GitHub Pull requests](https://docs.github.com/en/graphql/reference/pulls) 與 [Branches](https://docs.github.com/en/graphql/reference/branches)。拒絕規則是本專案政策，並非 GitHub 本身限制。

## 輸出與限制

拒絕 exit 1，stderr 三行說明理由及下一步；stdout 空。gh audit 只保存 `tool=gh`、instance、refuse／bypass、固定 code 與命令類型，不記 endpoint、headers、payload 或原始完整 argv。audit 寫入失敗不改拒絕結果。

這是第 3 施工關沿用的防誤操作安全帶，同 uid 可直接執行絕對路徑 gh、改 PATH、使用 alias／extension／直接 API 等繞過。shim 不展開 gh config 的 alias、不檢查互動 review 選項、不驗憑證或網路權限；不是沙箱。GraphQL 識別字檢查不代替完整 schema／query 驗證。

native holder probe 使用 shell 替身驗 runtime 的 PATH，不代替 D40 P8 選定真 CLI 版本的 Bash PATH 驗收；Claude Driver、settings ownership、啟動提示、Interrupt、orphan sweep 與完整 DRV-6／9 仍待接。

## 自動驗證與清理

`gh::tests` 三個 unit tests 驗常見 argv、選項值、REST 與 GraphQL 正反例。`agend/tests/shim_gh.rs` 五個 native cases 驗真正 binary 的 argv[0] 分派、拒絕時工具零次執行、audit 不存敏感輸入、原始 bytes／cwd／exit、操作者與明確 bypass、缺工具／PATH loop，以及三種 backend 的真 holder PATH。

真正 gh 由只記錄 argv、固定 exit 23 的 fixture 替身提供，沒有 GitHub 網路。holder probe 結束時送 Shutdown，確認 own locks 全部釋放；TempDir／Lab 清掉自己的檔案與 home。

```bash
# 在本批 checkout 內；所有 build artifacts 都放自己的暫存 target。
(
  set -e
  gate12_gh_target=$(mktemp -d /tmp/agend-g12a-gh-check.XXXXXX)
  export CARGO_TARGET_DIR="$gate12_gh_target"
  trap '~/.cargo/bin/cargo clean; rmdir "$gate12_gh_target"' EXIT
  ~/.cargo/bin/cargo test -p agend-shim
  ~/.cargo/bin/cargo test -p agend --test shim_gh
  ~/.cargo/bin/cargo xtask check-deps
)
```

正式驗證另跑 workspace tests、fmt／clippy 與實際 no-std；merge 前提供固定 head 的全新 verifier 結果及會自行建立、清掉驗證 worktree 的重驗入口。

## 進度紀錄

- 2026-10-04：共用 gh 防護及三 backend 真 holder probe 已實作；首輪 3 unit／5 native cases 通過，workspace clippy 通過；完整驗證另核，尚未合併（#150）。

## 下一步

固定本批 head，完成全新驗證與 CI，提供可重驗指令並等使用者確認；合併後刪除本批 worktree／branch，接 Claude Driver 與啟動設定。
