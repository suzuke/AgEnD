# 第 11 施工關 C 段：實作進度

> **TL;DR**
> - [draft PR #145](https://github.com/suzuke/AgEnD/pull/145) 持續實作完整終端；目前接通 holder／runtime／daemon／client 路徑，C 段尚未完成或驗收。
> - 已接通 frame／歷史及實際 resize／input ack；client 1.4 型別／傳輸已加入；daemon 多視窗與六項 fake／真契約已加入；TUI App 已接完整模式、鍵鼠／貼上與歷史；原生底線／標準游標已有讀回證據；完整 fake U17 本機已通過；真 Codex 0.159.3 首次 U17 已通過；版本開放、獨立與人工驗收待完成。
> - 下一步：全新 verifier 核原始證據，版本開放等使用者確認，再做逐步人工驗收；merge 仍另行確認。

## 已實作

| 區塊 | 行為 | 驗證來源 |
|---|---|---|
| 共用型別／holder 1.1 | 結構化 cells／color／style／cursor／mode、generation／revision、viewport request id；1.0 wire 保留 | core／protocol；holder 真 PTY／socket |
| holder 歷史 | 同一個 alacritty parser、normal 1,000 列絕對 row id、固定 viewport／淘汰 clamp；alt 無歷史，resize reflow 重編 row id | `terminal_frames` 與 parser 比對 |
| frame 上限 | 含換行最多 8 MiB，超限整份拒絕；不截斷成功，holder 請求仍最多 1 MiB | holder oversized frame、runtime 邊界／partial／EOF |
| holder 控制 | Acquire／Resize／Input／Release 共用 PTY FIFO；實際 resize 加完整 frame 才 grant、實際 write／flush 才 ack；寫前驗 owner／generation | `server` 的 5 個 native control cases、writer barrier |
| 原生 PTY 壓力 | raw agent 不讀 stdin 時，整次 write 最多 5 秒；明示可能部分寫入、不重送，新 grant 等舊 write 結束 | native backpressure case |
| runtime 背景操作 | holder 能力／連線 epoch、request id 配對、有界佇列與 pending；失效請求不送新連線 | runtime unit、真 holder 的 `terminal_runtime` |
| 取消競態 | 即使 grant 回覆已到、consumer 未接收就取消，也作廢連線；取消唯讀查詢則不打斷控制 | 真 holder 的 unconsumed-grant／readonly cancellation cases |
| client 1.4 傳輸 | 專用 reader／Sender，保留 request id；新行上限／完整請求拒絕／5 秒 write，失敗關閉、不重連重送；NEEDED 仍 1.3 | `full_terminal` 真 socket／holder parser；legacy client 回歸 |
| daemon 多視窗 | view／attach 綁 socket；每 instance 64 個有序操作、最後 Acquire 控制、舊 token／foreign view 拒絕；EOF／停止清理，保留尺寸 | `terminal_hub` 的 8 個真 daemon／holder／PTY cases |
| 畫面更新 | 同一 parser 的 grid／palette／mode 共用 50 ms 取樣，runtime dirty watch；各 view 的歷史選取獨立，最後 dirty 送出 | 真 parser 的精確 49／50 ms 邊界與 native 歷史／burst case |
| 開發中能力邊界 | 真 daemon 選 1.4；fake 預設 1.3，注入 TerminalProducer 後選 1.4；一般 NEEDED 保留 1.3，舊 peer 仍可用 B 路徑 | native 全路徑＋1.3 真／假能力拒絕＋CLP |

## 驗證紀錄

- 2026-10-03 使用者明確核准四回合真 Codex：0.159.3／gpt-6-luna／low 的 U17 首次通過，同 thread／holder、busy Queue、idle Send、重啟後 code word 與兩個獨立 durable receipts 有原始證據；第四回合只核 receipt，沒有最終回覆斷言。[live 證據](gate-11c-u17-live-validation.md)；版本開放、全新 verifier 與人工驗收仍待完成（draft PR #145）。

- 2026-10-03 U17 live preflight：本機 Codex 0.159.3 自產 schema 的 turns/list 預設為 summary，診斷工具改明確要求 itemsView: full；client id 欄位形狀已核，未啟動 backend 或模型。真 live 仍等四回合 opt-in（draft PR #145）。

- 2026-10-03 本批 accept tui exit 0：597 主 suite passed／0 ignored，fake／真 daemon／完整 fake U17 三個 demos 通過。執行在新增 golden 前已完成 holder 階段；golden 後另跑完整 terminal_frames 8 passed，沒有把它加進 597。fmt、workspace clippy／最後 TUI clippy、實際 thumb no-std 及 linkcheck 通過。56dbb71 四個 CI jobs 均成功，新提交 CI 另核。（draft PR #145）。

- 2026-10-03 C 段矩陣收尾：三個真 producer App 延遲／倒序／舊 generation cases 通過，三個對應 mutants 各 exit 101，原 source 已逐 byte 還原。新增真 PTY holder／client frame golden，完整 frame suite 8 passed；[證據](gate-11c-frame-order-validation.md)、[矩陣對照](gate-11c-matrix-status.md)。完整 acceptance／新 head CI 另核，真 live／獨立／人工驗收仍待完成（draft PR #145）。

- 2026-10-03 完整 fake U17 已納入 accept tui；594 主 suite passed／0 ignored，fake／真 daemon／U17 三個 demos 通過。新互動 demo 的兩個 App 共用 parser、尺寸交接／唯讀拒絕／重取控制與各自 termios 還原通過；[互動 demo](gate-11c-demo.md)。完整 workspace 888 passed／2 個既有 ignored 在抽取 fixture 前執行，真 Codex live 與完整驗收仍待完成（draft PR #145）。

- 2026-10-03 完整 fake U17：六個 tests 本機通過，含 App／client／daemon 子程序、同 holder／thread 重啟與草稿、scope／caller 拒絕及 durable turn id。attempted crash-window 原反例 exit 101，input-enabled scope 改要求自己的 clientId；五個 history 與十五個舊 driver 契約通過。真工具已編譯、guard exit 2，沒有 live 認證。[U17 證據](gate-11c-u17-validation.md)、[live 工具](gate-11c-u17-live.md)（draft PR #145）。

- 2026-10-03 C 段端到端時效（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：真 App 的 12 次最後 dirty burst 從通知 producer 前到外層可見皆 ≤300 ms，未加 holder round-trip 額度（最慢 215.302 ms）；800 ms 取樣 mutant 在 806.224 ms 被拒絕。還原後外層 6 passed／0 ignored，clippy／fmt／實際 no-std 通過；其餘矩陣與 U17 待完成。[證據](gate-11c-outer-validation.md)。

- 2026-10-03 C 段真外層 PTY（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：真 App event capture／kernel resize、多視窗、鍵鼠／paste／歷史、正常與 unwind 還原及 20 次程序 fd 清理通過；完整 agend 214 passed／0 ignored，clippy／fmt／實際 no-std 通過。`50851e2` 四個 CI jobs 成功；主 suite 計數已排除 filtered 子程序 probe 重複輸出。其餘矩陣、U17 與完整驗收待完成；[證據](gate-11c-outer-validation.md)。

- 2026-10-03 真 PTY App：兩個完整情境＋agent 入口共 3 tests 通過，核 raw bytes、實際 stty size、多視窗／EOF、固定歷史／淘汰、alt 與 daemon 重啟；20 次 close 每次 thread 為 0、fd 回同一基準。application-cursor mutant 同一回歸 exit 101，還原後通過；初跑 snapshot 同步錯誤保留。完整 agend 209／TUI 81 passed、0 ignored，workspace clippy／fmt／實際 no-std 通過。[原生證據](gate-11c-native-app-validation.md)。

- 2026-10-03 原生 renderer：五種底線形狀、色彩與六種標準游標經真 backend／第二個真 parser 讀回；TUI 81／holder 57 passed。單欄 resize 的寬字 reflow hang／新輸入越界已修正，相同負面回歸會失敗；原 stack／logs 保留。完整 workspace 865 passed／2 個既有 ignored、accept tui 572 passed／0 ignored，fake／真 demos、clippy／fmt／實際 no-std 通過；[本批證據與邊界](gate-11c-native-validation.md)。
- `d804580` 的 Ubuntu push／PR 及 macOS PR 三個 jobs 通過，各 workspace 865 passed／2 ignored、實際 no-std 通過；macOS push 在既有重連測試失敗（兩秒 2 次）。原 log 保留；受控 loop 停頓會讓原斷言失敗，新 fixture 改核實際進展／500 ms 時間下界並保留 r；100 ms mutant 被拒絕。具體 CI 停頓仍無 tick 紀錄可核，新 head CI 另核；[證據](gate-11c-native-app-validation.md)。

- `5b49df9` 的 push／PR Ubuntu、macOS 四個 CI jobs 均成功；核對真 no-std 與完整 tests。原 macOS Source overflow 失敗 log 保留，最新 renderer head CI 另核。

- 2026-10-03 App 接通：完整模式／尺寸確認／控制失效、模式按鍵／滑鼠／整段貼上與固定歷史，15 個真 parser＋socket App cases 通過。唯讀畫面跟隨最後輸出，holder 支援 live grid 小 viewport 定位。TUI 79、holder 56 passed；真 daemon TUI 2 cases／fake demo 通過。完整 workspace 861 passed／2 個既有 ignored（退出重訂修正前），修正後 TUI、clippy／實際 no-std 通過；198 個文件 links／anchors 有效。原 fd／編譯／fixture 失敗保留，詳細證據及未完成範圍見 [App 局部驗證](gate-11c-app-validation.md)。

- `f999bbf` 的 Ubuntu push／PR CI 成功，macOS 兩個 jobs 在 Source overflow fixture 失敗（只送 53／51 次，尚未達 64 個上限）。改為真 consumer 收件同步、到 worker 關閉才 drain，本機同一 assertion 通過；原 CI log 保留，新 head CI 另核。


- `1134d51` CI：push Ubuntu 成功，PR Ubuntu 在原生背壓案例失敗（`grant raced ahead of the native blocked write`）；兩個 macOS job 最後核對仍在執行。舊 fixture 用 GetFleet 回覆當操作已開始的同步點，未證明 writer 真的收到；改由真 PTY consumer 讀第一 byte 後寫 marker，再關舊 scope。本機同一 8 個 cases 通過，仍須核新 CI，未把原失敗改算成功。

- 2026-10-03 TUI Source：新增 daemon 1.4 的獨立 reader／writer、16 個寫入與 64 個控制回覆上限、合併最新 frame；實際寫入在背景，不等待 PTY ack。5 個真 parser／socket 測試通過，涵蓋交接、viewport、舊能力、阻塞、EOF 不重送、20 次 thread／fd 清理；原 fd 清理時序失敗保留。真 daemon／holder／原生 PTY 的 Source 輸入與 thread 清理也通過；完整 TUI crate 63 passed，fmt／workspace clippy／實際 no-std 通過。App 尚未選用此路徑，完整 renderer／輸入模式仍待接通。

- 2026-10-03 本批收尾：testkit 全 crate 115 passed、fake／native 共 12 項 C 契約及 1.3 能力拒絕通過；fmt、workspace clippy、實際 no-std check-deps 與 229 個文件 links／anchors 通過。原始輸出與負面證據共 130 份 log／source 的 manifest 為 `SHA256SUMS-fake`；本批未做完整 TUI／U17 或 fresh-context verifier。

- 2026-10-03 fake 生命週期：重啟先關閉所有 scope，保留 producer generation 與最後尺寸，舊 view／token 不恢復。3 個完整終端、1 個 fd、2 個操作／capture 壓力測試通過；移除訂閱替換保護後，相同 socket 測試抓到 `old capture leaked after replacement`，還原後通過。新增重啟測試的型別拼寫編譯失敗已修正，原 log 保留。
- `2ba1498` 的 push Ubuntu／macOS 與 PR Ubuntu CI 通過；PR macOS 在既有 CLP-11 retry attention 測試失敗，原因仍待核實。已補失敗時的 attention／events 診斷，本機同一 client protocol suite 通過；不能據此宣稱 CI 全綠。

- 2026-10-03 fake 路徑：core TerminalProducer port 注入真 holder parser、1.4 有界 worker／回覆／合併 frame；CLP-23–28 同跑 fake／真程序共 12 cases 通過，六個 mutants 被拒絕。producer generation／failed、native socket 操作阻塞與 20 次 fd 清理回歸通過。原 fixture 編譯、誤將更新 frame 當控制 ack、未等 consumer 收件、未分類訂閱錯誤的失敗皆保留；仍未做 TUI／U17 或完整 C 驗收。

- `5caa49b` 的 push／PR Ubuntu、macOS 四個 CI jobs 在 CLI-8／CLI-31 的版本期待失敗：真 daemon 已選 1.4，表格仍固定 1.3。修正為依 fixture 核對精確版本；真／假完整 CLI 表本機通過，原四份 CI log 保留，最新 CI 另核。

- 2026-10-03 daemon 1.4：8 個 native `terminal_hub` cases 通過，涵蓋正式 client、控制交接、foreign view、EOF、尺寸保留、20 次開關、重啟、固定歷史、dirty 尾段、真 PTY 背壓及 entry service 停止。原生反例先失敗：write 失敗後 release 的 control_lost 誤清旁邊 view；abort actor 留下 completed grant。修正後相同 cases 通過。並行單 thread probe 的大寫入可與 server frame write 互卡，fixture 改依正式 client 併行讀寫；原 BrokenPipe log 保留。
- 本機 holder 55、daemon 143、client 27、testkit 109、既有 TUI 58 passed；CLP 9／1.3 能力邊界 1／terminal_runtime 5 passed。accept core 156 passed／2 個既有 ignored，實際 no-std 通過。21a39ab 的 macOS PR CI 有 EOF fixture 失敗；已保存原 log，修正 EOF 起算並重跑 13 個 client cases 通過，最新 CI 另核。以上不是完整 C 段驗收。

- 2026-10-02 client 型別／傳輸：client 27 passed（13 個 native full-terminal cases）、testkit 109 passed、真程序 CLP 9／能力邊界 1／terminal_runtime 5 passed，既有 TUI 58 passed；accept core 156 passed／2 個既有 ignored，workspace clippy／實際 no-std 通過。macOS partial-EOF 在原 SHUT_RDWR 邏輯失敗，修正後同一回歸通過；能力列表排序及 fixture 編譯／clippy 的原失敗保留。這些仍是局部證據；daemon／TUI／U17 尚待接通。

- `a13d31c`：holder frame／歷史基礎 47 passed，`accept core` 通過；`24d107a` 記錄結果。該 head 的 push／PR CI 已通過。
- 2026-10-02 控制與 runtime：holder 53 passed；完整 daemon crate 測試通過。真程序 holder_process 7、holder_runtime 4、terminal_runtime 5 passed；runtime units 17 passed，含 legacy write lock 壓力。client protocol 9 與 TUI 2 個回歸通過；初跑缺 fake_codex example 的失敗保留，補建後重跑成功。最新 workspace clippy／實際 no-std 與 accept core 通過，兩個既有 deep explorers ignored，沒有 SKIPPED；結果記於 #145；C 段尚未獨立／人工驗收。
- 原取消邏輯在真 holder 回歸失敗：`an unconsumed grant survived cancellation`（exit 101）；修正後同一測試通過。
- 大行讀取原失敗與 partial fixture 的小 socket buffer 死鎖保留；最新讀取採 chunk 線性掃描、deadline poll／recv，關閉後仍讀完資料。

本機原始輸出與 SHA256 在 `/private/tmp/g11c-implementation-logs`；失敗 log 保留，不算通過證據。原生 macOS 測試不代表 Linux 或實際終端字型／游標外觀已驗收，雙平台 CI 與人工驗收仍須核最新實作 head。

## U17 foundation 與 CI 反例（2026-10-03）

- 真 holder／wrapper／PTY、fake frontend／app-server、driver／SQLite 的兩個情境通過，抓到未嘗試送出的 queued row 被人工文字誤判 confirmed；history 修正與既有送達契約通過。[範圍與剩餘歧義](gate-11c-u17-validation.md)。
- CLP-11 Retry 舊 attention 快照重建／覆蓋已受控重現；真 Fleet 原子條件更新後回歸及真 protocol suite 通過。最新 macOS 外層 PTY CI 又指出重新 Acquire 使用舊尺寸；draw 同步實際 backend area，新回歸拒絕缺同步 mutant。[證據](gate-11c-regression-validation.md)。
- 完整 U17、其餘矩陣、最新 CI、獨立與人工驗收仍待完成；本批不代表整段完成。

## 尚待完成

- 自動矩陣已有逐列 source／證據索引，包含最後補齊的 App 延遲 frame／query／舊 generation 與真 producer golden；最新 head CI 及全新 verifier 仍須重跑，不把既有六項契約當整份認證。[矩陣對照](gate-11c-matrix-status.md)。
- 原生底線／標準游標、mode-aware keys／mouse／paste、歷史、真外層 capture／restore 與資源清理已有自動證據；實際 Terminal／iTerm2／Linux 外觀與非美式鍵盤仍待人工驗收。
- 完整 fake U17 與真 Codex 0.159.3 首次 U17 已通過；[live 證據與限制](gate-11c-u17-live-validation.md) 待全新 verifier 核對。
- 真 smoke 通過並經使用者確認後，才開放已驗版本；目前正式 Codex 輸入仍 not_supported。
- 最新完整 acceptance／雙平台 CI、全新無 context verifier、逐步人工驗收與 merge 確認。

## 下一步

依 [矩陣執行狀態](gate-11c-matrix-status.md) 完成剩餘門檻，首次真 U17 已有明確核准及通過紀錄，版本開放等使用者確認。此頁的局部通過不能代替 C 段完成驗收。
