# AgEnD v2 五種基本關卡 — 用 v1 真實 task 資料驗證

> 歸檔註：設計階段的原始紀錄，內容原樣保存（2026-09-24）。只把絕對路徑換成佔位符：`<scratchpad>` = 當時的暫存目錄、`<v1-repo>` = agend-terminal repo、`<tmp>` = 系統暫存目錄、`~` = 使用者家目錄。文中提到的腳本、log、schema dump 沒有一起歸檔。

只讀分析，未修改任何檔案、未動 daemon。所有指令與腳本見文末，可重跑。

## 0. 資料來源與規模

- `~/.agend-terminal/task_events.jsonl`（root/`default` project，13788 行，current only）
- `~/.agend-terminal/boards/<name>/task_events.jsonl`（24 個 board 目錄）
- `~/.agend-terminal/boards/Hack_agend-terminal/task_events_archive/*.jsonl`（僅此 board 有實際內容的輪替檔，2 個檔 ~12MB；其餘 board 的 `task_events_archive/` 只有空的 `MANIFEST.json`）
- **未處理**：`~/.agend-terminal/task_events_archive/`（root 的輪替檔，336MB / 124 檔）——抽樣已從 current 檔案（1601 個 distinct task）取得足夠多樣性，沒有打開這批檔案；`decisions/`（5489 筆治理決策記錄）與 `task-progress/`（9532 筆，僅 `last_progress_at` heartbeat + `source` 標籤，無敘事內容）各抽看數筆確認格式，未逐筆解析。
- `task_index.jsonl`：只到 2026-08-26 就停更，且 88.7% 是 `Hack_agend-terminal`，用它做分層會漏掉 8/26 之後新開的 board（PCALM、nulls-research 等），改用「直接掃 boards/ 目錄」。

提取後：**8347 個 distinct task_id**，跨 25 個 board（含 root 當成 `__default__`）。達終態（Done/Cancelled/Superseded）的有 **7492 個**，855 個仍在 open/claimed/blocked 等未終態（本次分析只看終態）。

## 1. 事件格式與生命週期摸清（讀 `src/task_events.rs`）

`TaskEvent` enum（`kind` tag）：`Created, Claimed, InProgress, Verified, Done, Cancelled, OperatorSettled, Superseded, Linked, Blocked, Unblocked, Reopened, Released, MovedToBacklog, MovedToReview, TaskCloseProposed, OwnerAssigned, PriorityChanged, DescriptionUpdated, TagsSet, ResultSet, MetadataSet, BranchLinked`。

`Created` 帶的欄位已經比 5 關卡模型豐富：`depends_on: Vec<TaskId>`、`parent_id: Option<TaskId>`、`routed_to`、`branch`、`bind`、`eta_secs`、`governing_decision_id`、`review_class`（`ReviewClass` type，single/dual）。`Done.source` 是個獨立 tagged enum：`PrMerged / OperatorManual / LegacyBackfill / AutoCloseOnPrMerge / ReportAutoClose`。

**關鍵發現——型別存在但真實資料從未用過**（全 8347 個 task，任何狀態，掃過一輪 `kind_str` 統計，出現次數 0）：

| kind | 用途（依 schema 註解） | 實際出現次數 |
|---|---|---|
| `Linked` | task↔PR 顯式或 sweep 發現的連結 | **0** |
| `Verified` | by_reviewer + verdict 的正式核准事件 | **0** |
| `TaskCloseProposed` | sweep dry-run 提案 close，等 operator 確認 | **0** |
| `OperatorSettled` | 精確 operator 結算 | **0** |

也就是說 v1 real usage 裡「approval」從未走過型別化的 `Verified` 事件，「submit（PR 連結）」也從未走過 `Linked`；PR 的核准/駁回全部靠 `ResultSet` 自由文字敘述（`VERIFIED at exact head …` / `REJECTED …`），`Done.source` 只出現三種：`OperatorManual`（3792，50.6%）、`ReportAutoClose`（1904，25.4%，agent 送 kind=report 對到 correlation_id 自動關閉）、`AutoCloseOnPrMerge`（68，0.9%，branch 名比對到 merge 才自動關閉）——**從未出現 `PrMerged`**（真正靠 pr_id+merge_sha 顯式連結再關閉的那條路徑，一次都没被使用過）。

同一資訊也常見兩條路並用而不走型別欄位：`review_class`（single/dual）的 typed `Created.review_class` 只用了 1854 次，但同一語義透過 `MetadataSet` 的自由 KV 包（`key="review_class"`）用了 **3306 次**——即使 v1 已經有型別化欄位，實際使用仍傾向繞去自由文字/KV。這對新設計的啟示：**新關卡的參數如果沒有被 dispatch 路徑「強制」用上，agent 還是會繞去自由文字/metadata**。

## 2. 全體統計（7492 個終態 task，非抽樣，全量掃描）

| 訊號 | 出現數 | 佔比 |
|---|---|---|
| `parent_id` 非空（是某個複合任務的子任務） | 3491 | 46.6% |
| `depends_on` 非空 | 931 | 12.4% |
| `OwnerAssigned` ≥1 次 | 636 | 8.5%（其中 ≥2 次＝真正中途改派而非派工機制本身：48 個，最多 9 次） |
| `Blocked` ≥1 次 | 283 | 3.8% |
| `Superseded`（終態本身） | 159 | 2.1% |
| `Reopened` ≥1 次 | 16 | 0.2% |
| `Released` ≥1 次 | 42 | 0.6% |
| `Cancelled`（終態本身） | 1569 | 20.9% |
| `routed_to` 非空 / `Verified` / `Linked` / `TaskCloseProposed` / `OperatorSettled` | 0 | 0% |

`parent_id` 的 fan-out：865 個相異 parent 被至少 1 個已完成 child 引用，平均 fan-out 4.04，最大 77（`t-20260715082138656316-68811-14` "Architecture-14"，7/15–7/22 一週內展開成 77 個子任務）。

## 3. 抽樣方法（可重跑）

腳本都在本 scratchpad：`extract.py`（讀事件→per-task 摘要）、`sample.py`（分層抽樣）、`render.py`（人讀格式）、`classify.py`（規則式三分類）。

```bash
cd <scratchpad>
python3 extract.py > all_tasks_full.jsonl        # 8347 行，per-task 折疊後的完整紀錄
python3 sample.py                                 # 用 seed=20260924，輸出 sampled_tasks.jsonl
python3 render.py > sampled_render.txt            # 人讀敘事版
python3 classify.py                               # 規則式三分類
```

分層方式：先依 board 分配抽樣目標（`Hack_agend-terminal` 35、`__default__` 18、`agend-terminal` 6、`p2a-stress-20260826` 6、`archfix` 5、`nulls-royale-simulator-research` 5、`suzuke_agend-terminal` 3、`agend-collab-prototype`/`Hack_agentic-git`/`agentic-git` 各 2、其餘 8 個只有 1–2 筆終態的小 board 全取），board 內部再依「特徵桶」（`has_superseded`/`has_reopened`/`has_released`/`has_depends_on`/`has_ownerassigned`/`has_blocked`/`has_parent`/`cancelled`/`plain`）輪流各取至少 1 筆，剩餘名額隨機補滿——刻意讓罕見但重要的樣式（Superseded、Reopened、跨任務依賴）不被純隨機稀釋掉。**總抽樣 93 個 task，覆蓋全部 18 個有終態歷史的 board**（另外 7 個 board 目前只有 1 個未終態任務，無法抽終態樣本）。

## 4. 分類結果（93 個樣本，規則式）

| 分類 | 數量 | 佔比 |
|---|---|---|
| 完全符合（單一 work→[review-as-work]→done/cancel，一次到底） | 71 | 76.3% |
| 需要新參數（型別夠用，但缺跨任務等待/中途改派/reclaim 等參數） | 15 | 16.1% |
| 需要新的基本型別（Superseded 實例替換 / 終態後 Reopened） | 7 | 7.5% |

（分類規則：final=Superseded 或有 Reopened → 需要新型別；有 `depends_on`、或 `OwnerAssigned`≥2、或 `Blocked`、或 `Released` → 需要新參數；其餘（含一次到底的 review、disposable 立即 cancel）→ 完全符合。「完全符合」的 review 案例是把「work(role=reviewer) + 自由文字判定」對應到使用者設計的 `approval`(approver=審核者, count=review_class single/dual, bind_head=exact-head 慣例)——這個對應**驗證了 approval 的三個參數方向是對的**，只是 v1 從未走型別化的 Verified 事件。）

## 5. 需要新參數的具體案例

**跨任務/跨 board 依賴（`depends_on`，A 做完才能開始 B）**
- `t-20260719142708627000-80319-34`（board=agentic-git，"[agentic-git #26] Freeze and implement Embedder Contract v1"）：description 明寫「BLOCKED until agend-terminal #2454 and #2453 close and all code worktrees release」——依賴**跨 repo/跨 board**，`depends_on` 欄位本身只認同 board 的 task_id，跨 repo 依賴要靠人工在 description 裡寫死問題編號，daemon 無法自動判斷是否已解除封鎖。
- `t-20260713005818175593-15764-23`（Hack_agend-terminal，Cargo vendor 依賴）：description 直接引用一個更早被拒絕（PR #2776 R1）而衍生出的「logical prerequisite」task，並在 result_texts 明文吐槽「depends_on field is immutable」——即 v1 團隊自己在真實使用中發現 `depends_on` 建立後不能再補，是個已知痛點。
- `t-20260917141903490073-87735-515`（board=`__default__`，Darwin real-libg acceptance）：依賴一條「review 被拒→派 fix task→再 review→再被拒→再派 fix→再 review→VERIFIED」的 5 段鏈，每段是一個新建 sibling task，只靠 result_texts 自由文字（"Not run: primary … is REJECTED …"）串起來，`depends_on` 只指到鏈上最近一個，不是整條鏈的正式圖。
- `t-20260720164028192787-51840-9`（Merge Train Slice 1 RED，`depends_on` 兩個 task）：R1–R5 每輪被拒都新開一個 GREEN 實作 task，跨 5 輪 CI（ubuntu/macOS/Windows/coverage 間歇失敗）反覆重跑，"退回前一個 work" 在真實資料裡是**新建 sibling task**，不是同一 task 原地重跑。

**中途改派 / reclaim（`OwnerAssigned`≥2 或 `Released`）**
- `t-20260620044223857003-84833-24`（"[simplify epic] 14 個安全機械簡化…"）：`Released` 原因原文「PR-B 工作於 auth_error 丟失需重做；operator 即將 rebuild+restart，unassign 讓 post-restart 重派」——**daemon 重啟導致進行中的 work 遺失，必須整段重派**，這不是關卡失敗退回，是 work 內部的意外中斷。

## 6. 需要新的基本型別的具體案例

**Superseded：放棄本次實例、接續一個新實例（非 fail-retry、非 cancel、非 merge）**
- `t-20260831132843855330-38095-22`（"Publish and runtime-verify supervisor real-submit chain"）：description 明寫「Replacement for t-…-18 because its create-only governing decision is archived」——**上游治理決策被作廢**，導致整個任務要換一個新實例接手，跟「work 沒做完」或「gate 沒過」都無關。
- `t-20260901141532858280-1778-37` → `t-20260901142509915051-1778-39` → `t-20260901152604988699-1778-47`：同一件事（native pump bridge Phase 1）連續 supersede 三次，每次都是「移除一個 depends_on」這種**依賴圖形本身在變動**，而不是失敗。
- `t-20260917190605974857-52322-11`（PR #365 re-review）：審查中途 PR head 又變了（"exact-head" 紀律），review 直接被 Superseded 換成對新 head 的新 review task。
- 這類「輸入（head／依賴／授權決策）在流程進行中變動，必須整個實例報廢重開」在抽樣中佔 5/93（5.4%），全量中 159/7492（2.1%），5 個primitive 沒有一個能表達這個結果——merge 是成功終態、submit/approval/command 失敗只會退回 work，都假設「重來＝在原地重跑同一個 work」。

**Reopened：終態之後被人工介入重新打開**
- `t-20260709012405047317-61315-3`（"release_worktree squash-orphan 偵測失效"，59 個事件、6 輪 branch-cleanup 收斂）：`Reopened` 兩次，原因都是 "operator update"／"status done → open"——**auto-close 判定錯了**，operator 事後稽核發現殘留問題，把已經 Done 的任務手動重開，回到 InProgress 繼續做。5 個關卡模型裡「merge」是終態，沒有「merge 完之後才發現沒做完，要退回」的路徑。

## 7. 特別關注的 6 個類別逐一對應（依 prompt 要求）

| 類別 | 觀察到的真實案例 | 對 5 關卡模型的意涵 |
|---|---|---|
| **依賴（A 完才能開始 B）** | `depends_on` 931/7492（12.4%），見 §5；`t-20260719142708627000-80319-34` 跨 repo | `work` 需要一個「等待其他 task/board 完成」的參數；跨 board 依賴目前完全靠人工描述 |
| **拆分子任務** | `parent_id` 3491/7492（46.6%），fan-out 平均 4、最大 77（`t-20260715082138656316-68811-14` "Architecture-14"，一週內展開 77 個子任務） | 5 關卡是線性 pipeline，**沒有 fan-out/fan-in 的組合子**；子任務本身各自對應 work 沒問題，但「父任務等所有/部分子任務完成才算完」這個結構完全沒有對應物——需要新型別（例如一個「平行子工作群組」關卡） |
| **平行分派** | Architecture-14 底下 `"Architecture-14 item 5 Slice 2A controlled A/B: prepare usage-limit takeover"` 同一標題、同一秒建立，**指派給兩個不同 agent**（`archfix-codex-dev` 與 `claude-aef7c0`）同時做，之後有一個「Publish … selected Luna winner」任務挑贏家 | 真正的「同一件事發兩個 agent 賽跑，選贏家」，5 關卡模型的 `work` 只有一個角色/一份指示，無法表達「N 份候選、擇優」——需要新型別 |
| **中途改派** | `OwnerAssigned`≥2 次的 48 個任務；`Released`+`OwnerAssigned` 的 `t-…-84833-24`（daemon 重啟致工作遺失重派） | `work` 需要「執行者可在完成前被換掉／收回」的參數，且要跟 pipeline 層級的失敗退回區分開（這是 work **內部**的事） |
| **人類中途介入** | `Reopened` 16/7492；`governing_decision_id`／`decisions/` 目錄（5489 筆治理決策，獨立於 task 事件之外，先授權範圍再開 task，例：`d-20260423082821478657-0` "Team code-review 稽核" 授權一批 task） | 「決策」在 v1 是一個**先於/獨立於任務**的治理層（scope 授權），跟使用者設計的「approval」（work 完成後、gate 之內）語意不同——`decisions/` 更像是 workflow **啟動前**的授權，`Reopened` 則是**終態後**的介入，兩者都在 5 關卡模型的時間軸之外 |
| **等待外部事件** | 見下方獨立小節 | 見下方 |
| **重複/定期工作** | 見下方獨立小節 | 見下方 |

### 等待外部事件（部署、CI 以外的外部系統）

- **CI（非同步、多 runner、間歇性失敗）**：`t-20260720164028192787-51840-9`（Merge Train）、`t-20260831132843855330-38095-22`（supervisor real-submit）等大量 review/merge 任務的 result_texts 反覆出現「CI run 2980…failed, macOS/Windows 還在跑」「no manual polling」「exact-head CI watch armed」——CI 不是「跑一個指令拿 exit code」，是**推 push 後由 daemon 背景 watcher（`ci-watches/` 目錄）非同步等 webhook/poll**，command 關卡「exit 0 才過」的同步假設不成立，需要一個 async/watch 模式的參數（或獨立 watch 型別）。
- **等一個罕見、非我方觸發的外部重複事件，且有 TTL**：`t-20260702161319112091-56872-10`（"[flake-watch] worktree_pool … Windows 間歇紅 — 等第二個樣本"）：2026-07-02 開，一直開到 **2026-08-28（57 天）**因為「政策規定：沒等到第二個樣本就過期關閉」而 Cancelled，過程中完全沒有 work/command/approval 事件，只是**開著等一個外部（CI 隨機間歇性失敗）再發生一次**。這種「被動長期觀察、有過期時限、期間不做事」的形態，5 關卡模型沒有對應——最接近 `command` 但不是同步執行，最接近 `approval` 但沒有人要核准。**需要新型別**（一個「observe-with-TTL」關卡）。
- **部署（release.yml build+upload）**：`t-20260815072910057887-5272-195`（"Prepare v0.12.0 release candidate"）等一系列 v0.12.x release 任務，description 明寫「Do not tag or publish without explicit operator confirmation」——實際 `git tag` push 觸發 GitHub Actions 建置 5 個目標並上傳 artifact 這件事，**完全在 task 系統之外發生**（CLAUDE.md 記載，但 task_events 裡看不到任何對應事件）。**這一項未查證**：無法用 task_events 資料證實「task 曾經正式等待部署完成的訊號」，只能證實部署動作本身不在 task 生命週期的追蹤範圍內。

### 重複/定期工作

- 找到唯一一個機制性證據：`t-1789956486036377-65111514347490296773438411767120319946-0`（title="Scheduled job s-20260921020540218049-1"，`created_by="system:schedule_job"`，tags=`["schedule-job","j-30fc058260244613910fc48ee882d5ca"]`）——一個抓 podcast RSS→摘要→發 Telegram 的排程任務，由系統在時間到時自動 `Created`，之後 `OwnerAssigned`→`InProgress`→（因 `system:auto_orphan` 介入）→`Cancelled`。task_id 格式跟一般 `t-<timestamp>-<seq>` 完全不同（是一長串數字），顯示這是另一套子系統直接寫入同一個事件流。
- **頻率未查證**：對 root 336MB 的舊 archive 全文 grep `"system:schedule_job"` 與該 job id，**0 命中**——這個「排程觸發任務」機制看起來是最近才上線（本次觀察落在 2026-09-21），無法確認它過去是否曾以其他形式重複執行過，也無法確認這是不是唯一一次。**不應腦補**成「已驗證的重複模式」，只能說：機制存在、只觀察到 1 筆實例。
- 對 5 關卡模型的意涵：不管未來出現頻率如何，這揭示的缺口是**「什麼觸發一個 workflow 實例開始」**——5 關卡目前只描述「一個實例跑起來之後」的內部關卡序列，完全沒有「手動 vs 排程觸發」這個維度，需要在關卡序列之外加一個 trigger 概念（不算「基本關卡」，但漏了會做不到定期工作）。

## 8. Workflow 樣式出現次數（全量 7492 個終態 task，非抽樣；連續重複的 kind 已去重）

| 樣式（去重後的核心 kind 序列） | 次數 |
|---|---|
| Created → Claimed → Done | 1588 |
| Created → Claimed → InProgress → Done | 1067 |
| Created → Done | 1061 |
| Created → Cancelled | 658 |
| Created → Claimed → MovedToReview → Done | 474 |
| Cancelled（單獨，無法回溯 Created，多為 root-only 孤兒清理） | 334 |
| Created → Claimed → InProgress → MovedToReview → Done | 320 |
| Created → InProgress → Done | 264 |
| Created → Claimed → Cancelled | 232 |
| Created → InProgress → MovedToReview → Done | 132 |
| Created → Superseded | 106 |
| Created → Claimed → InProgress → Cancelled | 96 |
| Created → OwnerAssigned → Claimed → Done | 64 |
| Created → InProgress → OwnerAssigned → Done | 53 |
| Created → OwnerAssigned → Claimed → InProgress → Done | 46 |
| Created → Claimed → InProgress → MovedToReview → InProgress → MovedToReview → Done（一輪來回） | 44 |
| Created → OwnerAssigned → Cancelled | 43 |
| （其餘 275 種樣式，長尾） | 合計約 1300+ |

共 **291 種相異樣式**，前 10 種佔 62.7%（4696/7492），代表主流仍是「單一 work（可能含一次 review）直接到底」，但長尾的 275 種樣式（含前述所有依賴/改派/blocked/superseded/reopened 組合）合計仍佔約 17%，且集中在最重要的少數複雜任務上（如 Architecture-14、Merge Train、#2454 closure）。

## 9. 限制與未查證事項

- 抽樣與全量統計都只覆蓋「已進入 task_events.jsonl 的事件」；`decisions/`、`task-progress/` 只做格式確認，未逐筆纳入分類依據。
- Root board（`__default__`）的 336MB 舊 archive 未展開，只用了 current 檔（1601 個 task）；如果排程任務、supersede 鏈等模式在更早期（4–8 月）有不同分布，本報告不會反映到。
- 「部署等待」「排程重複工作的長期頻率」兩項明確標記未查證，見 §7。
- 三分類（71/15/7）用的是規則式自動分類（依 depends_on/OwnerAssigned/Blocked/Released/Superseded/Reopened 旗標），不是逐筆人工精讀 93 筆全部細節；抽樣中約 30 筆（review 類）有精讀確認分類合理，其餘用規則外推，屬於「同方法自驗」等級（未做 fresh-agent 第二意見複核，因任務屬資料分析非高風險判斷）。
