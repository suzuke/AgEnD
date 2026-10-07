# 第 12C：GitHub forge

> **TL;DR**
> - 正式 GithubForge、pipeline 接線與持久化遠端收尾由 #157 交付。
> - 離線原生 FRG 1–10 與真正 daemon 程序的重啟、main 前進、取消／WIP 收尾已通過。
> - 下一步：#157 完整驗收與 CI 通過後合併並清理；完成狀態於合併後生效。

## 正式接線

`SelectedForge` 依 task 固定 workflow 選 local／github；未知或混用 forge 在 workflow 驗證時拒絕。submit、checks、merge 與重啟對帳都使用相同 forge。GitHub 暫時受阻時保留任務與 binding，供 operator Retry。

GitHub checks 沿用 D29 的 command 關卡及 `{pr}`／`{head}`／`{branch}` 展開。checks 前確認已推送 head，command 前後再讀 PR head；遠端 head 改變就受阻，不套用過期 checks。main 只在乾淨 checkout 且可 fast-forward 時同步，不重設使用者提交；main 前進則沿用 D14 rebase、重新 push 同一 PR、重跑 checks。

## 傳輸與持久化

daemon 使用自己的 gh 登入連 github.com，排除 agent gh shim。token 只留環境或私人設定，不放命令列；repo 由明確 origin 解析。每次 API 呼叫限 60 秒及 runner 的 5 MiB 輸出上限，傳輸層不重送 mutation。

schema 0017 保存固定 task／本機 repo／GitHub repo ID／branch／nonce、PR number 與 revision。CAS 拒絕 stale revision、改綁 PR／repo 及兩個 task 佔用相同 repo ID／branch。

push 前保存 intent，以完整舊 SHA 的 force-with-lease 更新單一 ref；未確認 intent 先核遠端 head，不覆蓋外來更新。PR create 前保存 attempt；回覆遺失只搜尋固定 branch／base 與 task nonce，不重送 create，不認領外來 PR。

merge 前後核 repo ID 與 ownership marker。PUT 前持久保存綁定 approved SHA 的 attempt；unknown 結果即使重啟或 operator Retry 也只讀回對帳，不重送、不再 push 或刪除遠端分支。結果未知時 `find_merge` 先回受阻，不能進入 main 同步／rebase；pipeline 與遠端對帳必須保留 durable attempt 的原核准 head。PUT 帶 approved SHA；已 merged 或回覆遺失時只讀原 PR，核 merge commit 的兩個 parents 與 approved head，沒有完整收據就受阻。API 的 head CAS 見[官方端點](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request)。

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

原生測試的獨立 API 程序沿用真捕獲形狀，Git 自己產生 object／tree／parents；固定 GitHub URL 只映射到自有 bare repo。原生案例不呼叫外部 GitHub；嚴格保護另以本文下方受控真測驗證。測試結束移除自有 daemon／holder、repo 與 home。

2026-10-07 全新覆核曾以真 daemon 重現 unknown merge 重啟會重送（e55d3df，REFUTED）；已補 durable merge attempt 與真 daemon regression，等待修正後獨立重驗。成功後遺失回覆與未確認仍 open 分別測試，不互相代替。後續獨立反例又確認 ecde512 在 unknown＋main 前進時先 rebase，令原 head 的晚到收據無法恢復；已補對帳前阻擋與正式 daemon regression，原生 Forge 六案與 pipeline 五案通過；獨立重驗確認原核准 head 保持不變、晚到收據跨重啟完成、PUT 一次且 cleanup.complete=true，自有 fixture／程序已清。此結果不涵蓋 GitHub 真測、base policy 或後續 migration 整合。

## 已選政策與剩餘工作

GitHub merge API 只有 approved-head CAS，沒有 expected-base CAS。使用者已同意嚴格分支保護：forge 必須讀到 classic branch protection 的 `required_status_checks.strict=true`、非空 required contexts、`enforce_admins.enabled=true` 及允許 merge commit（required_linear_history=false），否則不送 merge。權限不足、查詢失敗、rulesets-only 或無法確認的設定都受阻；不自動修改共享 repo。分支在最後查核後前進時，由 GitHub 的 strict checks 拒絕過期分支。

正式 adapter 完成 repo／PR／head／保護查核後，才以 CAS 保存 intent 並立即呼叫 PUT。查核失敗不保存 intent，修正設定後可 Retry；保存後的傳輸結果不明仍不自動重送。已完成 merge 的收據恢復只讀，不要求當前保護仍存在。管理員同時移除保護的競爭不屬於可保證的 server enforcement；需維持此設定。嚴格檢查語義見 [GitHub protected branches](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches)。

受控 production Forge 真 GitHub submit／merge／收據恢復與 strict 拒絕已通過，daemon 流水線以原生測試覆蓋。尚需 migration 整合後的完整驗收、全新 verifier、CI、合併與施工目錄清理。本頁不宣稱第 12C 完成。

## 下一步

嚴格政策與受控真測已完成；完成整合驗收及 CI。離線重驗：先 `cargo build -p agend -p agend-testkit --bins`，再 `cargo test -p agend-daemon --test github_forge --test github_pipeline`。

2026-10-08 嚴格保護原生驗證：daemon crate 259 passed／0 failed；GitHub API 單元 19、原生 Forge 8、獨立 policy 2 與完整 contract_teeth 5 通過。honest strict producer 曾揭露原 FRG-10 的第二條 sibling 必須拒絕；改為 fixture 明列政策，local／fake 原始 ancestry 與 OverwritesBase 斷言不變，strict 額外核拒絕種類／base 與 head 不變，並加入三個反例。沒有以跳過或放寬所有錯誤來過關。全新 verifier 確認離線範圍；真 server enforcement 仍待測。

`github_live_probe` 是人工有界計畫使用的 production Forge probe：明確 HOME、repo、task、branch，分別 submit／merge／recover／cleanup。它不啟動模型、不代表真 daemon 自動流水線已完成；真測前固定 binary／runner 雜湊與自有 repo 範圍。

2026-10-08 production Forge 真測（固定 12e7553）：自有 GitHub repo 的兩條 sibling 各建立 PR／required status，第一條合併及新程序讀回收據同為 0deae23218ea457ad48f213fd0c0f124326e6234；第二條因 main 前進回 HTTP 405，main／head 不變，重開後 durable guard 禁止重送。第一條 cleanup.complete 與遠端 branch 404；最終 repo DELETE→404、本機 lab 移除。全新 verifier 核 44 份命令紀錄與雜湊 CONFIRMED，零模型；這是正式 Forge 子程序真測，daemon／holder 流水線另由原生測試覆蓋。原始證據保留於 AgEnD-ops/g12c-fresh-review-20261007/live-forge-v1。

2026-10-08 整合覆核：固定 2acd36a 的 migration／fixture 1–16 與已發布版本逐位元相同；獨立 SQLite 反例確認 linked unknown Telegram outbox／notice／update 與共用已讀資料通過 v17 後不變，外鍵與 schema 一致。production Forge 與真測版本未變。

同一版完整 accept 12 及 push／PR 雙平台 CI 都揭露既有 archive IO 回歸：終結 action 誤用背景 cleanup，吞掉本機 release 錯誤。修正為終結 action 傳回原 Refusal，背景 wake 維持延後恢復；不改 WIP 測試斷言，也不改遠端收尾待辦語義。原失敗與修正後結果分別保存；最終 CI／整合驗收以 PR #157 為準。

修正後的全新 verifier 已獨立重跑既有 archive suite 3/3 與 remote cleanup unit 1/1，確認本機拒絕／WIP 保留／稍後恢復及遠端失敗釋放容量皆通過；沒有改測試門檻。完整最終結果由 #157 的固定 head CI 與 ops `integrated-accept12-fixed.log` 記錄。
