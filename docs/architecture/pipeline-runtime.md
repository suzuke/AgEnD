# Pipeline 執行與恢復（第 10 施工關）

> **TL;DR**
> - 一條 daemon queue 推進 task：core `step` → SQLite CAS 與 event 同交易 → 副作用 → 結果回 queue。
> - checks 用新的 detached worktree 和寫入沙箱；merge 先記 intent，重啟以完整 head、兩個 parent 和 trailer 找回。
> - 下一步：跑 `cargo xtask accept pipeline`；人工驗收見 [第 10 施工關](../gates/gate-10-pipeline.md)。

## 執行順序

`pipeline` 是唯一推進者。step 拒絕的事件不存資料；CAS conflict 丟棄，不自動重跑。
`tasks.pipeline` 只存執行快照，workflow 由 task 的 id/version 固定；restore 驗證位置、attempt、關卡紀錄與 merge 門檻後才可派副作用。

| 關卡 | 副作用與結果 |
|---|---|
| work | 同 team 的 running instances 交給 core 派工；先持久化 binding，再建 `agend/<task>/<slug>` 與 `$AGEND_HOME/worktrees/<task>`，裝 hook、寫唯讀快照 |
| submit | LocalForge 讀取 branch head；本機 forge 沒有 change id |
| command | 每次以固定 head 建新 detached worktree；一次最多一個 checks 程序；結果帶原 stage/attempt/head |
| approval(role) | 排除作者、優先不同 backend；有 head 時在 detached `<task>-review` 審查並綁 head；無 head 時用 logical workspace binding |
| approval(human) | `approval:<ticket>` 顯示目標、head 前七碼及下一步；退回修改須填理由 |
| merge | rebase 前確認 worktree 乾淨；同 patch 保留核准、重跑 checks；不同 patch／conflict 回 work |

一個 agent 同時只持有一個 task；等待 checks 或人工核准仍持有。返工回原持有者，failed holder 等 retry。`planned` 的 Result 經核准後向前交接給下一個 Work 角色：釋放舊 binding、保存 WIP、CAS 清 assignee，再由 core 選新角色；已有 Branch 的向前角色交接保留原 branch／head，未提交 WIP 保存 patch，下一角色沿用原 commits；真正 `ReturnToWork` 不改派。派工的 `done`／`result` 提示依當前 Work output 選擇。當前 WorkProduct 的 summary／output／Plan items 會放入 durable 派工內容與人工 approval recap，重啟後照同一 snapshot 取得；role review 沒有 head 時用 workspace binding，不建立 detached review worktree。
角色不存在／只有作者能審查顯示 `no-role:<team>/<role>`；只有忙碌時排隊，角色加入後自動重試。
持有 task 的 instance 不可移除，先取消 task；merge 送出後不可取消。

## Domain 與 adapters

Engine 透過 core `PipelineStore`、`PipelineExecutor`、`PipelineView`、`Driver`、`Clock` 注入；跨模組 records 也在 core。SQLite、Codex、git、filesystem 與 checks 的組裝放在 `pipeline_runtime`，queue 不持有 concrete adapter。
`FakePipelineExecutor` 接 FakeStore／FakeForge／FakeRunner，完整 queue 測試另注入 FakeDriver／FakeClock，驗成功 merge、返工、stale、store failure 與單 task boot failure；真程序與 adapter 契約仍另跑。

## Checks 沙箱

| 項目 | 規則 |
|---|---|
| 工作目錄 | 每次新 detached worktree；不用 agent 的 worktree 或共用 build cache |
| 環境 | 白名單、不帶 `AGEND_*`；PATH 排除 shim；TMPDIR、CARGO_HOME、CARGO_TARGET_DIR、XDG_CACHE_HOME 各次獨立 |
| macOS | `sandbox-exec`：只有 checks worktree、該次 tmp 可寫；`.git` 不可寫；Unix sockets 只開 DNS 的 mDNSResponder |
| Linux | `bwrap`：根目錄與 canonical repo 明確唯讀（含 `/tmp` 下的 repo），worktree／tmp 可寫，`.git` 唯讀，隔離 PID、隱藏已知 runtime socket 目錄；遮蔽用的 `/tmp`／runtime tmpfs 最後設成唯讀，只有顯式掛載的 worktree／每次 tmp 可寫 |
| 失敗處理 | boot 與執行前 probe；工具缺失／失效保持 checks，顯示 `sandbox-missing:<task>`，retry 重新 probe |
| 啟動 marker | 先寫 regular file；只用 lstat 驗證，FIFO／symlink 不開啟；marker 異常再 probe，工具正常則算 checks 失敗 |
| 輸出 | `logs/checks/<task>/<stage>-<attempt>.log`，最多 10 MiB、截斷有註記；event 留兩個 stream 的最後 20 行；保留 14 天 |
| 程序 | 每次以新 process group 執行；成功、失敗與逾時都清掉群組內的子程序；同群組 watchdog 在父程序死亡後清理，涵蓋 SIGKILL；逾時回 work |

網路照常；這是寫入隔離。Linux 的任意其他檔案 socket 與 abstract socket，以及 macOS 蓄意 setsid 離開群組的程序，仍是提案列明的限制。
真實 `cargo test --offline` 與 `npm test --offline` 的小型專案在 adapter 測試內編譯、執行。

## Merge 與對帳

LocalForge 用 `merge-tree` 與 `commit-tree` 建兩個 parent 的 merge commit，訊息含精確 `Agend-Task: <task>` 行。
intent 先與 task/event CAS 存入 DB，再移 main；沒有 main checkout 時用 `update-ref` CAS；只有乾淨 canonical checkout 的 main 可 `merge --ff-only`。
其他 main checkout 或髒 canonical checkout 留在 merge，顯示 `merge-blocked:<task>`；清理後 retry。

重啟先查 main 最近 1,000 個 first-parent commit 的 trailer 與第二個 parent；再查 intent 的 head 與 ancestry。
若 head 已被外部手動合併，找最早包含它的 first-parent commit 並記錄完成，不再合併。
DB 丟失 intent 仍可從 trailer 找回；debug build 只有 `after-merge-intent`、`after-main-moved` 兩個 abort failpoint。

開機在 socket bind 前驗證 active snapshots、修復 binding、重寫投影與清理自己的孤兒；以 `outstanding_actions` 重派原 ticket。
checks 重啟用新目錄、同 attempt。work 訊息固定 `dispatch:<ticket>`；review 固定 `dispatch:<ticket>/<reviewer>`，多人審查保留同一關卡 ticket、每個 recipient 只存一列。接受結果與確認 dispatch 同 CAS 交易，副作用失敗仍保留已接受的結果。
每個 task 的 boot／派工副作用（含 merge proof 查詢）失敗獨立記成 Failed，不阻止其他 task 和 socket 啟動；terminal task 拒絕晚到結果。標成 Failed 後立即清理 binding；清理失敗保留 WIP 與 durable binding，每次 wake 重試，成功後才 CAS 清 assignee、釋放 capacity。缺少 git 時暫停 repo tasks、記 log，no-repo team 照常服務。
每日對帳只做投影、bindings 與清理，保留正在跑的 checks，不重新派所有 action。

釋放順序：快照先 unbound → 保存未提交／untracked WIP（未 merge 也保存 commits）→ 卸 hook → 刪 worktree／branch → 刪 binding。
patch 放 `archive/<task>-<unix-ms>.patch`，保留 30 天，task detail 顯示路徑。git binary patch 與 untracked 名稱清單直接串流至 daemon 建立的檔案，不經 5 MiB 診斷輸出 cap；完整檔案 sync 後原子發布，成功後才刪原 worktree。未 merge 的 commits 依 first-parent 順序保存：一般 commit 用完整 binary `format-patch`；merge commit 另保存 email metadata 與相對第一個 parent 的 binary diff，包含衝突解法，再接 HEAD 上的 WIP。archive 失敗保留 WIP／binding，每次 wake 重試。checks 孤兒只辨識 `t-<數字>-*` 與對應 `.tmp`，命名空間外 worktree／目錄保留。

## 命令與協定

client protocol 1.3 新增 team/workflow/task 操作者命令、attention note，以及 task 的 repo、關卡種類、agent、受阻理由、archive 路徑。
一般 client 至少要 1.3；重啟舊 daemon 的命令仍只要求 1.2。

```bash
agend team add web --repo /absolute/repo
agend team join web dev-1 --role dev
agend team join web review-1 --role reviewer
agend workflow list
agend workflow check workflow.toml
agend workflow apply workflow.toml
agend team set-workflow web code
agend task create --team web --role dev "implement feature"
agend task cancel t-1 --reason "scope changed"
```

workflow apply 新增版本，既有 task 不換版本；內建 workflow 不可覆寫。
本關拒絕 fanout、reassign、command `on_timeout` 和非 local forge；不自動建立 ephemeral agent。
`block/unblock` 的理由存在 task 與交易裡，不另外新增 attention；reminder 到期後先持久化固定訊息 id 再刪提醒。
請示永久保存 Question → Answer → FollowUp → Answer → Resolution；回答固定 `ask:<id>/<turn>`；ask_turns 永久保存派送 receipt，重啟只補未送回答，訊息過期後不重送歷史答案。

`delivery=inbox` 只給假 worker：沒有 backend driver，寫入 inbox 算 sent、讀取不算 confirmed；同 ticket 的結果才確認派工。
正式 push 仍走已完成的 Codex driver；Claude／OpenCode 的正式 push 在第 12 施工關。

## 下一步

```bash
cargo build -p agend -p agend-testkit --bins
cargo test -p agend --test pipeline
cargo test -p agend-daemon --test pipeline_adapters
cargo xtask accept pipeline
```
