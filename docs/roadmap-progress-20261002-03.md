# 施工路線圖：2026-10-02–03 歷史紀錄

> **TL;DR**
> - 保留原批次進度、反例與當時驗收／merge 狀態。
> - 目前狀態以 [ROADMAP](ROADMAP.md) 的狀態欄為準。
> - 下一步：依目前施工關頁與 ROADMAP 執行。

## 進度紀錄

- 2026-10-03 使用者授權整理第 12A 提案 #138：對齊 #146 合併後 v2、P1 協定版本與 P7 migration 現況，拆分 P1–P10、歷史 F1–F10 與驗收計畫；全部決策仍待確認，未授權提案 merge 或實作。

- 2026-10-03 使用者明確確認「確認合併145」，#145 已以 merge commit `b2152db` 合併進 `v2`。最終 head `cfee027` 經全新 r9 CONFIRMED；四個 PR／push Ubuntu／macOS CI jobs 各 900 passed／0 failed／2 既有 ignored，實際 no-std 通過。合併 tree 與已驗證 head 完全相同。C 段實作／驗收完成，原實機、反證、真 U17 限制及清理證據保留；[收尾紀錄](gates/gate-11c-closeout.md)。

- 2026-10-03 獨立文件覆核補同步 daemon／testkit 的目前終端許可、Codex 歸屬快照／保留規則及 U17 12-case 索引，D39 加使用者後續自動驗證方式；B 段舊 wire 規則明示歷史範圍。程式／測試不變，新固定 head 另核 verifier／CI，#145 merge 待確認。

- 2026-10-03 全新文件 verifier 找到輸入政策、驗收計畫、demo 與 crate 入口仍要求逐步人工驗收；同步使用者已授權的剩餘自動驗證方式，歷史批次加範圍標示，保留實機限制及原失敗；程式／測試不變，最新 head 獨立覆核與 CI 另核（draft PR #145，merge 待確認）。

- 2026-10-03 C 段實機驗收與收尾（#145）：完整模式、歷史固定／回底、mouse／Shift、alt／normal、含 0x1D 的多行貼上、超限拒絕、多視窗唯讀／重取與尺寸已有截圖及自行比對紀錄。使用者要求後續改採自動化並清理殘留；修正控制提示重複，原版 count=2 反例及正向保留。本批固定 head／fresh verifier／CI 以 PR 結果核實，未 merge；[紀錄](gates/gate-11c-closeout.md)。

- 2026-10-03 全新 verifier r2 找到 resume 前歸屬讀取的測試缺口：舊回歸未拒絕 preread mutant。新增預設關閉的 fake producer replay 與真人工 item 回歸（21d68c9）：固定 runtime 通過、移除先讀永久歸屬則錯領人工 turn 並 exit 101；U17 12／testkit 115 passed。原失敗保留，最終新 head 另派全新 verifier／CI，尚未人工驗收或 merge（draft PR #145）。

- 2026-10-03 0.159.3 開放後的完整 acceptance 抓到兩個舊 U17 拒絕訊息斷言；真／fake 文案及 native／TUI consumer 已同步為已驗版本條件，保留 accept-r1／r2 原失敗。1bd0d6d 不列完整通過；修正新 head 待完整驗收、CI 與全新 verifier（draft PR #145）。

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

- 2026-10-03：C 收尾 verifier 指出名詞表／core／client 狀態殘留，已同步；74fc554 的 macOS CI 抓到 CLP-26 將 GetFleet 誤當輸入完成 fence，契約改以獨立 PTY consumer 判定 EOF release，保留原失敗 log 並重驗（draft PR #145；未 merge）。

- 2026-10-03：C 收尾補同步 holder README／TESTING 的首次 U17 狀態，實機證據路徑改指向已核 hash 的封存包；本批只改文件，沿用 c834bfc 的獨立實作驗證並核最後提交 CI（draft PR #145；merge 待使用者確認）。

- 2026-10-03：全新文件覆核指出早期 U17／App 驗證頁混用歷史與現況，已保留原批次失敗及數字並標明當時範圍，施工關／progress 現況對齊已核首次 U17、0.159.3 許可與自動化收尾；本批只改文件（draft PR #145；merge 待確認）。
