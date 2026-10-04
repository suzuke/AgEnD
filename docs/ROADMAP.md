# 施工路線圖：13 個施工關

> **TL;DR**
> - 依 crate 由下往上分 13 個施工關；每個施工關單獨驗收，使用者確認後才開下一個施工關（D22）。
> - 目前狀態：**第 1–11 施工關完成並已合併**；第 11 施工關 C 段 #145 經全新 verifier、雙平台 CI、實機及後續自動驗收與清理，使用者於 2026-10-03 確認合併（`b2152db`）。第 12 施工關 A 段設計 #138 已 merge，client 基礎實作中、Claude 接入未完成；第 13 施工關未開始。
> - 下一步：依第 12A [D40](decisions/d40.md) 實作不重送的通訊、持久化、channel／Stop 與明確 ACK；完成後由全新 verifier 與使用者驗收。

## 13 個施工關

每個施工關的細節、你親自驗收的步驟與紀錄在 [docs/gates/](gates/README.md)。

| 施工關 | 狀態 | 範圍 | 驗收（可觀察） |
|---|---|---|---|
| 1 `core` | [完成（2026-09-25）](gates/gate-01-core.md) | agend-core：型別、兩套協定（client + holder）、trait、流水線狀態機（6 種關卡）、busy policy、去抖動、衝突偵測、merge 門檻（patch-id）、螢幕分類器 | `cargo xtask accept core` 跑測試並印出一個模擬 task 走完 `code` workflow（純邏輯，無 daemon） |
| 2 `testkit` | [完成（2026-09-25）](gates/gate-02-testkit.md) | 每個 trait 的假實作、契約測試套件、假 daemon、假 agent 程式 | 契約測試通過；假 agent 可單獨啟動並回應 |
| 3 `shim` | [完成（2026-09-26）](gates/gate-03-shim.md) | git／kill 防護、導向 worktree、protected-ref、快照與還原 | 在暫存 repo 以 `git` 名稱執行 shim：導向、拒絕、快照後還原 |
| 4 `holder` | [完成（2026-09-26）](gates/gate-04-holder.md) | PTY、畫面、holder 協定、活過 daemon（附屬程序移到第 7 施工關） | `agend holder` 包 bash + 小型探測 client：啟動器結束後 holder 還在、讀畫面、送鍵、中途斷線重連、四次開機，bash 存活 |
| 5 `store` | [完成（2026-09-26）](gates/gate-05-store.md) | daemon store：SQLite schema、migration、保留期限、每日快照 | 真的 DB 檔（temp dir）、跨真的 process 驗；xtask 命令印出資料表 |
| 6 `daemon-holder` | [完成（2026-09-26）](gates/gate-06-daemon-holder.md) | 整合施工關：agent runtime adapter | 真 daemon + 真 holder；重啟 daemon，agent 與畫面存活 |
| 7 `codex` | [完成（2026-09-28；已 merge #132）](gates/gate-07-codex.md) | codex driver + 送達模型、三級忙碌策略 | 假 app-server 與真 codex 0.158.0 驗收通過 |
| 8 `client` | [完成（2026-09-26）](gates/gate-08-client.md) | 整合施工關：agend-client + protocol server | CLI 連得上；daemon 重啟時會重試 |
| 9 `cli` | [完成（2026-09-29；已 merge #136）](gates/gate-09-cli.md) | agend CLI：agent 命令、操作者命令、status；安裝相關先做 `doctor`、`init`，服務註冊留第 13 施工關 | 對假／真 daemon 驗輸出與錯誤；兩個假 Codex agent 互傳訊息、中途重啟不漏不重 |
| 10 `pipeline` | [完成（2026-10-02；#143 已確認合併）](gates/gate-10-pipeline.md) | daemon：pipeline、git、runner、forge local、supervisor、reconcile | 假 driver + 暫存 repo：task 從派工走到 merge |
| 11 `tui` | [完成（A、B、C 已 merge；C 段 #145）](gates/gate-11-tui.md) | attention-first TUI、完整終端、resize、多視窗、鍵鼠／貼上與歷史 | 最終 `cfee027` 全新 verifier CONFIRMED；四個雙平台 CI jobs 各 900 passed／0 failed／2 既有 ignored、實際 no-std；0.159.3 真 U17 已核實並獲版本許可。實機紀錄及後續自動驗收、清理完成，使用者確認 merge `b2152db` |
| 12 `adapters` | [實作中（A 段 #138 設計已 merge；Claude 接入尚未完成）](gates/gate-12-adapters.md) | A claude、B opencode driver、C forge github、D Telegram | 先對假實作，再做真 backend smoke test |
| 13 `install` | [未開始](gates/gate-13-install.md) | 安裝與發布（最後一個施工關）：服務註冊、`agend uninstall`、`agend telegram setup`（由 daemon 配對）、`xtask release`、brew、GitHub release、`cargo install` | CI 用全新 HOME + 假 agent，從安裝到第一個 task 完成 < 5 分鐘；每個 `doctor` 檢查都有「故意弄壞 → 看到修正指令」的測試 |

## 第 1 施工關：開工前先提案、經使用者確認才實作

7 項提案（P1–P7）列在 [gate-01-core.md](gates/gate-01-core.md#開工前提案)，使用者 2026-09-25 確認，記為決策 D26–D32。實作草稿是在確認前寫的；草稿作者 2026-09-24 自己打的勾不算確認。

## 安裝相關的程式放在哪

| 內容 | 位置 |
|---|---|
| 規則：已測的 backend 版本範圍、怎麼判斷已登入、git 最低版本、產生的 launchd／systemd unit 文字 | `agend_core::setup`（資料 + 純函式，無 I/O，符合 no_std） |
| 執行：跑指令、寫檔、註冊服務 | `agend` crate 的 `setup` 模組 |

## 里程碑（使用者可見）

| 完成到 | 使用者看到 |
|---|---|
| 第 1–9 施工關 | 已驗收：兩個假 Codex agent 互傳訊息；中途重啟 daemon，每則剛好一次並到 `confirmed`；真 Codex 另有第 7 施工關 smoke 驗收 |
| 第 10–11 施工關 | 本機 repo 從派工走到 merge，TUI 可看可操作 |
| 第 12 施工關 | 三個 backend + GitHub + 手機（Telegram） |
| 第 13 施工關 | 其他人可以自己安裝 |

之後：與 v1 並行一週（另一個 Telegram bot、另一份 repo clone、另一個 home），一週內日常工作不需回 v1；learnability 複測（規劃 §6 第 5 階段）。

## 每個施工關的完成定義

- [ ] `cargo test -p <crate>` 單獨通過
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `cargo xtask check-deps` 通過（不能是 SKIPPED）
- [ ] `cargo xtask accept <施工關>` 存在、會跑測試並印出人看得懂的 demo
- [ ] 該 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻
- [ ] 使用者完成該施工關「你親自驗收」清單並填寫驗收紀錄
- [ ] 使用者確認後才開下一個施工關

需要改 agend-core 時：先改 core，重過第 1 施工關的測試。crate 之間不得有私下耦合。

第 1 施工關的 demo 由 `agend-core` example 呼叫 protocol、policy、assignment 與 pipeline API。verifier 回饋修正後，workspace tests（167 個，含 105 core unit tests、11 個可完成性測試、兩個狀態機探索器、7 protocol compatibility tests、2 workflow TOML golden tests）、clippy、check-deps 與 acceptance 已通過；fresh-context verifier r6 CONFIRMED，已 merge（#105），使用者 2026-09-25 親自驗收通過。

## 第 0 階段（spike）

已完成；8 個問題的結論在 [BACKEND-BEHAVIORS.md](BACKEND-BEHAVIORS.md#第-0-階段的-8-個問題)。

## 下一步

第 1–11 施工關已完成並合併。第 11 施工關 C 段 #145 使用者於 2026-10-03 明確確認合併，原驗收範圍與限制見 [收尾紀錄](gates/gate-11c-closeout.md)。第 12A #138 的 P1–P10 已依本輪使用者確認寫定為 [D40](decisions/d40.md)，剩餘採建議；設計文件已依使用者「merge後開工」合併（#138、`4390633`）；第 12A 在獨立 worktree 實作中，Claude 接入尚未完成。

## 進度紀錄

以下是各批次**當時**的進度原紀錄；其中「draft」「待驗證」「尚未 merge」只描述該批次，不是目前狀態。目前以頁首完成狀態及最新合併紀錄為準。原失敗與驗證範圍不改寫成成功；歷史證據僅列封存檔名，不公布本機暫存位置。

- 2026-10-04 第 12A P1–P10 設計確認記為 [D40](decisions/d40.md)（[draft PR #138](https://github.com/suzuke/AgEnD/pull/138)）：閒置 channel／忙碌 Stop、明確 agend_ack、P3／P4／P5＝A；剩餘依建議。只改文件，最新 head 全新 verifier／CI 另核；未 merge、實作或新增真模型回合。

- 2026-10-03 使用者授權整理第 12A 提案 #138：對齊 #146 合併後 v2、P1 協定版本與 P7 migration 現況，拆分 P1–P10、歷史 F1–F10 與驗收計畫；全部決策仍待確認，未授權提案 merge 或實作。

- 2026-10-03 使用者明確確認「確認合併145」，#145 已以 merge commit `b2152db` 合併進 `v2`。最終 head `cfee027` 經全新 r9 CONFIRMED；四個 PR／push Ubuntu／macOS CI jobs 各 900 passed／0 failed／2 既有 ignored，實際 no-std 通過。合併 tree 與已驗證 head 完全相同。C 段實作／驗收完成，原實機、反證、真 U17 限制及清理證據保留；[收尾紀錄](gates/gate-11c-closeout.md)。

- 2026-10-03 獨立文件覆核補同步 daemon／testkit 的目前終端許可、Codex 歸屬快照／保留規則及 U17 12-case 索引，D39 加使用者後續自動驗證方式；B 段舊 wire 規則明示歷史範圍。程式／測試不變，新固定 head 另核 verifier／CI，#145 merge 待確認。

- 2026-10-03 全新文件 verifier 找到輸入政策、驗收計畫、demo 與 crate 入口仍要求逐步人工驗收；同步使用者已授權的剩餘自動驗證方式，歷史批次加範圍標示，保留實機限制及原失敗；程式／測試不變，最新 head 獨立覆核與 CI 另核（draft PR #145，merge 待確認）。

- 2026-10-03 C 段實機驗收與收尾（#145）：完整模式、歷史固定／回底、mouse／Shift、alt／normal、含 0x1D 的多行貼上、超限拒絕、多視窗唯讀／重取與尺寸已有截圖及自行比對紀錄。使用者要求後續改採自動化並清理殘留；修正控制提示重複，原版 count=2 反例及正向保留。本批固定 head／fresh verifier／CI 以 PR 結果核實，未 merge；[紀錄](gates/gate-11c-closeout.md)。

- 2026-10-03 全新 verifier r2 找到 resume 前歸屬讀取的測試缺口：舊回歸未拒絕 preread mutant。新增預設關閉的 fake producer replay 與真人工 item 回歸（21d68c9）：固定 runtime 通過、移除先讀永久歸屬則錯領人工 turn 並 exit 101；U17 12／testkit 115 passed。原失敗保留，最終新 head 另派全新 verifier／CI，尚未人工驗收或 merge（draft PR #145）。

- 2026-10-03 0.159.3 開放後的完整 acceptance 抓到兩個舊 U17 拒絕訊息斷言；真／fake 文案及 native／TUI consumer 已同步為已驗版本條件，保留 accept-r1／r2 原失敗。1bd0d6d 不列完整通過；修正新 head 待完整驗收、CI 與全新 verifier（draft PR #145）。


- 2026-10-03 使用者「同意」只開放 Codex CLI 0.159.3：實作 holder 啟動版本辨識與 migration 0006 的永久 thread 歸屬，未知／其他版本仍拒絕；live／reconcile／events 不因版本降級退回文字匹配。原 head 68e15c0 經全新 verifier 核實 892 passed／2 既有 ignored；新實作另驗，不增加真模型或 merge 授權（draft PR #145；[版本政策](gates/gate-11c-codex-input.md)）。


- 2026-10-03 使用者明確核准四回合真 Codex：0.159.3／gpt-6-luna／low 的 U17 首次通過，同 thread／holder、busy Queue、idle Send、重啟後 code word 與兩個獨立 durable receipts 有原始證據；第四回合只核 receipt，沒有最終回覆斷言。[live 證據](gates/gate-11c-u17-live-validation.md)；版本開放、全新 verifier 與人工驗收仍待完成（draft PR #145）。

- 2026-10-03 U17 live preflight：本機 Codex 0.159.3 自產 schema 的 turns/list 預設為 summary，診斷工具改明確要求 itemsView: full；client id 欄位形狀已核，未啟動 backend 或模型。真 live 仍等四回合 opt-in（draft PR #145）。

- 2026-10-03 本批 accept tui exit 0：597 主 suite passed／0 ignored，fake／真 daemon／完整 fake U17 三個 demos 通過。執行在新增 golden 前已完成 holder 階段；golden 後另跑完整 terminal_frames 8 passed，沒有把它加進 597。fmt、workspace clippy／最後 TUI clippy、實際 thumb no-std 及 linkcheck 通過。56dbb71 四個 CI jobs 均成功，新提交 CI 另核。（draft PR #145）。

- 2026-10-03 C 段矩陣收尾：三個真 producer App 延遲／倒序／舊 generation cases 通過，三個對應 mutants 各 exit 101，原 source 已逐 byte 還原。新增真 PTY holder／client frame golden，完整 frame suite 8 passed；[證據](gates/gate-11c-frame-order-validation.md)、[矩陣對照](gates/gate-11c-matrix-status.md)。完整 acceptance／新 head CI 另核，真 live／獨立／人工驗收仍待完成（draft PR #145）。

- 2026-10-03 完整 fake U17 已納入 accept tui；594 主 suite passed／0 ignored，fake／真 daemon／U17 三個 demos 通過。新互動 demo 的兩個 App 共用 parser、尺寸交接／唯讀拒絕／重取控制與各自 termios 還原通過；[互動 demo](gates/gate-11c-demo.md)。完整 workspace 888 passed／2 個既有 ignored 在抽取 fixture 前執行，真 Codex live 與完整驗收仍待完成（draft PR #145）。

- 2026-10-03 完整 fake U17：六個 tests 本機通過，含 App／client／daemon 子程序、同 holder／thread 重啟與草稿、scope／caller 拒絕及 durable turn id。attempted crash-window 原反例 exit 101，input-enabled scope 改要求自己的 clientId；五個 history 與十五個舊 driver 契約通過。真工具已編譯、guard exit 2，沒有 live 認證。[U17 證據](gates/gate-11c-u17-validation.md)、[live 工具](gates/gate-11c-u17-live.md)（draft PR #145）。

- 2026-10-03 C 段 U17 foundation／CI 反例（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：真 holder／wrapper／PTY／driver／SQLite 的兩個 fake Codex 情境通過；原程式把未嘗試送出的 queued row 誤認人工 receipt，已修正，完整 daemon／client／App U17 仍待完成。Retry 舊快照重建／覆蓋已受控重現並以原子條件更新修正；draw 依實際 backend 尺寸同步，新回歸拒絕缺同步 mutant。最新 CI 另核；[U17 範圍](gates/gate-11c-u17-validation.md)、[反例證據](gates/gate-11c-regression-validation.md)。

- 2026-10-03 C 段端到端時效（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：真 App 的 12 次最後 dirty burst 從通知 producer 前到外層可見皆 ≤300 ms，未加 holder round-trip 額度（最慢 215.302 ms）；800 ms 取樣 mutant 在 806.224 ms 被拒絕。還原後外層 6 passed／0 ignored，clippy／fmt／實際 no-std 通過；其餘矩陣與 U17 待完成。[證據](gates/gate-11c-outer-validation.md)。

- 2026-10-03 C 段真外層 PTY（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：真 App event capture／kernel resize、多視窗、鍵鼠／paste／歷史、正常與 unwind 還原及 20 次程序 fd 清理通過；完整 agend 214 passed／0 ignored，clippy／fmt／實際 no-std 通過。`50851e2` 四個 CI jobs 成功；主 suite 計數已排除 filtered 子程序 probe 重複輸出。其餘矩陣、U17 與完整驗收待完成；[證據](gates/gate-11c-outer-validation.md)。

- 2026-10-03 C 段真 PTY App（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：兩個完整情境經真 daemon／holder 到 raw consumer，逐 byte 核鍵鼠／paste、stty 核 resize；多視窗、歷史／淘汰、alt、同 holder 重啟及 20 次 thread／fd 清理通過。mode mutant 被同一回歸拒絕，原失敗保留；外層 capture／restore、其餘矩陣與 Codex U17 待完成；[證據](gates/gate-11c-native-app-validation.md)。

- 2026-10-03 C 段原生 renderer（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：補五種底線／色彩與游標輸出；真 backend＋parser 讀回通過，單欄寬字 resize hang／輸入越界修正有負面回歸。TUI 81／holder 57、workspace 865 passed／2 個既有 ignored、accept tui 572 passed／0 ignored，clippy／fmt／實際 no-std 通過；前 head `5b49df9` 四個 CI jobs 成功，新 head 另核。其餘矩陣、Codex U17 與完整驗收待完成；[證據](gates/gate-11c-native-validation.md)。

- 2026-10-03 C 段 App（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：接通完整模式、尺寸確認／控制失效、鍵鼠／貼上、固定歷史與唯讀 live-grid 跟隨；15 個真 parser App cases 通過，TUI 79／holder 56 passed，真 daemon TUI 2 cases 通過。原生 renderer 細節、其餘矩陣／U17 及完整驗收待完成；[局部證據](gates/gate-11c-app-validation.md)。

- 2026-10-03 C 段 TUI Source（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：新增獨立 full-terminal reader／writer、有界 queue／回覆與 frame mailbox；5 個真 parser／socket 測試與真 daemon／native PTY Source 通過，TUI 63 passed；App 尚未接通。完整 renderer、鍵鼠／貼上／U17 與完整驗收仍待完成。

- 2026-10-03 C 段 fake／真終端契約（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：新增 core TerminalProducer port，fake 注入真正 holder parser 後提供 1.4，CLP-23–28 同跑 fake／native daemon／holder／PTY；generation／停止、操作阻塞與 20 次 fd 清理有回歸。TUI／鍵鼠／貼上／U17、完整 verifier／人工驗收仍待完成。

- 2026-10-03 C 段 daemon 多視窗（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：每個 instance 有界佇列、socket-scoped view／attach、最後 Acquire 控制、EOF／停止清理、舊版輸入防繞過、holder 共用 50 ms 畫面取樣及 dirty 通知已接通。真 daemon 選 1.4，fake 暫留 1.3；8 個 native 多視窗 cases 通過，原背壓清理／停止 owner 反例保留。TUI／fake 全套 C 契約／Codex U17 與完整驗收仍待完成。

- 2026-10-02 C 段 client 型別／傳輸（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：1.4 檢視／控制／完成回覆、request id、8 MiB bounded reader、1 MiB 整次拒絕、原生 socket write 期限與半關閉 EOF 修正；NEEDED 仍 1.3。daemon 完整路徑尚未接通，現階段只協商 1.3；TUI／U17、完整獨立與人工驗收仍待完成。


- 2026-10-02 C 段控制／runtime（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：holder 實際 resize／input ack、FIFO 交接與 5 秒 native write；runtime 能力／連線 epoch、背景配對、取消 grant 失效、8 MiB bounded reader。holder 53 passed、完整 daemon crate 與 16 個真程序回歸通過；原失敗保留。daemon／client／TUI 與 U17 仍待完成，未獨立／人工驗收或 merge；[實作進度](gates/gate-11c-progress.md)。


- 2026-10-02 C 段第一個實作提交 `a13d31c`（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：holder 1.1 的結構化 frame、request id、generation／revision、色彩／游標／mode、歷史 viewport 與 8 MiB 整份拒絕。holder 47 passed；accept core 含 workspace clippy／實際 no-std 通過，兩個既有 deep explorers ignored。完整 C 段與 U17 仍在實作，尚未獨立／人工驗收或 merge。

- 2026-10-02 C 段提案 #144 已合併（`139fea5`）；在 專屬實作 worktree、`feat/gate-11c-terminal` 開始實作 P1–P6。基線 check-deps 含實際 no-std 通過；完整功能、U17、獨立驗證與人工驗收仍待完成。

- 2026-10-02 使用者逐項確認 Gate 11 C 段 P1–P6，記為 [D39](decisions/d39.md)，另明確授權合併 [#144](https://github.com/suzuke/AgEnD/pull/144)。本提交只記錄確認並同步狀態；最新 verifier／CI 通過後合併，C 段尚未實作或驗收。

- 2026-10-02 依使用者「繼續往下推進」開 `docs/gate-11c-proposal` 專屬 worktree（[draft PR #144](https://github.com/suzuke/AgEnD/pull/144)），整理 C 段 P1–P6：完整畫面、holder frame／協商、控制／resize、mouse／paste／歷史、Codex U17 與驗收矩陣。只改文件，待全新 verifier、CI 與使用者確認；尚未實作或 merge。

- 2026-10-02 文件 verifier r18 REFUTED `044f36e`：363 個非 Markdown entries 與 `430478d` 的 blob／mode 完全相同，但四份 crate README 與名詞表六列仍有未標歷史的「待驗收」舊狀態；已同步完成狀態，產品碼未改。最終文件驗證與 CI 見 #143，原報告 `g10-r18-report.md` 保留。

- 2026-10-02 第 10 施工關完成驗收，使用者確認 merge（#143）：`430478d` 全新 verifier r17 CONFIRMED，完整 accept 718 passed／2 ignored，實際 no-std 與四個 Ubuntu／macOS CI job 通過；人工事件補驗確認完整 stage、一次正確 Approve resolution、單次 Git merge 及 teardown。舊反例與 frozen-head 紀錄保留；收尾文件與最新 CI 結果見 PR。

- 2026-10-02 verifier r16 REFUTED `eac09cf`：六項完整 baseline（workspace 774 passed／2 ignored、accept 716 passed／2 ignored）與四個 CI 全綠，但真 SQLite post-CAS note 故障留下 durable approval、resolved unknown 與停住的 merge，重啟才恢復一次。attention_reason 清除改與 CAS 同交易，fake／SQLite 契約及真程序反例回歸補齊；原失敗 log 保留，修正待全新驗證及人工補驗，未 merge（#143）。

- 2026-10-02 使用者完成 `45e957e` 的 11 步人工主流程；main 防護、WIP patch、checks 中重啟與沙箱拒絕通過，home／repo／原 holder 清理完成。發現 watch 缺 stage、timeout 核准項目重現與 resolved unknown；兩個真程序回歸已在原版本重現 exit 101，修正與新一輪驗證進行中，待補驗及確認，未 merge（#143；[人工紀錄](gates/gate-10-manual-record.md)）。

- 2026-10-02 `ea975c3` 的 macOS PR CI（job 110607655251）在 content-filter fixture 的準備斷言失敗；有效 stat cache 下普通 git add 沒套用新 filter。已確定性重現，改真 Git --renormalize 強制建立轉換後 blob，六個回歸與 clippy 通過；原 CI／101 log 保留，產品碼不變，待最新 CI（draft PR #143）。
- 2026-10-02 文件 verifier r13 在 `ea975c3` 找到索引的 target／PATH 與人工驗收頁不一致（REFUTED）；統一 CARGO_TARGET_DIR 與 binary PATH，實際初始化選到 `agend 0.0.0`。其餘文件、證據與產品 tree 核對通過，產品碼不變；人工驗收及 merge 仍待使用者（draft PR #143）。
- 2026-10-02 全新 verifier r12 CONFIRMED `dfe5bc6`：完整 workspace 771 passed／2 ignored、accept 713 passed／2 ignored與三組組合探測通過；原 PATH／等待 fixture 失敗保留。push／PR 共四個 Ubuntu／macOS CI job 成功。README、ROADMAP 與 Gate 10 驗收文件已同步／分頁，待人工驗收、未 merge（draft PR #143；[證據](gates/gate-10-verification.md)）。

完整紀錄見 [施工路線圖進度紀錄](roadmap-progress.md)。

- 2026-10-03：C 收尾 verifier 指出名詞表／core／client 狀態殘留，已同步；74fc554 的 macOS CI 抓到 CLP-26 將 GetFleet 誤當輸入完成 fence，契約改以獨立 PTY consumer 判定 EOF release，保留原失敗 log 並重驗（draft PR #145；未 merge）。

- 2026-10-03：C 收尾補同步 holder README／TESTING 的首次 U17 狀態，實機證據路徑改指向已核 hash 的封存包；本批只改文件，沿用 c834bfc 的獨立實作驗證並核最後提交 CI（draft PR #145；merge 待使用者確認）。

- 2026-10-03：全新文件覆核指出早期 U17／App 驗證頁混用歷史與現況，已保留原批次失敗及數字並標明當時範圍，施工關／progress 現況對齊已核首次 U17、0.159.3 許可與自動化收尾；本批只改文件（draft PR #145；merge 待確認）。

- 2026-10-04：依使用者確認合併 #138（`4390633`），於 `feat/gate-12a-claude` 開工；首批加入 client 單次請求 API 與 native socket 回歸，Claude 接入及第 12A 驗收尚未完成。

- 2026-10-04：首批 `7b1baeb` fresh verifier REFUTED：大請求預編碼超出 deadline，且狀態入口未同步；保留反例與原結果，補編碼大小／期限限制及入口狀態。修正版另驗（draft PR #147）。
- 2026-10-04：`7b1baeb` CI：Ubuntu 通過，macOS 的共用期限測試在 hello 階段提前逾時；放寬握手排程餘裕並保留「重設期限會錯誤成功」的反例檢查，修正版重新跑 CI（#147）。
- 2026-10-04：第二位 fresh verifier 對 `e068e59` 給 REFUTED：合法上限內的 plain 字串編碼與大型回覆解析仍有 CPU 逾時；保留失敗斷言，改為分段原生 JSON 編碼及解析期間檢查期限，修正版另驗（#147）。
- 2026-10-04：第三位 fresh verifier 對 `2d754bb` 給 REFUTED：internally-tagged 回覆讀完後轉換中間樹，公開 native API 兩次超過期限 100 ms 餘裕；保留原反例，改成 RawValue envelope + core 資料分段解析，合法欄位順序亦驗，修正版另核（#147）。
- 2026-10-04：`b5629be` 的覆核仍 REFUTED：Fleet 中 AskEntry 的大量 options 轉換尾段六次超過同一 100 ms 餘裕；改為巢狀 ask／entry／reply 分段解碼並補真 producer 回歸。另一次平台中斷記未完成，原證據保留，修正版另驗（#147）。
