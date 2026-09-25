# 名詞表

> **TL;DR**
> - 人和 agent 用同一套詞：這裡每個名詞一列：中文、英文、程式識別字、一句定義、容易混淆的詞、出處。
> - 記住：**workflow 的一步叫「關卡」（stage）；13 個施工階段叫「施工關」（gate）**。單獨的「關」不再使用。
> - 下一步：新增或改名名詞時先改這一頁，再改程式與其他文件（規則見 [AGENTS.md](../AGENTS.md#寫作與程式規則)）。

## 怎麼讀

- 程式識別字取自 `v2`（第 1 施工關已由 #105 併入）；沒有對應型別或模組的寫「—」。crate 名稱寫成 crate `agend-x`。
- 出處：D 編號見 [DECISIONS.md](DECISIONS.md)。「規劃 §x」指 [research/REWRITE-PLAN.md](research/REWRITE-PLAN.md)。
- 兩份文件用法不同時，以最新的決策為準（AGENTS「決策在哪」）。
- 加粗的混淆詞是已經撞過的詞，寫文件時一定要分清楚。

## 組織

| 名詞（中文） | English | 程式識別字 | 定義 | 不要跟…混淆 | 出處 |
|---|---|---|---|---|---|
| team | team | `Task::team_id`、`model::DEFAULT_TEAM` | 一組 instance 加上共享目錄 `teams/<team>/`，有 0 或 1 個 repo；內建不可刪的 `general`。 | Telegram 的 team topic；fleet（TUI 最上層，所有 team） | D12、D13、D15 |
| 角色 | role | `Stage::Work { role }`、`AssignmentRequest::role` | team 裡的職位名；work 關卡與 `agend task create --role` 用它，daemon 再依角色範本挑人。 | 角色範本；approval 的 `by`（核准者可以是人或角色） | D18 |
| 角色範本 | role template | `policy::assign::RoleCapacity`（只有人數部分） | team 對一個角色的設定：允許的 backend、模型等級、指示、人數上限（min／max）、session 策略。 | instance（範本不是成員） | D18 |
| instance | instance | `Candidate::instance_id`、`client::InstanceData` | DB 裡的一個成員：只屬於一個 team、有 backend 與 `lifetime`，各有一個 holder 與常駐 workspace。 | agent（instance 裡執行的程序） | D8、D12、[ARCHITECTURE](ARCHITECTURE.md#程序模型) |
| agent | agent | `client::AgentCommand`（agent 命令） | instance 裡執行的 backend 程序（codex／claude／opencode），PATH 上有 shim 與 `agend`。 | instance；操作者（人，用操作者命令） | [ARCHITECTURE](ARCHITECTURE.md#程序模型)、D17 |
| backend | backend | `model::Backend` | agent 使用的產品：`claude`、`codex`、`opencode`（v2.0 只有這三個）。 | driver（daemon 裡對接 backend 的 adapter） | [BACKEND-BEHAVIORS](BACKEND-BEHAVIORS.md) |
| 常駐／臨時 | persistent／ephemeral | `model::Lifetime::{Persistent, Ephemeral}` | instance 的 `lifetime`：常駐的由 daemon 啟動時拉起；臨時的隨 task／team 結束清理。 | agent 的暫存目錄（也會被清掉，但不是 instance） | D8、[tui-and-setup](architecture/tui-and-setup.md#設定與目錄d8) |
| task 持有者 | task holder | `Candidate::held_task`、`Purpose::Rework { task_holder }` | 從派工到 done／取消都持有某個 task 的 agent；一個 agent 同時只持有一個 task，返工回到它。 | **holder**（只指每個 instance 的程序）：一律寫「task 持有者」／task holder，不單寫 holder | D33 |

## 流水線

| 名詞（中文） | English | 程式識別字 | 定義 | 不要跟…混淆 | 出處 |
|---|---|---|---|---|---|
| workflow | workflow | `pipeline::workflow::Workflow` | 關卡的依序組合；TOML 定義、存 DB、每次存檔一個新版本；內建 `code`、`research`、`epic`、`planned` 唯讀。 | 流水線（daemon 執行 workflow 的機制，模組 `pipeline`） | D19、D21 |
| 關卡 | stage | `pipeline::stage::StageKind`、`workflow::Stage` | workflow 裡的一步，共 6 種：work、command、approval、submit、merge、fanout；都有 `timeout` 與逾時動作。 | **施工關**（gate）；merge 門檻；hard gate | [pipeline](architecture/pipeline.md#6-種關卡)、D11 |
| planned | planned workflow | `Workflow::builtin_planned` | 內建 workflow：work(計畫) → 人工核准計畫（不綁 head）→ work(branch) → submit → command → reviewer 核准（綁 head）→ merge；人只審計畫，實作交給 reviewer agent 與 checks。 | epic（拆子 task，不是先審計畫） | D34、[pipeline](architecture/pipeline.md#內建-workflow唯讀) |
| 已驗證 workflow | validated workflow | `workflow::ValidatedWorkflow`、`Workflow::validated` | 通過存檔檢查（D19）的 workflow；流水線狀態只能從它建立，所以不合規則的 workflow 跑不起來。 | workflow 版本（存檔次數，不是驗證） | D19、[pipeline](architecture/pipeline.md#workflow-管理d19d21) |
| 可完成證明 | completability witness | `pipeline::state::completability_witness`、`WorkflowError::NotCompletable` | 存檔檢查的最後一步：用純函式 `step` 實際走固定的成功序列、每個 command／approval 各返工一次、產出 branch 之後的每個關卡各收到一次新 commit（在 merge 關卡是送出中收到、接著 merge 失敗），每一趟都要走到 done，而且 done 時每個 command 與綁 head 的 approval 都涵蓋最後的 head、最後一個 pick fanout 的勝出者是它目前的子 task；否則拒絕存檔並指出卡住的關卡。 | 探索器（測試用、隨機序列；可完成證明是存檔規則、固定序列） | [pipeline](architecture/pipeline.md#workflow-管理d19d21) |
| 事件身分 | event identity | `PipelineEvent` 的 `stage_id`／`attempt`／`head`、`TransitionError::StaleResult`、`protocol::client::ResultIdentity` | 每個結果事件帶著要求它的 action 的身分：關卡 id、attempt，綁 head 的關卡另帶 head。`step` 只接受目前關卡、目前 attempt（與目前 head）的結果，其他一律 `StaleResult`、狀態不變。agent 回報的結果（`done`、`result`、`review approve`／`changes`）在 client protocol 帶 `identity`；沒帶的一律當成過期（`stale_result`）。 | head（commit）；task id | [pipeline](architecture/pipeline.md#6-種關卡) |
| attempt | attempt | `PipelineState::attempt`、action 與事件的 `attempt` 欄位 | task 進入某個關卡的次數（第一次是 1；返工、重跑 checks、fanout 重跑都加 1），以及關卡內作廢時的新一輪：head 變更清掉 approval 關卡已收的核准或 fanout 這一輪、逾時改派，都開新的 attempt 並重發要求。結果必須帶目前的 attempt。fanout 的 attempt 就是它這一輪的 run id。 | 重試次數；patch-id | [pipeline](architecture/pipeline.md#6-種關卡) |
| task | task | `pipeline::task::Task` | 一件工作：屬於一個 team、固定建立時的 workflow 版本；有 `parent`／`depends_on`／`superseded_by` 關係。 | 關卡（task 走過的步驟） | [pipeline](architecture/pipeline.md#6-種關卡)、D21 |
| epic | epic | `Workflow::builtin_epic` | 內建 workflow：work(plan) → fanout → approval，不需要 repo。 | fanout（epic 裡的一個關卡） | [pipeline](architecture/pipeline.md#內建-workflow唯讀) |
| fanout | fanout | `Stage::Fanout`、`FanoutJoin` | 拆出子 task 再匯合的關卡；子 task 來自 work 產出或明列，匯合方式 `all`／`first`／`pick`。 | `depends_on`（task 關係，不是關卡） | [pipeline](architecture/pipeline.md#6-種關卡) |
| work | work | `Stage::Work`、`WorkOutput` | 指派給 agent 做事的關卡；參數：角色、指示、產出（branch、result 或 plan）。 | worktree；返工（回到 work 關卡的動作） | [pipeline](architecture/pipeline.md#6-種關卡) |
| command | command | `Stage::Command` | 跑指令、exit 0 才通過的關卡，在 head 的臨時 detached worktree 執行。 | CLI 命令（agent 命令、操作者命令） | [pipeline](architecture/pipeline.md#6-種關卡)、D4 |
| change id／`{pr}` | change id／`{pr}` placeholder | `PipelineEvent::Submitted { change_id }`、`PipelineState::change_id`、`CommandContext::pr` | submit 關卡從 forge 拿回的變更編號（例如 PR 編號）；`command` 的 `{pr}` 佔位符展開成它（加單引號）。forge local 沒有 change id，所以用到 `{pr}` 的 command 在 local 下會讓 task 失敗。 | head（commit）；task id | D29、[pipeline](architecture/pipeline.md#6-種關卡) |
| checks | checks | `PassedCheck`、`GateFact::Check` | `command` 關卡的結果；merge 門檻要求 checks 在目前 head 上通過。 | 舊規劃的 Checks 介面（已取消，不是 trait） | D4、[DECISIONS 來源衝突](DECISIONS.md#來源衝突與處理) |
| approval | approval | `Stage::Approval`、`Approver`、`ApprovalRecord` | 等人或某角色的 agent 核准的關卡；參數：核准者、人數、是否綁 head（`bind_head`）。人工核准 merge = `approval(by = "human")`。 | backend 的授權提示（permission／approval 請求） | [pipeline](architecture/pipeline.md#6-種關卡)、D20 |
| 要求修改 | changes requested | `PipelineEvent::ChangesRequested`、`AgentCommand::ReviewChanges` | approval 關卡的審查者要求修改：task 退回最近的 work 關卡（或 `on_fail` 指定的 work），原因交給 task 持有者返工，不算 task 失敗。 | `StageFailed`（關卡本身出錯）；核准 | D18、D33、[pipeline](architecture/pipeline.md#6-種關卡) |
| submit | submit | `Stage::Submit` | 透過 forge 提交變更的關卡。 | merge | [pipeline](architecture/pipeline.md#6-種關卡)、D4 |
| merge | merge | `Stage::Merge`、`PipelineAction::Merge` | daemon 執行 merge 的關卡；只有 daemon 會 merge，而且要先過 merge 門檻。 | `git merge`（local forge 用 merge-tree + CAS `update-ref`，不在使用者目錄 merge） | [pipeline](architecture/pipeline.md#merge-與-main-前進) |
| merge 門檻 | merge gate | `policy::merge_gate`（`evaluate`、`MergeBlocker`） | merge 的條件：merge 前每個 `command` 關卡都在目前 head 上通過，每個 approval 關卡都有核准覆蓋目前 head（不綁 head 的只要有核准；patch-id 例外見 D14）。 | **施工關**（gate）；hard gate | [pipeline](architecture/pipeline.md#merge-與-main-前進)、D14 |
| merge 送出中 | merge in flight | `PipelineState::merge_in_flight`、`pending_head_changes`、`TransitionError::MergeInFlight`、`PipelineEvent::MergeFailed` | merge 關卡已發出 `Merge` action、還沒收到 forge 結果的狀態（daemon 契約）：只接受這次 merge 的 `MergeCompleted`／`MergeFailed` 與新 commit、main 前進（記成待處理，task 不離開 merge）；其他事件（取消、關卡失敗、逾時）一律回 `MergeInFlight`。`MergeFailed` 才套用待處理的變更（D14）或重跑 checks；收到回到已送出 head 的新 commit 代表 branch 被重設，待處理的變更丟棄。 | 取消；merge 門檻 | [pipeline](architecture/pipeline.md#6-種關卡) |
| head | head | `PipelineState::current_head` | `refs/heads/<branch>` 指向的 commit（不含未 commit 的變更）；核准與 checks 都綁它。 | git 的 `HEAD`（目前 checkout） | [pipeline](architecture/pipeline.md#merge-與-main-前進) |
| patch-id | patch-id | `PipelineState::patch_id`、`merge_gate::approval_after_rebase` | branch 自身 diff 的 `git patch-id`；main 前進後乾淨 rebase 且它不變就保留核准，否則退回 work。 | head SHA（rebase 後一定會變） | D14 |
| binding（工作／審查） | binding (work／review) | — | agent 目前唯一作用中的指派：工作 = (instance, task, branch, worktree)；審查 = detached 審查 worktree + 被審的 head。 | binding 快照（daemon 寫給 shim 的唯讀檔）；v1 的 HMAC binding | [pipeline](architecture/pipeline.md#binding)、D6 |
| 返工 | rework | `PipelineAction::ReturnToWork`、`Purpose::Rework` | 被要求修改、checks 失敗（`on_fail` 指向 work）或 patch-id 變了之後回到 work 關卡，由 task 持有者繼續。 | reopen（done 之後才發生） | D14、D18、D19（`on_fail`）、D33 |
| 改派 | reassign | `TaskOperation::Reassign`、`AssignmentDecision::Reassigned` | 同一個 task 換另一個 instance 接手（逾時動作，或 task 持有者額度用盡、被刪除）；新的 task 持有者拿到交接（`Handoff`：branch 與審查意見）。 | supersede（換成新 task） | [pipeline](architecture/pipeline.md#分派d18)、D33 |
| supersede | supersede | `TaskOperation::Supersede`、`TaskStatus::Superseded` | 輸入變了，由新 task 接手舊 task；不算失敗。 | 取消；改派 | [pipeline](architecture/pipeline.md#6-種關卡) |
| reopen | reopen | `TaskOperation::Reopen` | task done 之後由人重新打開。 | 返工 | [pipeline](architecture/pipeline.md#6-種關卡) |
| 取消 | cancel | `TimeoutAction::Cancel`、`PipelineEvent::Cancel`、`PipelineStatus::Cancelled` | 終止 task：操作者下指令或關卡逾時動作「取消」；是獨立的終止狀態，不算失敗；branch／worktree 清理與 merge 完成走同一流程。merge 送出後不能取消（見 merge 送出中）。 | supersede；fanout `pick` 取消落選的子 task | [pipeline](architecture/pipeline.md#worktree-與-branch-生命週期) |

## 送達

| 名詞（中文） | English | 程式識別字 | 定義 | 不要跟…混淆 | 出處 |
|---|---|---|---|---|---|
| 訊息 | message | `traits::AgentMessage`、`client::InboxMessage` | 送給 agent 的內容：一律完整內容、走 backend 的結構化 API；每則有 id，以 id 冪等。 | PTY 控制鍵（holder 只送單一按鍵）；Telegram 通知 | [delivery](architecture/delivery.md#送達模型) |
| 送達狀態 | delivery state | `model::DeliveryState` | 訊息狀態 `queued → sent → confirmed／failed`；確認不了就標未確認，不假裝成功。 | 忙碌等級的「排隊」（`queued` 是送達狀態） | [delivery](architecture/delivery.md#送達模型) |
| 忙碌等級：排隊／插入／中斷 | busy level: queue／steer／interrupt | `policy::busy::BusyLevel`、`effective_level` | agent 忙碌時的三種送法：turn 結束後送、插入不中斷、中斷後立即處理；只有 codex 能插入，其他改用中斷。 | 去抖動（判斷 busy／idle 何時生效） | [delivery](architecture/delivery.md#忙碌策略三級)、D16 |
| Stop hook decision | Stop hook decision | — | claude Stop hook 的輸出 `{"decision": "block", "reason": …}`：turn 結束時把排隊的訊息當成下一個 turn 送進去。 | **決策**；**請示** | D16、[delivery](architecture/delivery.md#claude-特別規則d16)、[spike-claude-f](research/spike-claude-f.md)（`reason`） |
| 來源說明 | source framing | — | 讓 claude 處理 agend channel 訊息的說明：專案 CLAUDE.md 寫明訊息來自使用者自己的團隊，訊息內可另加 from／task／request 標頭。 | 本 repo 的 AGENTS.md（給開發 AgEnD 的人和 agent） | D16、[spike-claude-f](research/spike-claude-f.md) |

## 執行環境

| 名詞（中文） | English | 程式識別字 | 定義 | 不要跟…混淆 | 出處 |
|---|---|---|---|---|---|
| daemon | daemon | crate `agend-daemon` | 常駐的唯一大型 I/O 層：protocol server、流水線、送達、監督、排程、對帳、DB。 | holder（agent 由 holder 持有，所以 daemon 可隨時重啟） | [ARCHITECTURE](ARCHITECTURE.md#程序模型)、D2 |
| holder | holder | crate `agend-holder`、`protocol::holder` | 每個 instance 一個的程序，持有 PTY、畫面與附屬程序；daemon 重啟時 agent 不斷線。 | **task 持有者**；agent runtime（管 holder 的 adapter） | D3、D11 |
| agent runtime | agent runtime | `traits::Runtime`、daemon `runtime` 模組 | daemon 啟動、停止、重新接回 holder 的 adapter（薄 `Runtime` 介面，即 holder 層）。 | **tokio runtime**（daemon 的 async runtime）：文中一律寫「agent runtime」或「tokio runtime」，不單寫 runtime | D3、[ARCHITECTURE](ARCHITECTURE.md#daemon-分層) |
| driver | driver | `traits::Driver`、`driver/{codex,claude,opencode}` | daemon 對一個 backend 的 adapter：送訊息、收狀態事件、重連。 | backend（產品本身） | D11、D16 |
| forge | forge | `traits::Forge`、`forge/{local,github}` | 提交與 merge 的 adapter：local（merge-tree + CAS `update-ref`）或 github（API）。 | GitHub CI（用 `command` 關卡接，第 1 施工關 P4） | D4、[pipeline](architecture/pipeline.md#merge-與-main-前進) |
| runner | runner | `traits::Runner`、daemon `runner` 模組 | 跑程序的 adapter；`command` 關卡與 git adapter 都經它：`run(cmd, dir, timeout)`。 | `command` 關卡本身 | [第 1 施工關 P3](gates/gate-01-core.md#p3runner-要不要-trait) |
| store | store | `traits::Store`、daemon `store` 模組 | daemon 的 SQLite 資料層，是唯一真相來源（instance、team、repo、workflow）。 | `config.toml`（人寫、daemon 只讀） | D8、[ARCHITECTURE](ARCHITECTURE.md#daemon-分層) |
| notifier | notifier | `traits::Notifier`、daemon `notifier` 模組 | 對外通知的 adapter（Telegram）；`agend telegram setup` 的配對也由它做（CLI 只請 daemon 配對）。 | 訊息送達（給 agent 的走 driver） | [README](../README.md#系統圖)、D13、[tui-and-setup](architecture/tui-and-setup.md#安裝與設定)、[第 13 施工關](gates/gate-13-install.md#範圍) |
| shim | git shim | crate `agend-shim` | 只放在 agent PATH 上的 git／kill 防護：導向 worktree、擋自建 branch／worktree 與寫 main；讀唯讀 binding 快照。 | 使用者自己的 git（shim 不改它） | D5、D6 |
| worktree | worktree | `model::worktree_dir`、`model::work_branch` | 只由 daemon 建立的 git worktree：`worktrees/<task-id>/`，branch `agend/<task-id>/<slug>`；審查另有 detached worktree。 | workspace（每個 instance 的常駐工作目錄） | [pipeline](architecture/pipeline.md#worktree-與-branch-生命週期) |
| 協定（client／holder） | protocol (client／holder) | `protocol::client`、`protocol::holder` | 兩套有版本的協定：client 協定給 TUI／CLI／GUI 連 daemon；holder 協定給 daemon 連 holder。 | backend 的協定（app-server、HTTP + SSE、hooks） | D1、D11 |
| hard gate | hard gate | `screen::HardGateKind` | 畫面上擋住 agent 的狀況：usage limit、permission／approval、rate limit、auth error、context full、啟動與更新選單。 | **施工關**（gate）；merge 門檻；approval 關卡 | [delivery](architecture/delivery.md#狀態偵測三層) |
| 螢幕分類器 | screen classifier | `screen::classify`、`SCREEN_RULES` | 直接讀 holder 畫面、只認 hard gate 的分類器；規則是資料，每條附真實畫面 fixture。 | 結構化事件（判斷 busy／idle 的來源） | [delivery](architecture/delivery.md#狀態偵測三層) |

## 介面

| 名詞（中文） | English | 程式識別字 | 定義 | 不要跟…混淆 | 出處 |
|---|---|---|---|---|---|
| 需要你 | needs-you | `DaemonEvent::AttentionRequired`、`NotificationSeverity::Attention` | 要人處理的例外（請示、門檻卡住、agent 卡住）：TUI 首頁最上方跨 team 的區塊，Telegram 有同名 topic。 | 已讀（看過不會離開「需要你」） | D13、[tui-and-setup](architecture/tui-and-setup.md#tui)、[README](../README.md#這是什麼) |
| 請示 | ask (needs-you item) | `protocol::ask::AskThread`、`AttentionRequiredData::ask` | 「需要你」裡 agent 問人的一項，由 `agend ask` 建立（v1 叫 `decision`）。是一段對話：提問可附選項，人可選選項或用自由文字回答（TUI、Telegram 或 CLI），agent 可追問，最後以結論結束（D35）。 | **決策**（D 編號）；**Stop hook decision** | D35、D17、規劃 §3.1、[gate-11](gates/gate-11-tui.md#你親自驗收)、[gate-12](gates/gate-12-adapters.md#你親自驗收) |
| 請示排序 | needs-you ordering | `policy::attention::order`、`AttentionItem` | 「需要你」的排序：解決後能讓越多 task／agent 繼續的越前面，再來等越久的越前面，最後以 id 決定。 | 已讀／已解決（狀態，不是順序） | D36 |
| 脈絡摘要 | context recap | `protocol::ask::ContextRecap`、`AttentionRequiredData::recap` | 跟著「需要你」項目的摘要：功能目標、目前為止的決定、在問什麼、之後會發生什麼；切換到該項目時顯示。core 只定型別，內容由 daemon 產生（第 11 施工關）。 | 請示本身；task 歷史 | D37 |
| ask | ask | `AgentCommand::Ask` | agent 命令 `agend ask`：建立一個請示；分派時需要的角色不存在也會轉成 ask。 | opencode 權限設定的 `"ask"` | D17、D18 |
| attention-first | attention-first | crate `agend-tui` | TUI 的原則：先看「需要你」，再看各 team。 | — | [tui-and-setup](architecture/tui-and-setup.md#tui) |
| 已讀／已解決 | read／resolved | — | 「需要你」的兩種狀態：看過只去掉粗體（已讀）；選了動作才解除（已解決）。 | — | [tui-and-setup](architecture/tui-and-setup.md#tui) |

## 施工

| 名詞（中文） | English | 程式識別字 | 定義 | 不要跟…混淆 | 出處 |
|---|---|---|---|---|---|
| 施工關 | gate (build gate) | `xtask::accept::Gate`、`GATES` | 依 crate 由下往上的 13 個施工階段之一；各自驗收，使用者確認後才開下一個（`cargo xtask accept <施工關>`）。 | **關卡**（workflow 的 stage）；merge 門檻；hard gate | D22、D24、[ROADMAP](ROADMAP.md) |
| 你親自驗收 | owner acceptance | — | 施工關頁面裡由使用者本人照做的步驟（指令、應該看到、打勾），至少一步故意弄壞。 | 自動驗收（`cargo xtask accept`、CI、verifier） | [gates/README](gates/README.md#範本) |
| 驗收紀錄 | acceptance record | — | 使用者在施工關頁面填的表（日期、結果、備註）；填好該施工關才算完成。 | 進度紀錄 | [gates/README](gates/README.md#範本) |
| 進度紀錄 | progress log | — | 每完成一件事加一行（日期 + 一行 + commit／PR，新的在上面）；ROADMAP 與每個施工關頁面各一份。 | 驗收紀錄 | [AGENTS](../AGENTS.md#目前狀態)、[ROADMAP](ROADMAP.md#進度紀錄) |
| 開工前提案 | pre-work proposal | — | 施工關開工前要使用者逐條確認的設計問題（例如第 1 施工關的 P1–P7）。 | 決策（確認後才會編成 D 編號） | [gates/README](gates/README.md#範本) |
| spike | spike | — | 開工前的實測（第 0 階段）；結論在 BACKEND-BEHAVIORS，原始紀錄在 research/。 | 施工關 | [ROADMAP](ROADMAP.md#第-0-階段spike)、[research](research/README.md) |
| 探索器 | explorer | `tests/pipeline_explorer.rs`、`tests/pipeline_explorer_splitmix.rs` | 不加依賴的 property test：固定種子產生大量事件序列跑狀態機，每一步用只看被接受事件的 oracle 檢查 merge 門檻、不跳過關卡、返工不遺失等不變量。 | 單元測試；竄改狀態測試 | [agend-core TESTING](../crates/agend-core/TESTING.md#狀態機探索器) |
| 竄改狀態 | tampered state | `pipeline::state::tests::tampered_states_never_panic_and_never_merge_past_the_records` | 刻意改壞欄位（任意關卡位置、status、head、紀錄）的流水線狀態；只有 crate 內的測試造得出來，用來證明 `step` 不 panic、紀錄不足時不會 merge。 | 已驗證 workflow（外部只能從它建立狀態） | [agend-core TESTING](../crates/agend-core/TESTING.md#狀態機探索器) |
| 假實作 | fake | crate `agend-testkit` 的 `fakes::Fake*`（`FakeDriver`、`FakeForge`…） | 一個 core trait 的測試用實作：可預測（計數器產生 id）、可檢查（`calls()`）、可編排（`fail_next`）；必須通過該 trait 的契約測試。 | mock（只驗呼叫、不守契約）；假 agent（是程式，不是 trait 實作） | D9、[agend-testkit](../crates/agend-testkit/README.md) |
| 契約測試 | contract suite | `contract::<trait>::run`、`contract::Report` | 一個 trait 的一組具名規則，對假實作與真實作跑同一份，讓假實作不會漂移；報表一行 `contract Forge: fake 10/10 pass`；規則編號見 [CONTRACTS.md](../crates/agend-testkit/CONTRACTS.md)。 | 單元測試；protocol golden 測試 | D9、v1 #1483、[agend-testkit](../crates/agend-testkit/README.md#契約測試怎麼接真實作) |
| 契約 fixture | contract fixture | `contract::<trait>::<Trait>Fixture`（例如 `ForgeFixture`） | 契約測試除了 trait 之外需要的操作（例如「在 branch 上 commit」）；每個實作各寫一個。 | 螢幕 fixture（畫面擷取） | [agend-testkit](../crates/agend-testkit/README.md#契約測試怎麼接真實作) |
| 契約規則／mutant | contract rule／mutant | `Case::rule`（`DRV-1`…`RUN-9`）、`tests/contract_teeth/` | 契約測試裡一條有編號的規則，全部列在 [CONTRACTS.md](../crates/agend-testkit/CONTRACTS.md)；mutant 是故意違反某條規則的實作，契約 suite 必須在標著那條規則的 case 上失敗。 | 決策編號（D1…）；mutation testing 工具 | D9、[agend-testkit](../crates/agend-testkit/CONTRACTS.md) |
| 假 daemon | fake daemon | `fake_daemon::FakeDaemon`、`ProbeClient` | 測試行程內的 client protocol v1 server，給 client／CLI／TUI 測試用；會執行事件身分規則。 | 真 daemon 的 `server` 模組 | [agend-testkit](../crates/agend-testkit/README.md#假-daemon) |
| 假 agent | fake agent | `fake-codex-app-server`、`fake-opencode-serve`、`fake-claude`（`fake_agent::*`） | 只講真 backend 協定一小部分的程式，回覆固定、stdin 結束就正常結束；涵蓋範圍寫在各模組開頭。 | 假實作（trait 層）；真 backend 的 smoke test | [agend-testkit](../crates/agend-testkit/README.md#假-agent-程式) |
| 錄製器 | backend recorder | `recorder::Backend`、`recorder::BACKENDS`、bin `agend-record`、`cargo xtask record` | 在寫入沙箱裡用假 agent 模擬的同一條傳輸驅動真 backend CLI 跑固定情境，把兩個方向的每則訊息遮蔽後存成錄製檔。 | 螢幕 fixture（畫面擷取）；v1 的 smoke test | [RECORDER](../crates/agend-testkit/RECORDER.md) |
| 錄製檔 | transcript | `recorder::read_transcript`、`crates/agend-testkit/transcripts/<backend>/<scenario>.jsonl` | 一個情境的錄製結果：header（CLI 版本、日期）加上依序的訊息。 | 真 CLI 自己的 transcript（claude 的 `~/.claude/projects/`） | [RECORDER](../crates/agend-testkit/RECORDER.md) |
| 一致性檢查 | conformance check | `tests/conformance.rs`、`recorder::shape::compare` | 用同一套情境驅動假 agent，和錄製檔按形狀（訊息種類、欄位、型別、順序）比對；規則只在 `recorder::shape`。 | 契約測試（trait 層，比規則不比形狀） | [RECORDER](../crates/agend-testkit/RECORDER.md#一致性檢查的比對規則recordershape唯一出處) |
| 決策 | decision (D*n*) | — | 經使用者確認的設計決定，有 D 編號；沒有新證據就不重開。 | **請示**；**Stop hook decision** | [DECISIONS](DECISIONS.md) |

## 細節：為什麼是「關卡」與「施工關」

- 衝突：「關卡」原本同時指 workflow 的 6 種步驟和 13 個施工階段（「第 1 關」「每關」「關卡頁」），讀的人要看上下文猜。
- 決定：workflow 的一步叫「關卡」（stage）。程式已經用 `stage`／`StageKind`，而且 workflow 的 TOML 與存檔檢查（D19）都叫它關卡，改名的成本最大。
- 施工階段改叫「施工關」（gate）：保留「第 N 關」的讀法，加上「施工」就不會和關卡混淆；英文沿用 xtask 的 `gate`。檔名 `docs/gates/gate-NN-*.md` 不變。
- 英文單寫 gate 只指施工關；merge gate（merge 門檻）與 hard gate 一律帶前綴。

## 下一步

```bash
grep -rnE "第 [0-9–、]+ 關|[每這本該]關|<關>" README.md AGENTS.md docs --exclude-dir=research --exclude=GLOSSARY.md   # 應該沒有輸出：施工階段要寫「施工關」
```
