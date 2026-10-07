# 第 12C：GitHub forge

> **TL;DR**
> - 正式 GithubForge、pipeline 接線與持久化遠端收尾已實作；尚未驗收完成。
> - 離線原生 FRG 1–10 與真正 daemon 程序的重啟、main 前進、取消／WIP 收尾已通過。
> - 下一步：確認遠端 base 競爭政策、受控 GitHub 真測、全新 verifier、CI 與合併。

## 正式接線

`SelectedForge` 依 task 固定 workflow 選 local／github；未知或混用 forge 在 workflow 驗證時拒絕。submit、checks、merge 與重啟對帳都使用相同 forge。GitHub 暫時受阻時保留任務與 binding，供 operator Retry。

GitHub checks 沿用 D29 的 command 關卡及 `{pr}`／`{head}`／`{branch}` 展開。checks 前確認已推送 head，command 前後再讀 PR head；遠端 head 改變就受阻，不套用過期 checks。main 只在乾淨 checkout 且可 fast-forward 時同步，不重設使用者提交；main 前進則沿用 D14 rebase、重新 push 同一 PR、重跑 checks。

## 傳輸與持久化

daemon 使用自己的 gh 登入連 github.com，排除 agent gh shim。token 只留環境或私人設定，不放命令列；repo 由明確 origin 解析。每次 API 呼叫限 60 秒及 runner 的 5 MiB 輸出上限，傳輸層不重送 mutation。

schema 0013 保存固定 task／本機 repo／GitHub repo ID／branch／nonce、PR number 與 revision。CAS 拒絕 stale revision、改綁 PR／repo 及兩個 task 佔用相同 repo ID／branch。

push 前保存 intent，以完整舊 SHA 的 force-with-lease 更新單一 ref；未確認 intent 先核遠端 head，不覆蓋外來更新。PR create 前保存 attempt；回覆遺失只搜尋固定 branch／base 與 task nonce，不重送 create，不認領外來 PR。

merge 前後核 repo ID 與 ownership marker。PUT 帶 approved SHA；已 merged 或回覆遺失時只讀原 PR，核 merge commit 的兩個 parents 與 approved head，沒有完整收據就受阻。API 的 head CAS 見[官方端點](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request)。

## 遠端收尾

任務完成、取消、失敗及 daemon 重啟都接入收尾。取消時核自有 PR 與 head 後關閉 PR；刪除 branch 使用精確 SHA lease。close／delete attempt 先存 DB，成功回覆遺失可讀回確認；若同名 branch 已重建，即使 SHA 一樣也不重刪。完成收尾的 ledger 不可重新寫入或重新 push。

remote 失敗會產生 `cleanup-remote` attention，仍繼續本機 WIP 存檔、解除 binding 與釋放 agent；一般 wake 不反覆發送 remote 操作。operator Retry 才重新對帳。已送出但結果不明的 close／delete 不靠 Retry 重送；必要時由人確認並關閉自有 PR／刪除自有 branch，再 Retry 確認結果。

## 已驗證範圍

| 驗證 | 證據與範圍 |
|---|---|
| API producer | gh 2.102.0 真唯讀 PR／404／repo／merge commit 捕獲；shell argv 保留 literal 字元，token 不入 argv |
| SQLite | 跨重開保留 intent／attempt；拒 stale revision、換綁、清除 cleanup attempt 及完成後改動 |
| `github_forge` | 正式 Forge 的 FRG 1–10；create／merge／close／delete 遺失回覆、外來 branch 更新、同 SHA 重建、dirty main 保存 |
| `github_pipeline` | 真 daemon／holder／fake-worker 程序；核准前強制重啟後只建立／merge 一次，WIP／remote 清理；main 前進後 checks attempt 2 並保留核准；取消關 PR、保存 WIP |
| pipeline 受阻收尾 | 遠端失敗仍釋放本機容量；wake 不重送，operator Retry 可完成 |
| 契約反例 | local FRG 與 contract mutant 仍通過；不同 branch 使用不同 task ID，原斷言保留 |

原生測試的獨立 API 程序沿用真捕獲形狀，Git 自己產生 object／tree／parents；固定 GitHub URL 只映射到自有 bare repo。沒有外部 GitHub mutation，也不認證 GitHub 分支保護政策。測試結束移除自有 daemon／holder、repo 與 home。

## 待決策與剩餘工作

GitHub merge API 只有 approved-head CAS，沒有 expected-base CAS。已提出要求目標分支嚴格 up-to-date 保護（建議），或接受最後查核到 merge 間 main 前進的空窗；尚待使用者決定。完整 merge 認證及外部寫入驗證暫不進行，其餘實作／測試持續。

尚需受控 GitHub 真 repo 流水線、全新 verifier、完整 CI、合併與施工目錄清理。本頁不宣稱第 12C 完成。

## 下一步

先完成 base 政策，固定真測版本、repo、命令與有限預算，再執行受控真測。離線重驗：先 `cargo build -p agend -p agend-testkit --bins`，再 `cargo test -p agend-daemon --test github_forge --test github_pipeline`。
