# 第 12C：GitHub forge

> **TL;DR**
> - 依 D4／D29 接提交與合併；GitHub checks 使用 command 關卡。
> - 正式 Forge／workflow 選擇與 checks head 核對已接入；完整原生流水線與受控 GitHub 真測尚未完成。
> - 下一步：push／PR 身分與恢復、核准 head 合併、pipeline 與原生契約，再做受控真測。

## 接線缺口

本批已讓 submit、checks、merge 與重啟對帳使用 task 固定 workflow 指定的 forge；未知或混用 forge 在 workflow 驗證時拒絕。GitHub 操作受阻保留任務與 binding，供 operator Retry，不因暫時網路錯誤直接清理 task。

GitHub checks 沿用 D29：`command` 的 `{pr}`／`{head}`／`{branch}` 展開與既有 head／attempt 規則。合併呼叫必須帶完整 approved head，並核 API 回傳的 merge 結果；網路回覆遺失不能直接宣稱成功。GitHub API 的 `sha` 不符回 409，見[官方端點](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request)。

## 傳輸基礎（未驗收）

以 daemon 自己的 gh 登入連 github.com，排除 agent gh shim；token 只留環境或既有私人設定，不放命令列。每次呼叫限 60 秒及既有 runner 的 5 MiB 輸出上限，不在傳輸層重送 mutation。repo 身分由明確 origin 解析，不依 ambient gh 預設 repo。

真 gh 2.102.0 的 PR GET／404 已唯讀捕獲並作 parser fixture；沒有建立或合併遠端 PR，也沒有新增真模型執行。測試包含實際 shell／子程序核對引號、換行、命令替換字元保持完整 argument，且 token 不出現在 argv。

API client 合併前先 GET 原 PR；head 不符直接回報 HeadChanged，已 merged 時只讀 merge commit 核第二個 parent。PUT 帶 approved SHA，回覆遺失後只 GET 同一 PR；未得到完整收據就保持受阻。11 個測試涵蓋真捕獲解析／負例、實際 argv 與錄製回覆重播；尚無完整假 GitHub server／正式 pipeline 或真 repo 寫入驗收。

## 尚未完成

- 正式 daemon 重啟與遠端清理驗證；GitHubForge 的離線原生 FRG 契約已通過。
- 已接入的遠端 main fast-forward／rebase／重新 push 和 checks，尚需完整故障驗證。
- 單次提交／回覆遺失／重啟的原生測試、完整 FRG 契約與真 repo 流水線。
- 全新 verifier、CI、合併與清理；本頁不宣稱第 12C 完成。

## 身分持久化基礎

schema 0013 保存固定 task／repo ID／branch／nonce 與 PR number，禁止 stale revision、改綁及兩個 task 佔用同一 repo ID／branch。未確認 push 必須先對帳相同 head；PR create attempt 一旦保存不能清回未嘗試。原生 SQLite 跨重開測試通過；正式 remote 操作尚未使用這個 ledger，不宣稱已完成重啟流水線。

## 提交操作基礎（未接入 pipeline）

`submit_pull` 先核 repo ID，PR 建立前持久 claim；回覆遺失只搜尋固定 branch／base 與 task nonce，unknown attempt 不重送，外來 PR 不認領。`push_owned` 核固定 origin，持久 push intent，再以完整舊 SHA 的 force-with-lease 更新單一 ref；回覆遺失核遠端 head，重啟未確認時不覆蓋新 head。

新增兩個捕獲回覆＋原生 SQLite 重開案例，以及一個真 Git／bare repo 的成功遺失回覆、競爭 writer、重啟不重送案例。測試沒有外部 GitHub mutation；正式 Forge 選擇、pipeline 與完整 FRG 仍待完成。

## 正式接線（施工中）

`SelectedForge` 依固定 workflow 選 local／github；GitHubForge 使用 durable ownership 執行 submit／head／merge，merge 前後都核 repo ID 與 PR marker，已合併時只讀原收據。checks 前重新確認已推送 head，command 前後再 GET PR head；遠端 head 變動會保留受阻狀態，不套用過期 checks。

GitHub main 只在乾淨 main checkout 且可 fast-forward 時同步；不重設使用者提交。rebase 後的 checks 會先更新同一個 PR。已加 whole-queue fake 測試證明所有階段選 github，既有真 local Forge FRG 1–10 仍通過。這不是正式 GitHub daemon 真測。

### 待決策的遠端 base 競爭

GitHub merge API 只接受 approved head，沒有 expected-base CAS。已向使用者提出：要求目標分支嚴格 up-to-date 保護（建議），或接受 main 在最後查核與 merge 間前進的空窗。尚未收到決定；相關完整 merge 認證及外部寫入驗證暫不進行，其餘接線／測試持續。

## 原生 Forge 契約（離線）

`cargo test -p agend-daemon --test github_forge` 直接呼叫正式 GithubForge，搭配原生 Git／bare repo、SQLite 及獨立程序 producer。API 形狀沿用真捕獲 fixture；commit／merge tree／parents 由 Git 產生。fixture 僅將固定 GitHub URL 映射到自有 bare repo，沒有外部網路寫入。

FRG 1–10 全通過；另驗 create／merge 回覆遺失、SQLite 重開後同一 PR／merge 收據且只有一次 mutation。本機 main 有 WIP 時拒絕同步，內容保留，排除 fixture WIP 後只讀收據完成同步，不重送 merge。

契約的不同 branch 現各帶不同 task ID，符合一個 task 固定一個 PR 的正式 namespace；所有原斷言保留。local FRG 與全部 contract mutant 反例仍通過。真 GitHub 分支保護、正式 daemon 端到端與遠端收尾仍不在這批通過範圍。
