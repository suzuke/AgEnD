# 流水線、worktree 與 shim

> **TL;DR**
> - workflow = 6 種關卡的依序組合，存在 DB、有版本；只有 daemon 會 merge。
> - 記住：**核准綁 head**；head 變了核准就失效（D14 的 patch-id 例外）。
> - 下一步：狀態機實作在 `crates/agend-core/src/pipeline/`，執行在 `crates/agend-daemon/src/pipeline.rs`。

來源：規劃 r4 §4.5–§4.6、D4、D12–D15、D18–D21。

## 6 種關卡

| 關卡 | 做什麼 | 參數 |
|---|---|---|
| `work` | 指派給 agent 做事 | 角色、指示、產出（branch 或 result） |
| `command` | 跑指令，exit 0 才通過（checks 就是它） | 指令、timeout；在 head 的臨時 detached worktree 執行 |
| `approval` | 等人或某角色的 agent 核准 | 核准者（人或角色）、人數、是否綁 head |
| `submit` | 透過 forge 提交 | forge |
| `merge` | daemon 執行 merge | 門檻：checks 通過且核准的 head = 目前 head |
| `fanout` | 拆子 task 再匯合 | 子 task 來源；匯合 `all`／`first`／`pick` |

- 所有關卡共用 `timeout` 與逾時動作（通知、改派、取消）。
- 失敗與要求修改：`command` 失敗、`approval` 被要求修改（`review changes`）時，預設退回最近的 `work`，交回原作者返工；`on_fail` 可指定其他更前面的關卡；其他關卡失敗且沒有 `on_fail` 時 task 失敗。取消（人下指令或逾時動作「取消」）是獨立的終止狀態，不算失敗。
- head 變更（新 commit、main 前進後 rebase）不會讓 task 往前：`work` 中只記錄新 head，返工不會被丟掉；`submit` 中記錄後仍要等提交完成；更後面的關卡退回最後一個 `work` 之後第一個 `command` 或綁 head 的 `approval` 重跑（D14 的保留規則照舊）。
- merge 門檻：merge 前**每個** `command` 都對目前 head 通過，**每個** `approval` 都覆蓋目前 head（不綁 head 的只要有核准）。
- `{pr}` 是 submit 回傳的 change id（如 PR 編號）；forge local 沒有，所以用到 `{pr}` 的 command 在 local forge 下會讓 task 失敗。佔位符不可加引號，展開時已逐一加單引號。
- `fanout all` 收到整組 child IDs 後前進；`first` 記錄先完成的 child 並取消其他 child；`pick` 把候選 child IDs 傳給後續 approval，核准時選一個並取消其餘 child。
- task 關係（不是關卡）：`parent`、`depends_on`（可改、可跨 team）、`superseded_by`。
- task 操作：改派、reopen（done 之後由人打開）、supersede（輸入變了，新 task 接手，不算失敗）。

## 內建 workflow（唯讀）

| 名稱 | 關卡 | 需要 repo |
|---|---|---|
| `code` | work → submit → command → approval → merge | 是 |
| `research` | work(result) → approval | 否 |
| `epic` | work(plan) → fanout → approval | 否 |

需要人工核准 merge：在 merge 前加 `approval(by = "human")`（D20）。沒有「每個 repo 選自動或人工」的設定。

## workflow 管理（D19、D21）

- 格式 TOML；DB 是真相來源；編輯時匯出、存回；每次存檔是新版本。
- 命令：`agend workflow list/show/new --from/edit/apply/check/history/rollback/delete`、`agend team set-workflow`。
- task 建立時固定 workflow 版本；修改只影響之後的新 task，不做關卡對應轉換。

存檔檢查清單：

- [ ] 關卡 id 唯一、kind 合法
- [ ] submit 前有產出 branch 的 work
- [ ] 有 submit／merge 就必須 `requires = ["repo"]`
- [ ] merge 前至少有一個 `command` check，且都在 merge 前最後一個 `work` 之後
- [ ] `command` 的佔位符沒有加引號，且有來源：`{pr}` 前面要有 submit，`{head}`／`{branch}` 前面要有產出 branch 的 work
- [ ] merge 前有 `bind_head` 的 approval，否則須明寫 `allow_unreviewed = true`
- [ ] `on_fail` 只能指向前面的關卡
- [ ] 角色存在於套用的 team

## team 與 repo（D12、D13、D15）

- 每個 instance 同時只屬於一個 team；內建不可刪的 `general`，沒指定 team 的都歸它。
- team 有 0 或 1 個 repo；`general` 沒有 repo；無 repo 的 team 沒有 worktree 與 shim。
- 跨 repo 工作用跨 team 的 `depends_on`。
- `teams/<team>/` 放 team 說明（注入給成員）、共享筆記、共享產出；成員各自的 workspace 不放進去。

## 分派（D18）

- agent 不建立 instance：`agend task create --role <角色>`，daemon 依角色範本分派或開臨時 instance。
- 角色範本：允許的 backend、模型等級、指示、人數上限（min／max）、session 策略。
- 分派輸入分開帶角色目前／最小／最大 instance 數、可用 backend 額度與每個 instance 的 task concurrency；最大 headcount 未滿且有可用額度時可回傳 `SpawnEphemeral`。
- 審查排除作者並優先不同 backend（只允許同一個 backend 時仍用它）；退回修改回原作者；超過上限就排隊。
- 等待 fanout 的父 task 不佔名額；偵測 team 內互等並通知；需要不存在的角色時轉成 ask；額度用盡改派其他允許的 backend。

## merge 與 main 前進

- head = `refs/heads/<branch>` 指向的 commit（不含未 commit 變更）；diff = `main...branch`。
- Forge local：`git merge-tree --write-tree` 產生結果，再以 CAS `update-ref`（比對 main 仍是預期 SHA）；不在使用者工作目錄 `git merge`。canonical 若 checkout 在 main，要處理工作目錄過期。
- Forge github：API merge PR，帶上核准的 head SHA。
- main 前進後（D14）：自動 rebase、重跑 checks；rebase 無衝突且 branch 自身 diff 的 `git patch-id` 不變才保留核准，否則退回 work。
- done 以 daemon 的 merge 記錄為準，不從 git 推論（v1 有 2,973 次 ancestry compare 失敗）。

## binding

| 種類 | 內容 | 建立時機 |
|---|---|---|
| 工作 | (instance, task, branch, worktree) | 指派 task |
| 審查 | detached 審查 worktree + 被審的 head | 指派 review；`approve` 自動綁這個 head |

每個 agent 同時只有一個作用中的 binding；多個待審指派排隊。

## worktree 與 branch 生命週期

1. 只有 daemon 建 worktree 與 branch；shim 擋 agent 自建（`git worktree`、`checkout -b`、`switch -c`、`branch <new>`）。
2. 先寫 DB「準備建立」，建好再標記完成；崩潰後開機接續。
3. 固定命名空間：branch `agend/<task-id>/<slug>`、worktree `worktrees/<task-id>/`。命名空間內但 DB 無記錄 = 孤兒；命名空間外一律不碰。
4. 清理由關卡事件觸發：merge 完成 → 刪 branch 與 worktree；審查 worktree 在核准或駁回時刪；取消走同一流程。
5. 已結束 task 仍有 WIP：存成 patch（diff + 未追蹤檔）放 `archive/`（有保留期限），照常刪除，並顯示在總覽。
6. 單一對帳程序（開機與每日）取代 v1 約 9 個清理機制。

## git shim（D5、D6）

- 以 v1 agentic-git 為基礎；binding 格式沿用 BindingV1（agent、task_id、branch、worktree、source_repo）+ binding 類型。
- 不用 HMAC：daemon 為每個 agent 寫唯讀快照檔。唯讀只是安全帶，同 uid 的 agent 仍能 chmod。
- Forge local 必須補 protected-ref 檢查：擋 `update-ref`、`push .`、`branch -f` 對 main 的寫入。
- 依賴的環境：agent 身分與 home 的環境變數、bypass 環境變數、父程序是否為 gh、canonical repo 判定、worktree 是否存在、真 git 的位置（排除 shim 目錄）。holder 啟動 agent 時注入。
- `kill`、`killall`、`pkill` 的防護一併保留。

## 衝突預防

daemon 記錄每個進行中 task 動到的檔案；派工時重疊就警告或排序；merge 前 rebase 並依序合併。

## 下一步

```bash
cat docs/architecture/delivery.md
```
