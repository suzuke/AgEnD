# agend v2 規劃 r4

> 歸檔註：設計階段的原始紀錄，內容原樣保存（2026-09-24）。只把絕對路徑換成佔位符：`<scratchpad>` = 當時的暫存目錄、`<v1-repo>` = agend-terminal repo、`<tmp>` = 系統暫存目錄、`~` = 使用者家目錄。文中提到的腳本、log、schema dump 沒有一起歸檔。

> 狀態：§1、§2 為使用者已確認；§3 以後是設計，已吸收兩輪對抗性審查（r1→r2、r3→r4），r4 本身尚未再審。
> v1 = agend-terminal（HEAD `6586472a`）。**v1 是研究過的參考資料，不是約束**：好的零件可以拿來用，
> 但每個設計都依需求重新選擇。原則：KISS。
> 證據：同目錄 inventory.md、usage.md、history.md、competitors.md、runtime-spike.md、injection.md；
> 使用量取自 `~/.agend-terminal/mcp-usage-stats.jsonl*`（2026-09-14～24，約 10 天）。

## 1. 目標與定位（使用者確認）

- **定位：異質 agent 團隊的自主 merge 流水線。** 不拼終端多工器的操作體驗（herdr，4 萬星）、
  不拼支援的 agent 數量、不拼手機 App 的精緻度。
- **差異化**（17 個同類專案中無人做好，見 competitors.md）：
  1. 機制保證的自動 merge 門檻：核准綁 head SHA + checks + git shim + 每個 repo 可選自動或人工核准。
  2. 跨廠牌：claude / codex / opencode 互審；某家額度用盡時 task 轉給別家。
  3. 派工時就避免衝突：daemon 知道每個進行中 task 動到的檔案，重疊就警告或排序；merge 前 rebase 並依序合併。
  4. 人只處理例外：只有請示、門檻卡住、agent 卡住時才通知 Telegram。
  5. git 防護內建於 orchestrator。
- **第一優先是自主開發團隊**；工作台（TUI）、手機遙控、用 agend 開發 agend 都是它的介面。
- 開源；第一版 macOS + Linux；backend 為 claude / codex / opencode。
- 要解決的痛點：agent 卡住沒人發現、worktree/branch 混亂、看不懂 fleet 在做什麼、系統本身不穩、訊息注入不穩。

## 2. 已確認的設計決定

| # | 決定 |
|---|---|
| D1 | daemon 對外只有一套公開、有版本的 protocol（事件串流 + 終端串流）。TUI 是第一個 client，GUI 之後接上。 |
| D2 | daemon 常駐（launchd/systemd），可從 TUI restart / shutdown；重啟前預檢新 binary，失敗就不切換。 |
| D3 | daemon 重啟時 agent 不斷線。runtime 採**自有 holder**：以 v1 的零件組成（portable-pty + alacritty_terminal），每個 agent 一個 holder，持有 PTY、畫面狀態與附屬程序。保留一層薄 `Runtime` 介面。 |
| D4 | 流水線拆成 Forge（第一版：local、github）與 Checks（指令、forge CI），每個 repo 各自設定。 |
| D5 | git shim 是一級元件，放在 v2 repo 內做成獨立 crate，隨同一個 binary 發布，以 argv[0] 分派。 |
| D6 | 不用 HMAC。binding 存 DB，daemon 為每個 agent 寫出唯讀快照檔給 shim。 |
| D7 | agent 介面以 CLI 為主；handler 與傳輸分離，需要時由同一份定義產生 MCP 轉接層。 |
| D8 | instance 全部存 DB（不再有 fleet.yaml）。透過 CLI／TUI 建立與修改；常駐或臨時是 DB 裡的一個屬性。 |
| D9 | 每個模組都能獨立驗證與測試（§5.1）。 |
| D10 | crate 劃分準則：編譯器強制的限制／獨立程序／對外使用／多 crate 共用的測試基礎設施；其餘為模組 + trait。新增 `agend-client`、`agend-testkit`（dev-only）、`xtask`。 |
| D11 | protocol 放 core（外部 GUI 用產生的 JSON schema）；agend-client 同步 I/O；driver／forge／store 為 daemon 內模組 + trait；holder 與 daemon 同一 binary、每個 instance 一個 holder；流水線狀態機（6 種關卡 + task 關係與操作）放 core 為純函式。 |
| D12 | 每個 instance 同時只屬於一個 team。內建一個不可刪除的預設 team `general`，沒指定 team 的 instance 與 task 都歸它——不存在「沒有 team」的特例；`general` 和其他 team 一樣有共享目錄。 |
| D13 | team 共享目錄 `teams/<team>/` 放 team 說明（注入給成員）、共享筆記、共享產出；成員各自的 workspace 不放進去。Telegram：一個「需要你」topic + 每個 team 一個 topic；個別 instance topic 可選、非預設。 |
| D14 | main 前進後：自動 rebase 並重跑 checks；rebase 無衝突且 branch 自身 diff 的 `git patch-id` 不變則保留核准，否則退回 work。 |
| D15 | team 有 0 或 1 個 repo。workflow 宣告需求（`code` 需要 repo；`research` 不需要）；無 repo 的 team 沒有 worktree 與 shim。預設 team `general` 無 repo。跨 repo 工作用跨 team 的 `depends_on`。 |
| D16 | claude driver 採互動式 TUI + hooks：狀態看 hooks；排隊用 Stop hook `decision: block`（先取出佇列再回傳，`stop_hook_active` 防迴圈）；閒置時經 channel 送；中斷 = `Esc` 後立即經 channel 送（Esc 不會觸發 Stop，不做等待）。專案 CLAUDE.md 必須說明 agend channel 訊息來自使用者自己的團隊（追加 spike：無說明 0/3，有說明 3/3，opus 2/2）；訊息內可另加 from/task/request 標頭。 |
| D17 | CLI 分 agent 命令（11 個：status、done、result、review approve/changes、send、inbox、ask、block/unblock、task create、remind）與操作者命令；daemon 依呼叫者身分限制權限，拒絕時附正確命令。 |
| D18 | agent 不建立 instance：`agend task create --role <角色>`，daemon 依 team 的角色範本分派或開臨時 instance。角色範本 = 允許的 backend、模型等級、指示、人數上限（min/max）、session 策略。分派規則：審查排除作者並優先不同 backend；退回修改回原作者；超過上限就排隊；等待 fanout 的父 task 不佔名額，偵測 team 內互等並通知；需要不存在的角色時自動轉成 ask；額度用盡時改派其他允許的 backend。沒有「agent 自己開 instance」的退路。 |
| D19 | workflow 定義格式為 TOML（DB 為真相來源，編輯時匯出／存回，每次存檔為新版本）。存檔檢查：id 唯一、kind 合法、submit 前需有產出 branch 的 work、有 submit/merge 必須 `requires = ["repo"]`、merge 前需有 `bind_head` 的 approval（否則須明寫 `allow_unreviewed = true`）、`on_fail` 只能指向前面的關卡、角色須存在於套用的 team。命令：`agend workflow list/show/new --from/edit/apply/check/history/rollback/delete`、`agend team set-workflow`；內建 workflow 唯讀。 |
| D20 | 取消「每個 repo 選自動或人工 merge」的設定；需要人工核准就在 workflow 裡於 merge 前加 `approval(by = "human")`。 |
| D21 | task 建立時記下 workflow 版本並固定；修改 workflow 只影響之後新開的 task，進行中的 task 照原版本走完，不做關卡對應轉換。 |
| D22 | 施工依 crate 由下往上分 12 關（core → testkit → shim → holder → store → daemon↔holder → codex driver+delivery → client+server → CLI → pipeline/git/forge local/supervisor → TUI → 其餘 adapter）。每關單獨驗收（`cargo test -p`、clippy、check-deps、`cargo xtask accept <關>`、文件更新、verifier），使用者確認後才開下一關；需要改 core 時先改 core 並重過第 1 關。 |
| D23 | 文件以繁體中文為主、程式輸出以英文為主；每個 crate 有 README.md 與 TESTING.md；AGENTS.md 為所有 agent 的唯一入口（不建 CLAUDE.md）；文件以 ADHD 讀者為預設（TL;DR、先結論、表格與清單、固定章節、下一步）。 |
| D24 | 新增第 13 關「安裝與發布」（服務註冊、uninstall、telegram setup 由 daemon 配對、xtask release、brew、GitHub release、cargo install；CI 以全新 HOME + 假 agent 從安裝到第一個 task 完成 < 5 分鐘；每項 doctor 檢查都有「故意弄壞 → 看到修正指令」測試）。第 9 關只做 doctor 與 init。安裝規則（版本範圍、登入偵測、git 最低版本、服務設定檔文字）放 agend-core 的 `setup` 純函式模組，由 agend 的 `setup` 模組執行。 |

## 3. 範圍

### 3.1 v2.0 包含

| 功能 | v1 證據（10 天） | v2 形式 |
|---|---|---|
| 訊息收送 | inbox 18,942、send 9,681 | 推送帶完整內容；inbox 只作補查 |
| task board | 12,196（其中 `task get` 4,460） | `agend status` 取代大部分 `task get` |
| review | reviewed_head 3,716；disposable review worktree 613 | review 指派為一級概念（§4.5） |
| checks | ci 1,596 | Checks 介面 |
| worktree | repo 1,360 + release_worktree 1,114 | 由 daemon 依 task/review 指派自動建立與回收 |
| decision + Telegram | 900 / 824 | 保留 |
| instance、team、set_waiting_on、interrupt、set_model | 20～343 | 保留 |
| schedule | 67（98 筆中 90 筆是 once） | 保留 cron；once 視為 task 的提醒 |
| operator_page、usage limit 處理、daemon restart | 54 / 38 / 11 | 保留 |
| TUI | 日常介面 | 流水線儀表板 + agent 清單 + 單一 agent attach；分割視窗之後再做 |

### 3.2 不包含

Discord、tray、grok/kiro/agy/shell backend、Windows、deployments、quickstart 精靈、skills 同步、token 成本統計、
`repo merge`（改由 daemon merge）、v1 的治理機制（receipt、claim_verifier、HMAC、operator_mode 簽章、assignment_authority 等約 18 個）。
之後要加必須附使用證據。

## 4. 架構

### 4.1 程序模型

```
TUI / GUI(未來) / CLI / Telegram
          │ protocol v1（unix socket；GUI 需要時加 WebSocket）
          ▼
┌──────────── agend daemon（常駐、tokio multi-thread runtime）────────────┐
│ protocol server ─ handlers ─ 流水線 ─ supervisor ─ checks/telegram/排程  │
│ DB 執行緒（SQLite，唯一持久狀態）    git（tokio::process + timeout）       │
└──────────┬───────────────────────────────────────────▲────────────────┘
           │ holder 協定（有版本、向後相容；daemon 重啟後重連）  │ agend CLI、hook 事件、結構化事件
           ▼                                           │
   holder ×N：PTY + alacritty_terminal 畫面 + 附屬程序（codex app-server / opencode serve）
           │                                           │
           ▼                                           │
   codex / claude / opencode ── PATH：git、kill → shim；agend CLI ──┘
```

- **agent 側不得有 daemon 的子程序**。附屬程序由 holder 啟動並持有；v1 由 daemon 啟動 codex app-server（`src/transport/codex_app_server.rs:286-316`）與 opencode serve（`src/transport/opencode_server.rs:452-500`），這是 v2 必須改的地方。
- **daemon 重啟後的恢復**：重連 holder → 直接取得 holder 裡的現成畫面（不重播位元組）→ 重連 codex app-server（WebSocket，用 `thread/read` 補回斷線期間的 turn 結束）、opencode（SSE 無 replay，用 `GET /session/status` 補狀態）、claude channel bridge（SSE 帶 last_event_id）。
- **claude hook 事件**：daemon 不在時寫入磁碟佇列，daemon 回來後補送；恢復後再以螢幕分類器確認一次狀態。v1 在 daemon 停機時直接丟棄（`src/main.rs:1243-1245`），漏掉 Stop 就會一直被當成 busy。
- **舊版程序的相容**：holder 與 claude channel bridge 升級前仍會跑舊 binary，兩者與 daemon 的協定都必須有版本且向後相容。
- **I/O**：runtime 為 multi-thread；SQLite 由專屬執行緒持有，透過 channel 存取；git、gh、checks 指令一律 `tokio::process` 並設 timeout。v1 兩天有 861 次「scanner-thread slip」。
- **自舉隔離**：daemon、holder、shim 都跑已安裝的 release 版；開發中的 agend 在另一個 clone。

### 4.2 holder

- 職責：spawn agent 與附屬程序、PTY 讀寫、resize、signal、以 alacritty_terminal 維護畫面、提供畫面快照與輸出串流、回報 exit code。
- 很少更新；協定有版本協商（v1 的 `src/framing.rs` 只有一個版本位元組、沒有協商）。
- 限制（spike 實測）：holder 自己被硬殺，裡面的 agent 會一起死。這是所有方案共同的上限。

### 4.3 agent 狀態偵測

1. 結構化事件判斷 busy / idle / exited。
2. 螢幕分類器直接讀 holder 的畫面，只認 hard gate：usage limit、permission/approval、rate limit、auth error、context full、啟動與更新選單。hard gate 不被結構化事件覆蓋。每條規則附真實畫面 fixture。
3. 去抖動：idle↔active 穩定 N 秒才生效（v1 約兩天 75 萬次轉換）。

### 4.3.1 啟動與授權提示（trust、yes/no、更新選單）

v1 證據：以 bypass 旗標啟動（`src/backend.rs:419,528`），其餘靠刮螢幕自動應答：`src/agent/dismiss.rs`（44 KB）+ `dev_modal.rs`（59 KB），27 個 commit，2026-09-14 仍在修。
每次 backend 改版畫面一變就要追著修。v2 分四層，越上層越優先：

1. **避免出現**：選不會跳提示的啟動方式與旗標；啟動前預先寫好 trust 狀態（例如 codex 的 `[projects."<path>"] trust_level = "trusted"`，
   v1 `src/backend.rs:553-554` 的註解提到；claude 的對應設定待 spike 驗證）；盡量不用需要逐次確認的實驗旗標
   （v1 的 dev_modal 就是 `--dangerously-load-development-channels` 帶來的確認框）。
2. **結構化處理**：工作中的授權請求走結構化管道——codex app-server 的 approval 請求、opencode 的 permission 事件、claude 的 permission hook——
   由 daemon 的政策決定或轉問人，不從畫面判斷。
3. **已知提示規則是資料，不是程式**：每個 backend 一份規則檔（比對樣式 → 要送的單一按鍵），附真實畫面 fixture，改版時只改規則檔。
4. **未知提示的兜底**：啟動後 N 秒內沒進入 ready、畫面又停住不動 → 狀態標為「卡在未知提示」→ 把畫面快照送到 TUI／Telegram 請人處理；
   人的回答以單一按鍵送出，這個畫面同時存成新規則的候選 fixture。不盲目亂按。

另外，daemon 偵測到 backend 版本變了，先在暫存 workspace 起一個 canary instance 確認能正常進入 ready，再讓整個 fleet 重啟；
不行就提前通知，而不是整批卡住。

### 4.4 訊息送達模型

v1 的教訓（injection.md）：改走結構化 API 後，「貼上沒送出」類問題幾乎消失（26 次修復中 23 次在 2026-04）。
剩下的不穩定來自忙碌處理各自為政、三套去重機制並存、狀態誤判導致時機錯誤。

- 每則訊息有 id，狀態為 `queued → sent → confirmed | failed`；以 id 冪等，只有一套去重。
- **忙碌策略三級**，由 daemon 依訊息的緊急程度決定：

| 等級 | codex | claude | opencode |
|---|---|---|---|
| 排隊（turn 結束後送） | 等 turn 結束再 `turn/start` | channel（本來就排到下一個 turn） | 自建排隊，turn 結束時送 |
| 插入（不中斷） | `turn/steer` | 無程式化方式 → 改用中斷 | 未查證 |
| 中斷後立即處理 | interrupt 後送 | `Esc` 後經 channel 送 | session abort（未查證） |

- **各 backend 實測對應（2026-09-24 spike，見 spike-codex/、spike-claude.md、spike-opencode.md）**：

| | codex 0.156.1 | opencode 1.18.31 | claude 2.1.281 |
|---|---|---|---|
| 重連 | `thread/resume` + `thread/turns/list` 補回 | SSE 無 replay，以 REST 補狀態 | hooks 重新觸發 |
| 狀態來源 | app-server 事件 | SSE + REST 核對 | hooks |
| 排隊 | `thread/queue/add` | `prompt_async`（server 原生 FIFO） | Stop hook `decision: block` |
| 插入 | `turn/steer` | 不支援 → 中斷 | 不支援 → 中斷 |
| 中斷 | `turn/interrupt` | `POST /session/:id/abort` | `Esc` 後立即送（需 CLAUDE.md 來源說明） |
| resume | id | id | `--resume <id>` |
| 授權 | `requestApproval`（6 種回覆） | 以 `GET /permission` 輪詢為準 | allowlist；拒絕情況未釐清 |
| 啟動提示 | trust（預寫 config 可跳過） | 無 | trust、MCP trust、dev-channels |

  陷阱：codex socket 長路徑只是 symlink，連線前 `realpath`；codex sandbox 內連不到 workspace 外的 unix socket（daemon socket 放進 workspace 範圍或由 holder 自動核准）；
  claude 閒置時 channel 可靠、忙碌時訊息被擱置不處理；opencode `permission.asked` SSE 事件曾漏發一次。
- 訊息內容一律走結構化 API；**PTY 只允許送單一控制鍵**（如 `Esc`），不模擬打字。
- 不能確認送達的路徑明確標示為未確認，不假裝成功。
- 推送一律帶完整內容。v1 會截斷超過 200 字的訊息或只推標頭（`src/inbox/notify.rs:228-256`），是 inbox 被呼叫 1.9 萬次的主因。

### 4.5 流水線、worktree 與 review

```
task → 指派 → 工作 worktree → 開發 → 提交變更(Forge) → Checks → review(綁 head) → daemon merge → done
                         ▲                 │ 失敗            │ 要求修改／新 commit 使核准失效
                         └─────────────────┴─────────────────┘
```

- **binding 兩種**：
  - 工作 binding：(instance, task, branch, worktree)，由 daemon 在指派 task 時建立。
  - review binding：daemon 在指派 review 時，於被審的 head 建立 detached 的審查 worktree，並記下該 head。reviewer 的 `approve` 自動綁定這個 head。
  - 每個 agent 同一時間只有一個作用中的 binding（shim 只導向一個 worktree）；多個待審指派排隊處理。
- **head**：`refs/heads/<branch>` 目前指向的 commit，不含未 commit 的變更。**diff**：`main...branch`。
- **Checks `command`**：在該 head 的臨時 detached worktree 執行，不在開發中的 worktree 執行。
- **merge**：只由 daemon 執行，條件是 checks 通過，且核准的 head 等於目前的 head。
  - Forge local：用 `git merge-tree --write-tree` 產生結果，再以 CAS 方式 `update-ref`（比對 main 仍是預期的 SHA），不在使用者的工作目錄裡執行 `git merge`。canonical 若 checkout 在 main，要處理工作目錄過期。
  - Forge github：透過 API merge PR，並帶上核准的 head SHA。
- **main 前進後的政策**（待定）：checks 是否要在合併後的結果上重跑、既有核准是否失效。
- **done**：以 daemon 的 merge 記錄為準，不從 git 推論（v1 有 2,973 次 ancestry compare 失敗）。之後清理 worktree；有未 commit 的變更就保留並提示。
- **衝突預防**：daemon 記錄每個進行中 task 動到的檔案，派工時發現重疊就警告或排序；merge 前先 rebase，並依序合併。
- 每個 repo 的設定：forge、checks、review 必要或選擇性、merge 自動或等人核准。

### 4.5.0 workflow 模型（關卡組合）

workflow 是基本關卡的依序組合，存在 DB 並帶版本；新增組合或調參數不需改程式碼。

| 基本關卡 | 參數 |
|---|---|
| `work` | 角色、指示、產出（branch 或 result） |
| `command` | 指令、timeout、執行位置（head 的臨時 worktree） |
| `approval` | 核准者（人或角色）、人數、是否綁 head |
| `submit` | forge |
| `merge` | 門檻 |
| `fanout` | 子 task 的來源（由 work 產出或明列）、匯合方式：`all`（全部完成）／`first`（第一個完成）／`pick`（交給 approval 選出勝者，其餘取消） |

- 所有關卡共用參數：`timeout` 與逾時動作（通知、改派、取消）。
- task 層級的關係（不是關卡）：`parent`、`depends_on`（可修改、可跨 team）、`superseded_by`。
- task 層級的操作：改派、reopen（done 之後由人重新打開）、supersede（輸入變了，由新 task 接手，不算失敗）。
- 內建 workflow：`code`（work → submit → command → approval → merge）、`research`（work(result) → approval）、`epic`（work(plan) → fanout → approval）。

驗證（workflow-validation.md，v1 8,347 個 task、18 個 board，分層抽樣 93 個）：原 5 種關卡完全符合 76%、需要新參數 16%、需要新型別 8%。
缺口為：子 task 拆分與匯合（完成的 task 中 46.6% 有 parent，最多一個 parent 77 個子 task，另有「同一題派給兩個 agent 比較後選一個」）、
task 依賴、supersede（2.1%）、reopen（0.2%）、等待外部事件且有期限。以上分別由 `fanout`、task 關係與操作、共用 `timeout` 參數涵蓋。
另一個發現：v1 定義的結構化事件 `Verified`、`Linked`、`TaskCloseProposed`、`OperatorSettled` 在 8,347 個 task 中一次都沒被使用，
核准與 PR 連結全靠 agent 在 `ResultSet` 裡寫自由文字——再次說明記帳要由 daemon 自動完成，不能指望 agent 呼叫額外的命令。

### 4.5.1 worktree 與 branch 的生命週期（避免孤兒）

v1 證據（agend-terminal repo，2026-09-24）：本機 branch 137 個，git 判定已合併的只有 2 個（squash merge 讓 ancestry 推論失效）；
62 個沒有 upstream；`~/.agend-terminal/worktrees/` 49 個目錄；清理機制約 9 個（branch_sweep、cleanup_intents、worktree_cleanup、
auto_release、janitor、orphan_sweep、boot_sweep、task_sweep、shutdown_cleanup）；「worktree has WIP, skipping GC」兩天 301 次。

1. 只有 daemon 建立 worktree 與 branch；shim 擋 agent 自建（`git worktree`、`checkout -b`、`switch -c`、`branch <new>`）。
2. 先記錄再建立：DB 先寫「準備建立」，建好再標記完成；崩潰後開機可接續。
3. 固定命名空間：branch `agend/<task-id>/<slug>`、worktree `worktrees/<task-id>/`。命名空間內但 DB 無記錄即為孤兒；命名空間外 daemon 一律不碰。
4. 清理由關卡事件觸發：merge 完成（daemon 自己 merge，無須 ancestry 推論）→ 刪 branch 與 worktree；審查 worktree 在核准或駁回時刪；task 取消走同一流程。
5. 已結束 task 仍有 WIP：存成 patch（diff + 未追蹤檔）放封存區（有保留期限）後照常刪除，並顯示在總覽。
6. 單一對帳程序：開機與每日比對 DB 與 git 實況，雙向處理；取代 v1 的約 9 個清理機制。

### 4.6 git shim

- 以 v1 agentic-git 為基礎。binding 格式沿用 BindingV1（agent / task_id / branch / worktree / source_repo），加上 binding 類型（工作或審查）。
- 移除 HMAC：讀取端 `verify_sidecar`（`agentic-git/src/lib.rs:579-603`）、寫入端 `ensure_key` 與 `sign_binding`（`cli.rs:468`、`cli.rs:974`），並刪除 `integrity_core`。
- **Forge local 必須補 protected-ref 檢查**：擋 `update-ref`、`push .`、`branch -f` 對 main 的寫入。v1 對已綁定 agent 的 `update-ref` 直接放行（`classify.rs:752-759`）。
- 快照檔設為唯讀；和 HMAC 一樣只是安全帶，同 uid 的 agent 仍能 chmod。文件要寫明。
- shim 依賴的環境：agent 身分與 home 的環境變數、bypass 用環境變數、父程序是否為 gh、canonical repo 判定、worktree 路徑存在與否、真 git 的定位（排除 shim 目錄）。holder 啟動 agent 時負責注入。
- kill、killall、pkill 的防護 shim 一併保留（v1 `~/.agend-terminal/bin` 已有）。

### 4.7 agent 介面（CLI）

- **身分與上下文一律從「呼叫者身分 → DB 裡的 binding」推得，不看 cwd**（v1 agent 的 cwd 是 `workspace/<instance>`，不是 worktree）。
- agent 只傳意圖：收件者、訊息內容、請求類型或 review 結論、完成條件、附件、回覆對象（有多個未回訊息時必填）、期望回覆時間。
  daemon 推得：task_id、branch、repository、PR 編號、reviewed_head / expected_head、correlation_id、binding 相關參數。
- 命令總數 < 15、常用命令參數 ≤ 2；`agend status` 顯示所在步驟與可用的下一步；錯誤訊息附可執行的正確命令；`--help` 範例優先；支援 `--json`。
- daemon 重啟中：CLI 重試最多 10 秒後印出明確訊息。
- 啟動路徑約束：CLI 與 shim 模式不建 runtime、不讀設定、不開 DB；argv[0] 分派放在 main 最前面。實測啟動 p50 4.1 ms、unix socket 來回 0.014 ms。

### 4.8 設定與目錄

- **唯一真相來源是 DB**：instance、team、repo 與其流水線設定（forge、checks、review、merge 政策）都存 DB，由 `agend instance ...`、`agend repo ...` 與 TUI 管理。
  - instance 的 `lifetime` 屬性：`persistent`（常駐，daemon 啟動時拉起）或 `ephemeral`（隨 task／team 結束清理）。
  - 不再有 daemon 需要寫回的 YAML，v1 的 fleet.yaml 備份與註解保留問題一併消失。
- **設定檔只剩 daemon 層級**：`config.toml`（人寫、daemon 只讀）放 home 路徑、Telegram 等連線設定；secret 以環境變數或檔案路徑引用，不直接寫進 DB。
- **可重現與備份**：`agend export` / `agend import` 以文字格式匯出與匯入 instance、team、repo 設定（用於備份、搬機、分享範例）；
  daemon 每天以 `VACUUM INTO` 做 DB 快照，保留 N 份。
- 每個 instance 有常駐工作目錄（v1 的 `workspace/` 佔 23G），定義其內容與清理規則；給 agent 一個會被清掉的暫存目錄（v1 的 108G `evidence/` 是 agent 自己寫的）。daemon 監看 home 目錄大小並警告。

### 4.9 安裝與設定

v1 證據：#2207（空 allowlist 讓 Telegram 雙向訊息靜默丟棄，detached 模式下看不到錯誤）、#2005（quickstart 寫入與讀取的 token 變數名不一致）、
#3402（codex 專案設定未被信任）、#3499（關終端 daemon 跟著死）、#1351／#2204（quickstart 流程反覆改）。共同點：設定錯了不報錯，只是靜靜不動。
原則：**每個設定錯誤都在第一次使用前被明確指出，並附修正指令。**

1. `agend doctor`：git 版本（merge-tree 需 ≥ 2.38）、gh 登入（僅 github forge 需要）、各 backend 安裝／版本是否在已測範圍／登入、服務狀態、磁碟、
   Telegram（token 可連線、allowlist 為空即報錯）。每項附修正指令；支援 `--json`。
2. `agend init`：建 home 與 `config.toml`、註冊 launchd／systemd、偵測 backend、建 `general` team 與一個 agent；在 repo 內執行時詢問是否登記該 repo；
   最後跑 doctor。只在互動終端才發問，否則全用參數。
3. TUI 空畫面即引導：第一步是「開始第一個 agent」，repo 為可選的下一步；不做獨立精靈。
4. `agend telegram setup`：貼 token → 對 bot 傳 `/start` → 自動取得 chat id 並加入 allowlist。
5. 自動處理 trust 設定、shim 安裝、agent PATH；shim 只進 agent 的 PATH，不改使用者自己的 git。
6. `agend uninstall`：移除服務與 shim；資料是否刪除另外詢問。
7. 驗收：從安裝到第一個 agent 完成 task < 5 分鐘；CI 以全新 HOME + 假 agent 跑 e2e。
8. 安裝管道：brew、GitHub release 預編譯 binary、`cargo install`。

## 5. 資料夾結構

```
agend/
├── Cargo.toml                 # workspace
├── crates/
│   ├── agend-core/            # 純邏輯；不依賴 tokio、rusqlite、std::process
│   │   └── src/ config.rs（config.toml）, model/, protocol/（client、holder）, traits/, pipeline/（狀態機）,
│   │            policy/（busy、debounce、conflict）, screen/
│   ├── agend-daemon/          # 入口：protocol server、handlers、hook 接收
│   │                          # 領域：pipeline 執行、delivery、supervisor、scheduler
│   │                          # adapter：driver/{codex,claude,opencode}、runtime（holder client）、forge/{local,github}、
│   │                          #          checks/{command,forge}、store（sqlite）、notifier（telegram）、git
│   ├── agend-holder/          # portable-pty + alacritty_terminal + 附屬程序；holder 協定 server
│   ├── agend-shim/            # git 與 kill 防護（含 audit 記錄）；只讀 binding 快照
│   ├── agend-client/          # 同步 I/O 連 daemon、重試、protocol 版本檢查；TUI／CLI／未來 Rust GUI 共用
│   ├── agend-tui/             # 儀表板、agent 清單、attach
│   ├── agend/                 # 唯一 binary：argv[0] → shim；子命令 daemon / holder / app / CLI / debug
│   └── agend-testkit/         # dev-dependency only：各 trait 的假實作、契約測試套件、假 daemon server、
│                              # 假 agent 程式（模擬 codex app-server、opencode serve、帶 hook 的 claude）、git/binding fixture 產生器
├── xtask/                     # 開發者工具：產生 protocol JSON schema、打包 release、錄製 backend 畫面 fixture
├── docs/ ARCHITECTURE.md, DECISIONS.md, BACKEND-BEHAVIORS.md
└── tests/ fixtures/screens/, e2e/, learnability/
```

### 5.1 模組獨立驗證（D9）

規則：
1. **依賴方向單向**：模組之間只透過 `agend-core` 定義的 trait 與型別溝通；每個 crate 可以單獨 `cargo test -p <crate>` 通過，不需要啟動其他模組。
2. **每個外部邊界有 trait + 假實作**：Runtime、Driver、Forge、Checks、Store、Clock、Notifier（Telegram）。
3. **契約測試**：同一套測試同時跑真實作與假實作，確保假的不會和真的漂移
   （v1 教訓 #1483：測試手寫的輸入格式，production 從來不會送出，matcher 實際上是死碼，但測試全綠）。
4. **輸入用 producer 產生**：測試 consumer 時，用真正的 producer 產生輸入，不手寫格式（v1 #1493 規則）。
5. **每個模組可以單獨跑起來**：holder、shim、daemon 都能獨立啟動並以指令操作，方便人工與 agent 驗證。

| 模組 | 怎麼獨立驗證 |
|---|---|
| agend-core | 純函式單元測試 + property test（busy 策略、去抖動、衝突偵測、merge 門檻判定） |
| agend-holder | 用 `sh`、`cat` 這類假程式取代 agent：PTY 讀寫、畫面快照、resize、exit code、daemon 斷線重連 |
| agend-shim | 暫存 git repo + binding 快照 fixture：導向、拒絕、快照與還原、protected-ref |
| store（DB） | in-memory SQLite：migration、交易、retention、`VACUUM INTO` 備份 |
| delivery | 假 driver：排隊／插入／中斷、冪等、重連後不遺失不重複 |
| driver/codex、opencode | 假 app-server（JSON-RPC over WebSocket）、假 opencode HTTP/SSE server；另有對真 backend 的選擇性 smoke test |
| driver/claude | 假 channel bridge + hook 事件；真 backend smoke test |
| screen 分類器 | `tests/fixtures/screens/` 的真實畫面 |
| forge/local | 暫存 repo：merge-tree、CAS update-ref、main 前進時的衝突 |
| forge/github | 錄製的 HTTP 回應或假 server |
| checks/command | 暫存 worktree 內跑假指令：成功、失敗、逾時 |
| pipeline | 假 Forge + 假 Checks + 假 Driver：task 從派工走到 done 的完整狀態轉換 |
| protocol server / CLI | 假 daemon 或 in-process daemon；CLI 輸出與錯誤訊息的 snapshot |
| agend-tui | ratatui `TestBackend` 畫面 snapshot，餵假 protocol 事件 |
| e2e | 真 daemon + 真 holder + 假 backend；每階段驗收用 |

## 6. 階段

| 階段 | 內容 | 驗收（使用者可觀察） |
|---|---|---|
| 0 Spike | ① holder 持有 codex app-server / opencode serve 時，daemon 重啟後能否重連並補回事件 ② codex app-server 是否把 TUI 手動發起的 turn 通知給 daemon ③ claude 被 `Esc` 中斷後 channel 訊息是否立即進入下一個 turn；send-now 有無程式化方式 ④ opencode 的插入與中斷 API ⑤ 以明確 id resume ⑥ codex sandbox 開啟時 CLI 能否連 unix socket ⑦ claude 以 allowlist 免除 `agend` 權限提示 ⑧ 三個 backend 的啟動提示能否全部以旗標或預寫設定避免；授權請求的結構化管道是否可用 | 每項有實測結論，寫入 BACKEND-BEHAVIORS.md |
| 1 骨架 | core + holder + daemon + DB + codex driver + 送達模型 + CLI 訊息 | 兩個 codex agent 互傳訊息；重啟 daemon 時 agent 不中斷、訊息不遺失不重複 |
| 2 TUI | 儀表板、agent 清單、attach、restart/shutdown | TUI 可看、可操作、可重啟 daemon |
| 3 流水線 | task、兩種 binding、shim、review、Forge local、Checks command、claude/opencode driver、螢幕分類器、衝突預防 | 本機 repo：派 task → 開發 → checks → review → daemon merge → done；learnability 首測 |
| 4 對外 | Forge github、Checks forge、Telegram、decision、operator_page、schedule、usage limit 轉派 | 手機收請示並回覆；GitHub repo 走完整流水線；額度用盡時 task 轉給別家 |
| 5 並行 | 另一個 Telegram bot、另一份 repo clone、另一個 home，與 v1 並行一週 | 一週內日常工作不需回 v1；learnability 複測 |

## 7. 風險

- **第二系統效應**：範圍以 §3.1 為準，新增需附使用證據。
- **自有 holder 的維護成本**：協定相容與 TUI 維護會回來；TUI 範圍刻意壓小。
- **自動 merge 的信任**：業界主流（WorkOS、Factory.ai、Cursor）反對 agent 自動 merge；v2 必須把門檻機制講清楚，且預設可選人工核准。
- **backend 協定演進**：driver 隔離 + 螢幕分類器兜底 + BACKEND-BEHAVIORS.md 隨版本更新。

## 8. 更正記錄

| 版本 | 被推翻的前提 | 實況 |
|---|---|---|
| r1 | 狀態只取自結構化事件 | hard gate 由螢幕判定；usage_limit 10 天 1,589 次轉換 |
| r1 | worktree 綁 instance | 綁 task；repo 相關呼叫約 2,500 次 |
| r1 | v1 hot-restart 讓 agent 不斷線 | successor 等前任退出後才 spawn（`run_successor_handoff`，`src/daemon/mod.rs`） |
| r1 | reviewer 只是一個欄位 | review 參數每日上千次 |
| r1 | 以「fleet.yaml 註解被清」為理由 | 已由 `fee2430e` 修復 |
| r1 | 108G evidence 可由 retention 解決 | daemon 沒有寫入者，是 agent 寫的 |
| r2 | codex bypass sandbox，所以 shim 擋不住 | PATH shim 與 sandbox 無關 |
| r3 | holder 只管 PTY 就能讓 agent 不斷線 | codex app-server、opencode serve 是 daemon 子程序，須移到 holder |
| r3 | daemon 從 cwd 推得 task | agent 的 cwd 是 workspace；改從身分 → binding |
| r3 | binding 只有 (instance, task, branch) | reviewer 用 detached 審查 worktree（613 次），需第二種 binding |
| r3 | 推送後就不需輪詢 | v1 本來就推送，但截斷或只推標頭，才造成大量 inbox 呼叫 |
| r3 | Forge local 下 daemon 是唯一 merge 者 | shim 放行已綁定 agent 的 `update-ref`；需補 protected-ref 檢查 |
| r3 | 用 ring buffer 重建畫面 | 重播可能從轉義序列中段開始；改由 holder 持有畫面 |
