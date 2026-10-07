# 第 12C：GitHub forge

> **TL;DR**
> - 依 D4／D29 接提交與合併；GitHub checks 使用 command 關卡。
> - 目前已建立 gh API 傳輸與原 PR 的 head／merge 收據核對，尚未接入正式流水線。
> - 下一步：push／PR 身分與恢復、核准 head 合併、pipeline 與原生契約，再做受控真測。

## 接線缺口

既有 workflow 的 submit 含 `forge`，但 daemon 執行時仍一律建立 LocalForge。12C 必須讓 submit、merge 與重啟對帳使用 task 固定 workflow 指定的 forge；不能只有一個未使用的 GitHub adapter。

GitHub checks 沿用 D29：`command` 的 `{pr}`／`{head}`／`{branch}` 展開與既有 head／attempt 規則。合併呼叫必須帶完整 approved head，並核 API 回傳的 merge 結果；網路回覆遺失不能直接宣稱成功。GitHub API 的 `sha` 不符回 409，見[官方端點](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request)。

## 傳輸基礎（未驗收）

以 daemon 自己的 gh 登入連 github.com，排除 agent gh shim；token 只留環境或既有私人設定，不放命令列。每次呼叫限 60 秒及既有 runner 的 5 MiB 輸出上限，不在傳輸層重送 mutation。repo 身分由明確 origin 解析，不依 ambient gh 預設 repo。

真 gh 2.102.0 的 PR GET／404 已唯讀捕獲並作 parser fixture；沒有建立或合併遠端 PR，也沒有新增真模型執行。測試包含實際 shell／子程序核對引號、換行、命令替換字元保持完整 argument，且 token 不出現在 argv。

API client 合併前先 GET 原 PR；head 不符直接回報 HeadChanged，已 merged 時只讀 merge commit 核第二個 parent。PUT 帶 approved SHA，回覆遺失後只 GET 同一 PR；未得到完整收據就保持受阻。11 個測試涵蓋真捕獲解析／負例、實際 argv 與錄製回覆重播；尚無完整假 GitHub server／正式 pipeline 或真 repo 寫入驗收。

## 尚未完成

- GitHubForge 的 submit／head／merge，以及 task／PR 身分持久對帳。
- 正式 workflow forge 選擇、遠端 main 前進與 rebase、head 變更時重新 checks／核准。
- 單次提交／回覆遺失／重啟的原生測試、完整 FRG 契約與真 repo 流水線。
- 全新 verifier、CI、合併與清理；本頁不宣稱第 12C 完成。

## 身分持久化基礎

schema 0013 保存固定 task／repo ID／branch／nonce 與 PR number，禁止 stale revision、改綁及兩個 task 佔用同一 repo ID／branch。未確認 push 必須先對帳相同 head；PR create attempt 一旦保存不能清回未嘗試。原生 SQLite 跨重開測試通過；正式 remote 操作尚未使用這個 ledger，不宣稱已完成重啟流水線。
