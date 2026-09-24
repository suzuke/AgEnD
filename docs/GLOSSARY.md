# 名詞表

> **TL;DR**
> - 人和 agent 用同一套詞：這裡每個名詞一列：中文、英文、程式識別字、一句定義、容易混淆的詞、出處。
> - 記住：**workflow 的一步叫「關卡」（stage）；13 個施工階段叫「施工關」（gate）**。單獨的「關」不再使用。
> - 下一步：新增或改名名詞時先改這一頁，再改程式與其他文件（規則見 [AGENTS.md](../AGENTS.md#寫作與程式規則)）。

## 怎麼讀

- 程式識別字取自 `feat/gate-01-core`（第 1 施工關的 branch，尚未併入 `v2`）；沒有對應型別或模組的寫「—」。crate 名稱寫成 crate `agend-x`。
- 出處：D 編號見 [DECISIONS.md](DECISIONS.md)；D26–D33 目前只在第 1 施工關的 branch 上。「規劃 §x」指 [research/REWRITE-PLAN.md](research/REWRITE-PLAN.md)。
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
| task 持有者 | task holder | `Candidate::held_task`、`Purpose::Rework { task_holder }`（第 1 施工關 branch 改名中） | 從派工到 done／取消都持有某個 task 的 agent；一個 agent 同時只持有一個 task，返工回到它。 | **holder**（只指每個 instance 的程序）：一律寫「task 持有者」／task holder，不單寫 holder | D33 |

## 流水線

| 名詞（中文） | English | 程式識別字 | 定義 | 不要跟…混淆 | 出處 |
|---|---|---|---|---|---|
| workflow | workflow | `pipeline::workflow::Workflow` | 關卡的依序組合；TOML 定義、存 DB、每次存檔一個新版本；內建 `code`、`research`、`epic` 唯讀。 | 流水線（daemon 執行 workflow 的機制，模組 `pipeline`） | D19、D21 |
| 關卡 | stage | `pipeline::stage::StageKind`、`workflow::Stage` | workflow 裡的一步，共 6 種：work、command、approval、submit、merge、fanout；都有 `timeout` 與逾時動作。 | **施工關**（gate）；merge 門檻；hard gate | [pipeline](architecture/pipeline.md#6-種關卡)、D11 |
| task | task | `pipeline::task::Task` | 一件工作：屬於一個 team、固定建立時的 workflow 版本；有 `parent`／`depends_on`／`superseded_by` 關係。 | 關卡（task 走過的步驟） | [pipeline](architecture/pipeline.md#6-種關卡)、D21 |
| epic | epic | `Workflow::builtin_epic` | 內建 workflow：work(plan) → fanout → approval，不需要 repo。 | fanout（epic 裡的一個關卡） | [pipeline](architecture/pipeline.md#內建-workflow唯讀) |
| fanout | fanout | `Stage::Fanout`、`FanoutJoin` | 拆出子 task 再匯合的關卡；子 task 來自 work 產出或明列，匯合方式 `all`／`first`／`pick`。 | `depends_on`（task 關係，不是關卡） | [pipeline](architecture/pipeline.md#6-種關卡) |
| work | work | `Stage::Work`、`WorkOutput` | 指派給 agent 做事的關卡；參數：角色、指示、產出（branch、result 或 plan）。 | worktree；返工（回到 work 關卡的動作） | [pipeline](architecture/pipeline.md#6-種關卡) |
| command | command | `Stage::Command` | 跑指令、exit 0 才通過的關卡，在 head 的臨時 detached worktree 執行。 | CLI 命令（agent 命令、操作者命令） | [pipeline](architecture/pipeline.md#6-種關卡)、D4 |
| checks | checks | `PassedCheck`、`GateFact::Check` | `command` 關卡的結果；merge 門檻要求 checks 在目前 head 上通過。 | 舊規劃的 Checks 介面（已取消，不是 trait） | D4、[DECISIONS 來源衝突](DECISIONS.md#來源衝突與處理) |
| approval | approval | `Stage::Approval`、`Approver`、`ApprovalRecord` | 等人或某角色的 agent 核准的關卡；參數：核准者、人數、是否綁 head（`bind_head`）。人工核准 merge = `approval(by = "human")`。 | backend 的授權提示（permission／approval 請求） | [pipeline](architecture/pipeline.md#6-種關卡)、D20 |
| submit | submit | `Stage::Submit` | 透過 forge 提交變更的關卡。 | merge | [pipeline](architecture/pipeline.md#6-種關卡)、D4 |
| merge | merge | `Stage::Merge`、`PipelineAction::Merge` | daemon 執行 merge 的關卡；只有 daemon 會 merge，而且要先過 merge 門檻。 | `git merge`（local forge 用 merge-tree + CAS `update-ref`，不在使用者目錄 merge） | [pipeline](architecture/pipeline.md#merge-與-main-前進) |
| merge 門檻 | merge gate | `policy::merge_gate`（`evaluate`、`MergeBlocker`） | merge 的條件：checks 通過且核准的 head = 目前 head（patch-id 例外見 D14）。 | **施工關**（gate）；hard gate | [pipeline](architecture/pipeline.md#merge-與-main-前進)、D14 |
| head | head | `PipelineState::current_head` | `refs/heads/<branch>` 指向的 commit（不含未 commit 的變更）；核准與 checks 都綁它。 | git 的 `HEAD`（目前 checkout） | [pipeline](architecture/pipeline.md#merge-與-main-前進) |
| patch-id | patch-id | `PipelineState::patch_id`、`merge_gate::approval_after_rebase` | branch 自身 diff 的 `git patch-id`；main 前進後乾淨 rebase 且它不變就保留核准，否則退回 work。 | head SHA（rebase 後一定會變） | D14 |
| binding（工作／審查） | binding (work／review) | — | agent 目前唯一作用中的指派：工作 = (instance, task, branch, worktree)；審查 = detached 審查 worktree + 被審的 head。 | binding 快照（daemon 寫給 shim 的唯讀檔）；v1 的 HMAC binding | [pipeline](architecture/pipeline.md#binding)、D6 |
| 返工 | rework | `PipelineAction::ReturnToWork`、`Purpose::Rework` | 被要求修改、checks 失敗或 patch-id 變了之後回到 work 關卡，由 task 持有者繼續。 | reopen（done 之後才發生） | D14、D18、D33 |
| 改派 | reassign | `TaskOperation::Reassign`、`AssignmentDecision::Reassigned` | 同一個 task 換另一個 instance 接手（逾時動作，或持有者額度用盡、被刪除）。 | supersede（換成新 task） | [pipeline](architecture/pipeline.md#分派d18)、D33 |
| supersede | supersede | `TaskOperation::Supersede`、`TaskStatus::Superseded` | 輸入變了，由新 task 接手舊 task；不算失敗。 | 取消；改派 | [pipeline](architecture/pipeline.md#6-種關卡) |
| reopen | reopen | `TaskOperation::Reopen` | task done 之後由人重新打開。 | 返工 | [pipeline](architecture/pipeline.md#6-種關卡) |
| 取消 | cancel | `PipelineEvent::Cancel`、`TimeoutAction::Cancel` | 由操作者或逾時動作終止 task；branch／worktree 清理與 merge 完成走同一流程。 | supersede；fanout `pick` 取消落選的子 task | [pipeline](architecture/pipeline.md#worktree-與-branch-生命週期) |

## 送達

| 名詞（中文） | English | 程式識別字 | 定義 | 不要跟…混淆 | 出處 |
|---|---|---|---|---|---|
| 訊息 | message | `traits::AgentMessage`、`client::InboxMessage` | 送給 agent 的內容：一律完整內容、走 backend 的結構化 API；每則有 id，以 id 冪等。 | PTY 控制鍵（holder 只送單一按鍵）；Telegram 通知 | [delivery](architecture/delivery.md#送達模型) |
| 送達狀態 | delivery state | `model::DeliveryState` | 訊息狀態 `queued → sent → confirmed／failed`；確認不了就標未確認，不假裝成功。 | 忙碌等級的「排隊」（`queued` 是送達狀態） | [delivery](architecture/delivery.md#送達模型) |
| 忙碌等級：排隊／插入／中斷 | busy level: queue／steer／interrupt | `policy::busy::BusyLevel`、`effective_level` | agent 忙碌時的三種送法：turn 結束後送、插入不中斷、中斷後立即處理；只有 codex 能插入，其他改用中斷。 | 去抖動（判斷 busy／idle 何時生效） | [delivery](architecture/delivery.md#忙碌策略三級)、D16 |
| Stop hook decision | Stop hook decision | — | claude Stop hook 的輸出 `{"decision": "block", "reason": …}`：turn 結束時把排隊的訊息當成下一個 turn 送進去。 | **決策**；**請示** | D16、[delivery](architecture/delivery.md#claude-特別規則d16) |
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
| notifier | notifier | `traits::Notifier`、daemon `notifier` 模組 | 對外通知的 adapter（Telegram）；Telegram 配對也由它做。 | 訊息送達（給 agent 的走 driver） | [README](../README.md#系統圖)、D13 |
| shim | git shim | crate `agend-shim` | 只放在 agent PATH 上的 git／kill 防護：導向 worktree、擋自建 branch／worktree 與寫 main；讀唯讀 binding 快照。 | 使用者自己的 git（shim 不改它） | D5、D6 |
| worktree | worktree | `model::worktree_dir`、`model::work_branch` | 只由 daemon 建立的 git worktree：`worktrees/<task-id>/`，branch `agend/<task-id>/<slug>`；審查另有 detached worktree。 | workspace（每個 instance 的常駐工作目錄） | [pipeline](architecture/pipeline.md#worktree-與-branch-生命週期) |
| 協定（client／holder） | protocol (client／holder) | `protocol::client`、`protocol::holder` | 兩套有版本的協定：client 協定給 TUI／CLI／GUI 連 daemon；holder 協定給 daemon 連 holder。 | backend 的協定（app-server、HTTP + SSE、hooks） | D1、D11 |
| hard gate | hard gate | `screen::HardGateKind` | 畫面上擋住 agent 的狀況：usage limit、permission／approval、rate limit、auth error、context full、啟動與更新選單。 | **施工關**（gate）；merge 門檻；approval 關卡 | [delivery](architecture/delivery.md#狀態偵測三層) |
| 螢幕分類器 | screen classifier | `screen::classify`、`SCREEN_RULES` | 直接讀 holder 畫面、只認 hard gate 的分類器；規則是資料，每條附真實畫面 fixture。 | 結構化事件（判斷 busy／idle 的來源） | [delivery](architecture/delivery.md#狀態偵測三層) |

## 介面

| 名詞（中文） | English | 程式識別字 | 定義 | 不要跟…混淆 | 出處 |
|---|---|---|---|---|---|
| 需要你 | needs-you | `DaemonEvent::AttentionRequired`、`NotificationSeverity::Attention` | 要人處理的例外（請示、門檻卡住、agent 卡住）：TUI 首頁最上方跨 team 的區塊，Telegram 有同名 topic。 | 已讀（看過不會離開「需要你」） | D13、[tui-and-setup](architecture/tui-and-setup.md#tui)、[README](../README.md#這是什麼) |
| 請示 | ask (needs-you item) | — | 「需要你」裡 agent 問人的一項，由 `agend ask` 建立；可以是選項選擇，也可以是自由文字的多輪對話（v1 叫 `decision`）。 | **決策**（D 編號）；**Stop hook decision** | 規劃 §3.1、D35（即將提出） |
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
