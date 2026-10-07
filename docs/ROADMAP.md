# 施工路線圖：13 個施工關

> **TL;DR**
> - 依 crate 由下往上分 13 個施工關；每個施工關單獨驗收，使用者確認後才開下一個施工關（D22）。
> - 目前狀態：**第 1–11 施工關完成並已合併**；第 12A Claude 完整真模型 smoke、獨立覆核及 CI 通過，#154 已合併。12B OpenCode #155 已合併並清理；12C GitHub forge 原生 pipeline 已實作，嚴格 base 保護策略已選定待接入；12D Telegram 手機操作、G4、真 forum 分流及原生驗收通過，覆核／合併收尾中，第 13 施工關未開始。
> - 下一步：依持續授權完成 [12D Telegram](gates/gate-12d-telegram.md) 合併收尾，再整合 12C 嚴格分支保護與真測。

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
| 12 `adapters` | [實作中（A／B 已合併；C／D 實作與驗證中）](gates/gate-12-adapters.md) | A claude、B opencode driver、C forge github、D Telegram | 先對假實作，再做真 backend smoke test |
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

第 1–11 施工關及 12A／12B 已完成並合併。#155 已合併為 `2a02fda`，自有 worktree／暫存已清理；12C／12D 實作與驗證中。使用者已授權持續完成第 12 施工關，範圍見 [AGENTS.md](../AGENTS.md#第-12-施工關持續授權2026-10-07)。

## 進度紀錄

- 2026-10-06：#154 固定 `1af2a31` 的 v7 另行授權後、四 CI checks 通過才執行一次；兩個初始 idle／六 keys／兩則 channel ACK 通過，但 Bash 專用 `type -P` 在實際工具回報 bad option，空路徑檔使完整 smoke FAILED。自有 holders／home／session 暫存清理，trust entries 依使用者指示保留；[原始失敗與有限範圍](gates/gate-12a-observed-smoke-v7.md)。修正 shell 探測／保存原始 guard 觀察與雙 shell 零模型契約，未重跑模型，獨立覆核及新 head CI 另核。

- 2026-10-06：使用者確認 #153 合併為 `0288824`，tree 與審閱版本相同；author／fresh verifier worktree、branch、targets 已清理。v6 初始 idle／唯一 ACK 有限證據獨立確認，完整 smoke 仍 FAILED；兩筆 account trust entries 依使用者指示保留。本批[smoke 指令修正](gates/gate-12a-smoke-contract.md)改用唯讀 help 測試既有 gh 防護並核原生 audit，零模型回歸通過，新固定計畫／覆核／CI 待核。

- 2026-10-04：使用者確認 #149，`c7e398c` 全新 verifier r2 CONFIRMED、push／PR 雙平台 CI 通過，合併為 `6dd552e`；使用者另重驗 16 native cases 全過。已清理 feature／verifier worktree、branches、targets；另移除已合併且乾淨的舊 `docs/gate-12-proposal` worktree／branch。有未提交變更的舊 worktree 保留。
- 2026-10-04：下一批在 `feat/gate-12a-gh-shim` 完成 D40 P4 的 [共用 gh 防護](gates/gate-12a-gh-shim.md)，首輪 3 unit／5 native cases 與 workspace clippy 通過；完整驗證、fresh verifier 與 CI 另核，未 merge（`d4853ad`／[draft PR #150](https://github.com/suzuke/AgEnD/pull/150)），完整 A 段仍未完成。

- 2026-10-04：使用者確認 #147，合併為 `8dfccf8`；`5a4047c` 全新 verifier CONFIRMED、46 client tests 通過，push／PR 雙平台 CI 通過。PR macOS 首次未改動的 TUI 時序測試超過 300 ms，原失敗保留，重跑通過且門檻未改。舊 worktree／branch 已清理；在 `feat/gate-12a-claude-store` 開始 [投遞／ACK／retention 基礎](gates/gate-12a-store.md)，Claude 接入尚未完成。

以下是各批次**當時**的進度原紀錄；其中「draft」「待驗證」「尚未 merge」只描述該批次，不是目前狀態。目前以頁首完成狀態及最新合併紀錄為準。原失敗與驗證範圍不改寫成成功；歷史證據僅列封存檔名，不公布本機暫存位置。

- 2026-10-04 第 12A P1–P10 設計確認記為 [D40](decisions/d40.md)（[draft PR #138](https://github.com/suzuke/AgEnD/pull/138)）：閒置 channel／忙碌 Stop、明確 agend_ack、P3／P4／P5＝A；剩餘依建議。只改文件，最新 head 全新 verifier／CI 另核；未 merge、實作或新增真模型回合。


- 2026-10-03 使用者「同意」只開放 Codex CLI 0.159.3：實作 holder 啟動版本辨識與 migration 0006 的永久 thread 歸屬，未知／其他版本仍拒絕；live／reconcile／events 不因版本降級退回文字匹配。原 head 68e15c0 經全新 verifier 核實 892 passed／2 既有 ignored；新實作另驗，不增加真模型或 merge 授權（draft PR #145；[版本政策](gates/gate-11c-codex-input.md)）。


- 2026-10-03 使用者明確核准四回合真 Codex：0.159.3／gpt-6-luna／low 的 U17 首次通過，同 thread／holder、busy Queue、idle Send、重啟後 code word 與兩個獨立 durable receipts 有原始證據；第四回合只核 receipt，沒有最終回覆斷言。[live 證據](gates/gate-11c-u17-live-validation.md)；版本開放、全新 verifier 與人工驗收仍待完成（draft PR #145）。


- 2026-10-02 C 段控制／runtime（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：holder 實際 resize／input ack、FIFO 交接與 5 秒 native write；runtime 能力／連線 epoch、背景配對、取消 grant 失效、8 MiB bounded reader。holder 53 passed、完整 daemon crate 與 16 個真程序回歸通過；原失敗保留。daemon／client／TUI 與 U17 仍待完成，未獨立／人工驗收或 merge；[實作進度](gates/gate-11c-progress.md)。


- 2026-10-02 C 段第一個實作提交 `a13d31c`（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：holder 1.1 的結構化 frame、request id、generation／revision、色彩／游標／mode、歷史 viewport 與 8 MiB 整份拒絕。holder 47 passed；accept core 含 workspace clippy／實際 no-std 通過，兩個既有 deep explorers ignored。完整 C 段與 U17 仍在實作，尚未獨立／人工驗收或 merge。

- 2026-10-07：12B 補已套用 POST／回覆遺失故障注入，三次 SQLite 重開不重送；chunked 超限統一分頁縮小。OpenCode 18 tests、clippy 與 check-deps 通過；這是 native producer 自動測試，非真模型網路故障驗證。

- 2026-10-07：12B 真 OpenCode 1.18.34 兩則權限蒐證通過，REST 無 SSE 取得請求、once 完成與 reject 拒絕；原始輸出納入回歸，自有程序／port／暫存已清，共享 auth 不變。尚未代表完整 12B 驗收。

- 2026-10-07：12B 三真 backend v1 因完整 workspace 被 CLI 截短而在送件前停止，零工作訊息且自有資源已清；獨立靜態覆核另找到 permission 驗證 GET 失敗會提前耗用 claim 的 P2，接續修正，未合併。

- 2026-10-07：12B 修正獨立覆核的 permission GET／claim 次序問題，原版本反例失敗、修正版與 lost-POST 重啟回歸通過；三真 backend v2 採短 namespace 與唯讀原 thread 狀態，尚未執行。

- 2026-10-07：12B 三真 backend v2 六方向 PASS，12 筆完整身分訊息全數 Confirmed；自有程序／暫存／session artifacts 已清，共享帳戶與 Claude trust 保留。permission P2 局部覆核已解決；進入完整覆核與 CI，尚未合併。

歷史紀錄見 [2026-10-02–03](roadmap-progress-20261002-03.md) 與 [較早紀錄](roadmap-progress.md)。

- 2026-10-04：依使用者確認合併 #138（`4390633`），於 `feat/gate-12a-claude` 開工；首批加入 client 單次請求 API 與 native socket 回歸，Claude 接入及第 12A 驗收尚未完成。

- 2026-10-04：首批 `7b1baeb` fresh verifier REFUTED：大請求預編碼超出 deadline，且狀態入口未同步；保留反例與原結果，補編碼大小／期限限制及入口狀態。修正版另驗（draft PR #147）。
- 2026-10-04：`7b1baeb` CI：Ubuntu 通過，macOS 的共用期限測試在 hello 階段提前逾時；放寬握手排程餘裕並保留「重設期限會錯誤成功」的反例檢查，修正版重新跑 CI（#147）。
- 2026-10-04：第二位 fresh verifier 對 `e068e59` 給 REFUTED：合法上限內的 plain 字串編碼與大型回覆解析仍有 CPU 逾時；保留失敗斷言，改為分段原生 JSON 編碼及解析期間檢查期限，修正版另驗（#147）。
- 2026-10-04：第三位 fresh verifier 對 `2d754bb` 給 REFUTED：internally-tagged 回覆讀完後轉換中間樹，公開 native API 兩次超過期限 100 ms 餘裕；保留原反例，改成 RawValue envelope + core 資料分段解析，合法欄位順序亦驗，修正版另核（#147）。
- 2026-10-04：`b5629be` 的覆核仍 REFUTED：Fleet 中 AskEntry 的大量 options 轉換尾段六次超過同一 100 ms 餘裕；改為巢狀 ask／entry／reply 分段解碼並補真 producer 回歸。另一次平台中斷記未完成，原證據保留，修正版另驗（#147）。
- 2026-10-04：`8594cba` fresh verifier CONFIRMED，46 client tests／10 獨立 native tests 通過，使用者重跑 46 tests 通過並清掉 754 MiB。PR CI 雙平台通過；push macOS 因 socket 先逾時而未回 InvalidData 的測試斷言失敗，拆出直接 CPU 解析檢查、接受 native I/O 逾時，原 80＋100 ms 不變，修正版另驗（#147；未 merge）。

- 2026-10-04：第 12A store 基礎加入 migration 0007、投遞前原子預約、四欄 ACK、結果不明與人工放棄、driver_events 14 天及未終結訊息保留。首輪真 SQLite／schema v1–v7、daemon 回歸及 accept core 通過；完整 workspace／全新 verifier／固定 head CI 另核，未 merge（`6193ea4`、[draft PR #148](https://github.com/suzuke/AgEnD/pull/148)）。

- 2026-10-04：`6193ea4` 全新 verifier REFUTED：event-only DB 快照被誤判空資料庫；保留失敗 log，修正 daily snapshot 判斷並保持原回歸，補 instance 移除／500 天後快照還原 ACK；daemon 162 passed／0 ignored、workspace clippy／fmt／實際 no-std 通過，另一位全新 verifier／CI 重驗（#148；未 merge）。

- 2026-10-04：`2af0473` 全新 verifier REFUTED：README 誤稱一般 messages 單表也算快照非空；獨立 native probe 核 Codex push／Claude inbox 皆不符。保留反例，修正文件並明示既有行為不變，第三位全新 verifier／固定 head CI 另核（#148；未 merge）。

- 2026-10-04：`4c5e76b` 全新 verifier REFUTED：名詞表仍稱事件表待新增與保留實作未變；保留反例，同步名詞表、0007／schema 入口與完整接入計畫，明示 store 已實作、runtime 尚待接入，修正版另由全新 verifier／CI 覆核（#148；未 merge）。

- 2026-10-04：使用者確認 #148，`8998f58` 經全新 verifier CONFIRMED、雙平台 push／PR CI 通過，合併為 `7877dbe`；merge tree 與驗證 head 相同，舊 worktree／branch 與編譯 target 已清理。接續在 `feat/gate-12a-claude-bridge` 實作 protocol 1.5、native channel／Stop helper 及 ACK spool；完整第 12A 未完成，見 [本批範圍](gates/gate-12a-bridge.md)。

- 2026-10-04：[draft PR #149](https://github.com/suzuke/AgEnD/pull/149)（首個提交 `999203e`）接通 protocol 1.5、channel／Stop helpers 與 hook／ACK spool；13 native cases、accept core（fmt／workspace clippy／protocol／實際 no-std）通過。完整 workspace、全新 verifier／固定 head CI 收尾中；未 merge，完整 Claude Driver 與真 CLI 驗收仍待完成。

- 2026-10-04：#149 原 head `b4c6b46` 被全新 verifier r1 判定 REFUTED（hook 發布／live RPC 解鎖空窗）；修正單次 flock、歷史 busy 撤銷 idle 與 holder 查詢 revision 核對，新增三個 native 回歸，16 cases／accept core 通過；完整 workspace、全新 r2 與 CI 收尾中。

- 2026-10-04：#150 gh shim fresh r1 `e281b19` **REFUTED**（GraphQL CR／block string 漏判、重複 approve 值與 fixture 完成競態）；修正與 native regression 已補，待全新 r2 及固定 head CI，未合併。

- 2026-10-05：#150 已依使用者確認合併為 `572dd73`（原 head `689aeeb`）；在 `feat/gate-12a-claude-driver` 接 Driver／設定 ownership／holder 1.2 單鍵控制。18 native bridge cases、10 Claude Driver／啟動設定 tests、workspace clippy／fmt／實際 no-std 通過；原自動放棄未送訊息反例已修正。完整 DRV 四次開機、啟動提示、清掃、真 CLI、全新 verifier 與 CI 仍待完成，未 merge，見[本批進度](gates/gate-12a-driver.md)。

- 2026-10-05：Driver 施工加入實際 Written 回條等待、Claude 精確 argv 孤兒清掃、結果不明的 `abandon` 人工入口及 ACK 事件 id 撞號修正。16 Claude 單元／19 native bridge／3 native process cases、daemon／holder 回歸通過；實際 no-std 通過。完整 DRV、啟動提示與真 CLI 尚未完成，workspace 及 fresh verifier 另核（`feat/gate-12a-claude-driver`，未提交／未 merge）。

- 2026-10-05：實際 Claude Driver 的 agent send／native channel／ACK 回歸通過；四個 daemon 程序不重送，另用新 HOME 證明獨立送達。accept core（含 protocol 相容性及實際 no-std）通過；workspace 首次在 retry 舊啟動參數斷言失敗，納入完整 D40 旗標後單例通過，完整重跑中（同工作分支，未提交／未 merge）。

- 2026-10-05：Driver 施工補通 pipeline 原 dispatch id 的 native ACK，通用回條只觀察、task 完成不代確認。完整 Git task／review／人工核准／single merge 經四個 daemon 通過；共用 DRV-1–9 十個案例經獨立 composition processes 通過，新 HOME 反向在 boot 2 失敗；真 Esc completion 遺失的四次開機不重送。最新 workspace 回歸收尾中；啟動提示與真 CLI 仍待授權及驗收（同工作分支，未提交／未 merge）。

- 2026-10-05：native Driver 檢查點 `afe5188` 已推送並建立 draft #151；統一 `demo adapters` 通過，workspace 結果為 992 個測試函式通過／0 failed／2 既有 ignored（子程序入口不算獨立行為證據）。實際 no-std、clippy、fmt 通過；全新 verifier 及 Ubuntu／macOS CI 進行中。12A 啟動提示、真 CLI 版本／PATH／ACK 驗收仍待完成，尚不可 merge。

- 2026-10-05：#151 的全新 checkpoint verifier 找出 `--` options terminator 使 owned 旗標落入 positional tail；原反例保留。拒絕 terminator 後，17 Driver 單元、整個 daemon crate、workspace clippy／fmt／實際 no-std 通過；修正版獨立驗證與 CI 待核。必要作者證據移至 AgEnD-ops，5 份被最新結果取代的成功 log 刪除，native fixture 目錄查無殘留；施工 target／worktree 保留，完整 12A 未完成。

- 2026-10-05：#151 的 `441d658` terminator 修正獲全新局部 verifier CONFIRMED；CI fixture 修正改等真 Driver idle 與同一 fleet view 的 approval attention。提早 hint 回歸與反向、17 Driver／3 process／27 bridge 入口、15 pipeline cases、clippy／fmt／實際 no-std 通過；新版 CI／fresh verifier 待核，完整 12A 仍未完成（[範圍](gates/gate-12a-driver.md)）。

- 2026-10-05：#151 固定 `cb0212c` 的全新 verifier 對 CI fixture 修正局部 CONFIRMED；native demo、15 pipeline、clippy／fmt／真 no-std 全過，idle／attention 空窗正反對照成立。自己的 verifier worktree／branch／target／程序／fixtures 已清理；雙平台 CI 仍執行中，P5／真 CLI 未完成，未 merge（[範圍](gates/gate-12a-driver.md)）。

- 2026-10-05：#151 的 `50e0e83` push／PR CI 均在 Ubuntu、macOS 通過；補[被動啟動畫面蒐證工具](gates/gate-12a-startup-capture.md)，用 native producer 驗兩種 PTY 寬度、零輸入與清理。真 CLI／P5／P6 仍待授權與完成，未 merge。

- 2026-10-05：蒐證工具的 fresh verifier 在 `4511e21` 用真 holder 重現跨 soft-wrap Bearer 前綴繞過 scan；依 native wrap 標記合併 logical line後補拒絕與電郵遮蔽回歸，修正後再交獨立核對。完整12A仍未完成（#151）。

- 2026-10-05：`356fbef` 的全新無相關 context verifier 局部 CONFIRMED 被動蒐證工具修正；原同一 native Bearer wrap 反例改為寫入前拒絕，5 cases、四種尺寸／寬字 spacer、零 stdin、成功／失敗清理、clippy／fmt／實際 no-std 通過。`4511e21` 的原 REFUTED 證據保留；只認證工具，真 CLI／P5／P6 未完成（#151）。

- 2026-10-05：另獲使用者授權，固定真 Claude 2.1.284 查版本及兩寬被動蒐證成功，0 模型回合／按鍵／訊息；保存完整信任 frame 為版本化 fixture，自有程序與暫存已清理。P5 自動按鍵、P6 初始 idle、後續真驗仍未完成；本批回歸與 fresh verifier 待核（#151）。

- 2026-10-05：為後續 P5/P6 真 fixture 準備受控 trust 蒐證模式，預設被動不變；兩寬及未知／外來／未切換選項的原生正反例通過。這是 operator 輸入蒐證，正式 daemon-key、後續提示與初始 idle 未認證，真按鍵待另行授權（#151）。

- 2026-10-05：受控 trust 工具 `b0d8074` 的全新 verifier 重現路徑前綴／畫面別處提及自有路徑會誤送 Down、Enter，原 REFUTED 證據保留。改成唯一 workspace 標頭下完整路徑相等並補兩寬原生拒絕回歸；修正版獨立核對待完成，未執行真按鍵，完整 12A 未完成（#151）。

- 2026-10-05：`7abb646` 路徑修正獲 fresh verifier 局部 CONFIRMED，push／PR 雙平台 CI 均通過。另獲授權後，Claude 2.1.284 兩寬受控 trust 蒐證各完成 Down／Enter，保存選到 Yes 與 development channels 真 fixture；未確認後續提示、0 模型／訊息，自有程序／暫存與兩筆個人 trust 條目已清理。原分類器漏掉 development channels 的反例已重現，補 StartupMenu 規則；本批 core／fresh 重驗待核，P5／P6 仍未完成（#151）。

- 2026-10-05：為取得 channels 確認後的真畫面，準備第三個獨立 opt-in 的受控蒐證模式；只在兩個 trust 回條後，對完整且唯一 `server:agend` 選單送一次 Enter，最多三次 operator Input。13 個 native cases、整個 daemon／fmt／clippy／實際 no-std 通過；全新 verifier 待核，真三鍵執行尚未授權，P5／P6 未完成（#151）。

- 2026-10-05：第三鍵蒐證工具 `28d6341` 的全新 verifier 重現三種矛盾／重複選單會誤確認，原 REFUTED 證據保留。改成整份已錄製畫面只忽略空白後相等，補兩寬拒絕回歸；14 native cases／整個 daemon／fmt／clippy／實際 no-std 通過，新全新 verifier 待核，真三鍵蒐證未執行（#151）。

- 2026-10-05：`8d605bf` 的新 fresh verifier 找到 trust 回條後外來 frame instance／view 仍可觸發第三鍵，原 REFUTED 保留。補 subscribe／acquire／outer frame 身分核對與兩寬 native proxy 回歸；新回歸先核原 consumer 失敗，15 native cases／整個 daemon／fmt／clippy／實際 no-std 通過，新全新 verifier 待核，真三鍵未執行（#151）。

- 2026-10-05：`01f438e` 的全新 verifier 重現 resize ACK 等待略過不一致 native frame 後仍送三鍵；原 REFUTED 保留。補兩寬 instance／view／generation／size 零 Input 回歸，原始與目標尺寸仍接受；新回歸先核原 consumer 失敗，修正後 16 native cases／整個 daemon／fmt／clippy／實際 no-std 通過，新全新 verifier 待核，真三鍵未執行（#151）。
- 2026-10-05：`433d2a8` 全新 verifier 確認 resize 身分回歸成立，但 trust 選單追加第二個 selected Exit 仍確認，原 REFUTED 保留。trust 改為兩寬完整真 No／Yes fixture 僅替換本次 canonical path 後比對，新增兩階段 extra／duplicate selection 拒絕回歸；17 native cases／整個 daemon／fmt／clippy／實際 no-std 通過，新全新 verifier 待核，真三鍵未執行（#151）。
- 2026-10-05：`dabb35e` fresh verifier／使用者 native 重驗及 push／PR 雙平台 CI 全通過。另獲三鍵蒐證授權後，固定 Claude 2.1.284 的 100 欄只完成一個 Down，畫面仍為 No；按計畫停於首個失敗，未重送／Enter／140 欄。0 模型／訊息，native 暫存已清理；真原因與 session 觀察缺口保留。補完整提示穩定等待與私有清理身分，native 延遲 receiver 原 consumer 失敗，19 native cases／整個 daemon／fmt／workspace clippy／實際 no-std 通過，新全新 verifier 待核（#151）。

- 2026-10-05：`e0cedfb` 的提示穩定等待與私有清理身分獲全新 verifier 局部 CONFIRMED，19 native capture／199 daemon 測試、獨立 32 native cases、fmt／workspace clippy／實際 no-std 及 push／PR 雙平台 CI 通過。其後新核准 ready 計畫的真 Claude 2.1.284 兩寬蒐證各完成 Down／trust Enter／development Enter，100／140 欄保存 13／11 frames 與主介面，0 模型 prompt／團隊訊息。另一位全新 verifier 核保留證據一致性與指定殘留目前不存在，有限範圍 CONFIRMED；7 個記憶體 mutation 均拒絕。原始失敗證據與清理歷史證明缺口保留，正式 P5／P6、先前 Down 原因及完整 12A 未認證（draft #151）。

- 2026-10-05：接在 `53f4ea6` 開 `feat/gate-12a-startup-gate`，實作[正式 P5／P6 啟動處理](gates/gate-12a-startup-runtime.md)：完整真 fixture、單鍵 revision CAS、schema v9 按鍵 intent 及 SessionStart＋Ready 初始 idle。每次開 worktree 前先核前批已結束資源清理，規則加入 AGENTS.md；native／core 重驗、fresh verifier 與 CI 待核，未 merge。
- 2026-10-05：draft #152 接在 #151 後；`1beb03a` 的全新 verifier 重現五秒到期至首次 poll 間漏看未知畫面的誤投遞，REFUTED 證據保留。改為首次 poll 成立前持續採樣，納入原 native 反例與恢復後穩定五秒才可投遞的正向；重新獨立驗證與 CI 待核，未 merge。
- 2026-10-05：正式 P5 與 private startup capture 隔離：啟動前持久登記 manual，停用自動鍵與初始 resize，raw PTY fixture 同樣明確登記；原失敗證據保留。既有 TUI 六例與兩寬已知 trust 被動零鍵回歸通過；另修正人工按鍵測試的空檔案競態，完整重驗與新 fresh verifier 待核（#152）。
- 2026-10-05：`d81a7f3` 第二位全新 verifier 重現 P5 背景取樣令人工 TUI 漏掉尾段 modes，兩次反例與單獨 dirty 因果正對照保留為 REFUTED。terminal hub 改在 frame request 起點核 sample 是否已跨 notice 50ms，未跨則保留 dirty 到下一輪；新增 auto sampler 原生回歸先核舊版失敗，修正後 outer PTY 七例及額外 auto 尾段輸出 300ms 回歸通過，獨立重驗待核（#152）。
- 2026-10-05：`26de55a` 本機完整 workspace／實際 no-std 及 Ubuntu CI 通過；macOS CI 分別失敗於 CLP-25 BrokenPipe 與 auto 尾段 309.403ms。第三位全新 verifier 用 4KiB socket buffer 重現原測試工具雙向阻塞；大型請求改為並行讀寫，同連線超限拒絕及後續真 PTY 輸入通過。P5 改一次完整 24 列查詢，人工及已錄製尺寸各 12 筆仍核 300ms；原失敗保留，新 head 獨立驗證與 CI 待核（#152）。
- 2026-10-05：`eda202c` Ubuntu CI 在新增 100×24 情境記錄 300.808ms，原證據保留；完整終端改每 50ms 讀取 frame mailbox，首頁及舊終端仍 100ms、舊重拿仍 200ms。真 outer PTY 八例、完整 TUI、workspace clippy／fmt／實際 no-std 通過，36 筆最後 dirty 更新皆 ≤300ms；新無 context verifier 與 CI 待核，未 merge（#152）。
- 2026-10-05：第四位全新 verifier 用 4KiB socket buffer 與合法 24KiB 分行 input 推翻 `eda202c` 的 >64KiB 並行門檻，固定 head 記為 REFUTED。testkit 改每份控制請求都並行讀寫，新增原生 consumer 精確收齊 24KiB 及後續 sentinel 回歸；原負例保留，最終 head 重驗與 CI 待核，未 merge（#152）。
- 2026-10-05：`bd85766` macOS CI 仍記錄 300.315／330.091ms，未認證時效。未知且 notice 未變的畫面減少無授權作用的完整取樣，輸出／link 變動立即重查、靜默 resize 最多一秒重查；terminal hub 從首筆 dirty 等 50ms 避開過期共用 frame，後續 notice 不延長等待。原 300ms 門檻不變，新 native／daemon／fresh／CI 待核（#152）。
- 2026-10-05：`1727c17` 的全新 verifier 本機 startup7／native4／outer8與36筆原300ms輸出通過，CI另在舊取消測試 first poll 必為Pending的假設失敗；真holder可以先回覆。測試改先核自有PID及已停止狀態，取消後RAII恢復，避免搶monitor退出回條；不改runtime，原CI證據保留，新head驗證待核（#152）。

- 2026-10-06：使用者確認 #151／#152 合併，分別為 `cac2226`／`e7a8987`；兩個合併 tree 與原驗證 head 相同，原 worktree／branch 已移除，編譯暫存僅保留約 51MiB 的兩個歷史稽核必要 binary。使用者要求完整模型 smoke 為必要驗收；本批在清理核對後建立獨立 worktree，真模型計畫準備中，尚未執行。

- 2026-10-06：完整真模型 smoke 腳本 `1256c99` 經全新 verifier 找出 native cleanup 的 foreign workspace 先停止、漏額外 holder、HOME 漂移及早期無 DB 殘留；已保留原負例，改為全面預核及環境綁定後重驗。仍未啟動真 Claude／模型。
- 2026-10-06：`2b6084d` 的全新 verifier 以兩個自有 native holder 重現控制 lock／socket 符號連結會誤停另一 home，原 REFUTED 保留；清理 helper 在讀 lock／連 socket 前全面核對控制路徑型態，模型命令改釘完整 Haiku ID，修正版重驗中，真模型尚未執行。
- 2026-10-06：`fb4801f` 全新 verifier 證明控制檔案硬連結仍能重導 Shutdown 到另一自有 native lab，原 REFUTED 保留；檔案／socket 追加單一 link ownership 核對，修正版重驗，零真模型執行。
- 2026-10-06：draft #153 固定 `3ac1b5d` 的 push／PR 雙平台四個 CI jobs 通過；取得完整計畫授權後真 Claude 2.1.284 執行一次，兩個 instance 各三個 production startup key 寫出及 SessionStart，仍無初始 idle，180 秒等待逾時停機，零測試 send／訊息／ACK，完整 smoke **FAILED**。另一位全新 verifier 核固定原始證據；發現漏 scratchpad 與 Ready `halted=1` audit 誤拒絕，補清後獨立核 49 paths／兩 trust keys／PID／PGID absent；未留 raw frames／transcripts／usage，不能推斷失敗畫面或精確 API 次數。

- 2026-10-06：#153 補只讀 startup frame 診斷（單 instance／90 秒／零工作訊息，真 CLI 尚未執行），修 smoke Ready audit 及 scratch namespace 清理。native producer 已驗 Unknown／Ready 原始 frame、只有三個 production keys、正常 Ready `halted=1`，以及含額外 bootstrap UUID 的 namespace 清除並私有保留內容；新計畫待全新 verifier 及另行授權，完整 smoke 未通過。

- 2026-10-06：#153 的固定診斷 v2 另行授權後執行一次：一個 instance、四份相同 raw frame、三鍵 written 與 live SessionStart、零工作訊息，結果 CAPTURED。全新 verifier 核原 classifier 因已錄製的 Try 提示變體而拒絕，footer 已涵蓋；補完整 `100x24-3` literal 與 native P6 回歸，不放寬未知畫面。本次自有程序、home、scratch、session 暫存與 trust key 已清理；完整 smoke 仍 FAILED，禁止自動重跑，未 merge。

- 2026-10-06：#153 的 `91bb2bc` push 雙平台 CI 通過，但 PR macOS 的既有終端尾段測試測得 301.540459ms，超 300ms；原失敗保存，不放寬門檻。Ready literal 修正與完整 native bridge／outer 回歸、全新 verifier 及新 head CI 待核。

- 2026-10-06：#153 準備下一份完整 smoke 的 observed runner：首次 idle 等待最多 28 份只讀 A／B frame，工作前重核 idle；後續工作不擷取。原七則訊息／900 秒／零重跑與 audit／cleanup 不變；固定新計畫待獨立核對及另行授權，沒有再次真 CLI 執行。

- 2026-10-06：#153 的完整 native bridge34／outer8及 accept core 通過；全新 verifier 核 Ready fixture byte 匯出、原 None→舊提示 Ready 反例、startup7／outer8及真 no-std通過。新增 observed 計畫的 tuple→JSON list scope mismatch 在任何 CLI 前被 verifier 推翻，保留原計畫與反例，修正為一致 list 後重新產生固定計畫；沒有再執行真 Claude。

- 2026-10-06：#153 固定 `990bead` observed v2 另行授權後執行一次，A idle／B unknown，初始 idle 逾時 FAILED；24 份 frame、683 次 status、零工作訊息。自有程序、13 精確路徑、兩 trust keys 已清理；補已錄製 how-does 完整 Ready literal 與 native P6 回歸。990 push CI 雙平台通過，PR macOS 尾段 301.612416ms>300ms 失敗保留；[本次紀錄](gates/gate-12a-observed-smoke-v2.md)，完整 12A 未驗收。

- 2026-10-06：#153 第二次完整 smoke 的全新 verifier 核固定990 A Ready／B None 與單換提示反例、11語意 mutations／5 native 負例、13paths／PID／PGID／兩trustkeys absent。how-does 新 literal 修正前 native P6 exit101，修正後 accept core、startup8及前後實際 check-deps 通過；編譯 target 清除，新改動待另一全新 verifier，未再次啟動真 CLI。

- 2026-10-06：#153 固定6a666ba的 observed v3 另行授權後執行一次，B terminal 未註冊使只讀 helper 回 no_terminal，約一秒內停止；僅 A 空白 frame、零 startup keys／SessionStart／工作訊息，完整 smoke FAILED。補清兩個早期 scratch namespaces，13 paths／5 PID／兩 trust keys absent。新增獨立 runner 有界等候首次註冊，其他錯誤及後續 terminal 消失立即停止；native daemon／shell 先重現舊 runner 拒絕，再核修正版兩 frame 與早期 scratch 清理，不啟動真 Claude。6a PR 雙平台通過，push macOS 306.841042ms 超過300ms，原 log 保留。

- 2026-10-06：#153 固定c7b756b／observed v5 已另行授權執行一次：24次只讀frame皆成功、707次status無idle、兩個live SessionStart與各三個production keys written；新fix-typecheck／fix-lint Try文字令Ready None，180秒初始idle逾時FAILED、零工作訊息。只改建議文字即可與既有Ready其餘tokens吻合；未放寬正式classifier、未重跑。自有13路徑／8PID／5PGID／兩trustkeys核absent，c7 push／PR雙平台四job通過；完整12A仍未驗收，下一步先確認Ready建議列是否可變，[紀錄](gates/gate-12a-observed-smoke-v5.md)。

- 2026-10-06：使用者確認「視為可變」，記 [D41](decisions/d41.md)：只放寬 Ready 唯一完整單行 Try 建議內容，其他完整畫面／版本／路徑／尺寸／位置、P5 選單及 P6 初始 idle 門檻不變；v5 原始失敗保留，修正與原生驗證中，#153 尚未 merge、完整12A未驗收。

- 2026-10-06：D41 作者修正完成，core 146 passed／2 既有 ignored、原生啟動10 passed、accept core／workspace fmt與clippy／協定golden／修改前後實際no-std通過；新真模型計畫尚未執行、完整12A未驗收。全新 verifier與新提交CI待核（#153）。

- 2026-10-06：#153 的 f1e1b70 Ready 可變提示修正經全新 verifier（1,046 cases、native10＋獨立10）、使用者重驗1,073 cases、accept core／前後 no-std 及 push／PR 雙平台四 CI jobs通過；作者與驗證編譯暫存／工作樹／程序已清理，未 merge。

- 2026-10-06：固定 f1e1b70／[observed v6](gates/gate-12a-observed-smoke-v6.md)另行授權執行一次：兩個初始 idle、六 startup keys及第一則 channel 明確 ACK 通過；模型依 CLAUDE.md 拒絕 gh pr merge 0 負例，第一段工作逾時 FAILED，互傳／queue／Interrupt 未執行。native cleanup 已移除 home／holders／session 暫存；兩個 trust entries 因外來 Claude 程序使 guard 拒絕寫入而暫留，獨立覆核與後續方案待核，沒有重跑。

- 2026-10-07：#154 固定 `81804dc`／[observed v8](gates/gate-12a-observed-smoke-v8.md) 經四個 CI checks 全通及另行授權單次執行；INITIAL confirmed、模型 peer sent，完整 smoke 因 startup gh token refusals 誤算進工作 audit 次數而 FAILED。自有 runtime／home／session 已清理、trust entries 保留；修 scripts 的 pre-INITIAL append-only prefix 邊界與原生雙 shell 反例，未改 Rust／防護政策，未重跑模型、未 merge。

- 2026-10-07：#154 的 `a85a91a`／未執行 v9 經全新 verifier 判 REFUTED：audit 文字讀取正規化 CRLF，漏掉 prefix bytes 改寫。原失敗保留，改逐 byte 保存／比對與 base64 失敗證據，雙 shell 各二十三個反例通過；沒有新增真模型執行，新固定計畫與獨立覆核待核。

- 2026-10-07：使用者授權持續完成第 12 施工關 A–D，保留獨立覆核、合併 CI 與每批清理。#154 固定 ab5a296 的 v10 單次真測已有五則 confirmed（四 channel、一 Stop），queue 後 active Stop 被設成 busy 而 idle 逾時，Interrupt 未驗；原證據與獨立 FAILED 覆核保留。補原生反例及 idle 修正，完整 12A 仍待真測。

- 2026-10-07：#154 的 v11 完成啟動與 INITIAL ACK，但模型將命令後說明送入 Bash，peer send exit 2；提前停止自有 runner 並清理，未進入 queue／Interrupt。命令改用獨立區塊，Bash／zsh 原生參數檢查通過；完整 12A 仍未完成，詳見 [Stop idle](gates/gate-12a-stop-idle.md)。

- 2026-10-07：#154 固定 `0f4b8e0` 的 v12 完整真模型 smoke 單次 PASS：七則 confirmed（六 channel／一 Stop）、模型互傳、queue 後恢復 idle、Interrupt 完成，自有清理成功且保留 trust。全新 outcome／CI 收尾中；[完整紀錄](gates/gate-12a-complete-smoke.md)。使用者表示 Telegram 測試憑證稍後提供，先推進其餘工作。
- 2026-10-07：依持續完成第 12 關授權，12B 在 `feat/g12b-opencode` 開始傳輸／session／歷史核對；daemon lib 102 tests、clippy 與前後實際 no-std 通過。真 OpenCode 1.18.34 的 noReply 身分捕獲已納入回歸，程序與隔離目錄已清；正式 Driver／holder／恢復／真模型測試仍待接，尚未 merge。12D Telegram 專用 bot 與 chat 資料由使用者稍後提供，不阻擋其他實作。

- 2026-10-07：12A 完整真測、全新覆核及固定 head 雙平台四 CI jobs 通過，#154 合併為 `a6cdb4c`；12A worktree／本機 branch 已刪。12B 已 rebase 至合併後版本，Telegram 專用測試設定已備齊（私有設定不入 repo），12B–D 持續實作。

- 2026-10-07：12B OpenCode 初版 Driver／supervisor worker 接線，持久 attempt、分頁 REST 核對與 unknown 狀態；108 個 daemon 單元測試通過，holder／permission／真測尚未完成（feat/g12b-opencode）。

- 2026-10-07：12B 權限 REST 核對與 schema v10 單次決策 attempt 持久化；worker 輪詢保存，attention／operator 接線仍待完成（feat/g12b-opencode）。

- 2026-10-07：12B permission attention 與 operator-only AnswerAsk 接線，完整 snapshot 再核對、持久單次回覆，pipeline refresh 保留 backend 權限提示；holder／真測尚未完成（feat/g12b-opencode）。

- 2026-10-07：12B 原生 daemon／holder 兩種恢復路徑（正常停止／SIGKILL）、permission 回覆與單次投遞整合通過；自有 lab 清除，真 CLI／模型及三 backend 驗收待完成（feat/g12b-opencode）。

- 2026-10-07：12B terminal event 回填、schema v11 去重與真 HTTP acceptance→Sent 接線；10 原生 OpenCode 案例／兩個 holder 恢復案例通過，完整 DRV／模型驗收未完成（feat/g12b-opencode）。

- 2026-10-07：12B 完整 DRV suite 10/10（原生 REST／重開 SQLite）通過；固定版本零 prompt provider inventory 完成並清理，接續受控模型真測（feat/g12b-opencode）。

- 2026-10-07：12B 固定 OpenCode 1.18.34／gpt-6-luna 正式 daemon 真模型單則 smoke 通過，原生歷史與 DB 確認單次投遞及完成；自有程序／port／目錄已清，共享 auth 不變。busy／跨 backend 與 fresh verifier 仍待完成。

- 2026-10-07：12B busy queue→interrupt 原生反例重現並修正：abort 回覆成功後單次提交，不等待可能不存在的 idle 空窗；新增 steer／interrupt 兩條路徑的完整歷史與無重送檢查。

- 2026-10-07：12B 真 OpenCode 1.18.34 busy smoke v2 通過三筆 Confirmed／首輪中斷／緊急回覆；保留 v1 過早 busy 斷言與 v2 精確範圍，原生歷史加入回歸，自有程序與暫存已清。

- 2026-10-07：12B 三 backend 六方向正式 daemon 路由驗證通過，六則均 Confirmed、內容完整且每 receiver 恰兩筆；Claude 使用實際 helper ACK／Stop。此為零模型 fixture 驗證，真模型互傳與完整覆核仍待完成。

- 2026-10-07：12B 獨立缺口覆核找出 unknown retention、全量歷史上限與人工終結三項不足；先補 schema 0012 投遞歸屬及未終結保留，未宣稱完整通過。

- 2026-10-07：12B unknown delivery attention／operator-only Abandon 已接線；原生兩次重啟、agent 拒絕、人工終結、晚到確認與零重送驗證通過。長 REST 歷史分頁仍待修正。

- 2026-10-07：12B 真 1.18.34 零模型捕獲兩頁游標與單筆歷史查詢；新增固定 endpoint 分頁 API 與 captured producer 回歸。worker 分頁接線仍待完成，原長歷史缺陷保持未完成。

- 2026-10-07：12B worker 已接最新頁／歷史回填與舊 attempt 定點查詢；超過 16 MiB 原生歷史回歸核舊收件、新派工與早期完成只發布一次，沒有提高傳輸上限。

- 2026-10-07：#155 固定 `e96f429` 全新 verifier CONFIRMED_SCOPED_SUCCESS：workspace 1,062 passed／0 failed／2 既有 ignored、fmt／clippy／實際 no-std；獨立核對六方向真模型 12 筆 Confirmed。補清早期 model-smoke-v1 自有 holder／attach，原清理誤判及更正保留；目前狀態文件更新，最終 CI／合併仍待完成。

- 2026-10-07：12D 開始 secret-reference／allowlist 設定與 HTTPS 傳輸；getMe 唯讀一次、零訊息，基礎測試通過；正式 notifier、手機操作與 G4 仍待實作。12C 未合併工作樹保留供覆核，開 D 前已確認完成的 12B 與本輪 C 測試暫存清理。

- 2026-10-07：12D 通知完整分段與 SQLite 逐段送出意圖／收據通過 NTF、長 Unicode 及未知結果重開不重送測試。真 Telegram 三則文字探測均已刪除；發現裸文字會 trim，改用首尾標記保留完整內容。daemon worker、手機操作及 G4 尚未完成。

- 2026-10-07：12D 接上 daemon config 與 outbound worker；持久 needs-you source 對帳避免 boot 游標重建造成重送，內容更新／解除／再開另立 delivery。手機 inbound、互動操作、G4 與整體驗收尚未完成。

- 2026-10-07：12D 手機 inbound checkpoint：已確認通知收據綁 allowlist／選項，SQLite update 與單通知操作 claim 阻止重播；重試等待 supervisor 處理、修改原因以回覆輸入，pipeline 執行前核任務與注意事項版本。跨入口同原因再開、失敗事件重啟與 unknown 重開納入回歸；真手機 callback／G4、全新端到端驗證及 CI 尚未完成。

- 2026-10-07：12D 補 native HTTP／SQLite／production pipeline 的一次操作與失效按鈕回饋，另驗取消待處理重試不誤回成功；doctor 提示空 allowlist 並安全檢查 token reference。全新覆核 focused 14＋3＋1 通過，範圍不含真 daemon 停機程序／真手機／G4，完整 D 仍待完成。

- 2026-10-07：12D G4 共用已讀：SQLite v16／protocol 1.6 同步 TUI 與 Telegram；雙 TUI 真 daemon 重啟、native HTTP 保留動作、舊追問拒絕及斷線反例已補測，全新 focused 覆核通過。完整 D 真手機／topic／CI 尚未完成（feat/g12d-telegram 本次 checkpoint）。

- 2026-10-07：12D 接上 team topic 任務摘要與輔助 outbox 恢復；native HTTP／SQLite 驗雙 topic、內容更新、重啟不重送及 unknown／foreign 排除。真手機／forum 與完整端到端驗收仍待完成（feat/g12d-telegram 本次 checkpoint）。

- 2026-10-07：12D 專用 Telegram 私訊真測完成 Mark read → 正式 TUI 同步且待辦保持開啟 → acknowledge 關閉；SQLite 核 read／accepted，通知與自有程序／home 已清理。完整 12D 尚未完成；見 gate-12d-telegram 手機驗收。

- 2026-10-07：12D 補原生問答／追問流程：選項與完整多行自由文字經 HTTP、SQLite、正式 pipeline 各投 inbox 一次，重複輪詢與舊通知拒絕；真 Telegram 問答與其餘操作仍另驗。

- 2026-10-07：12D 增補 native human approval／request_changes：原因提示不提前執行、空白拒絕、多行保留與舊按鈕拒絕；無 repo research 範圍，非 Git merge 或真 Telegram 核准認證。

- 2026-10-07：12D 未知通知已接本機需要你／operator-only Abandon；正常傳輸不誤報，重啟恢復未知，不自動重送、不偽造收據。core／daemon 399 tests 通過（2 項既有 ignored）；正式 daemon 三次 boot 驗無 token 仍可處置、拒絕 agent、持久保留原文與未知證據。完整 12D 仍未完成。

- 2026-10-07：PR #156 整體覆核修正 doctor 空 Telegram allowlist 未回報 fail，以及真 daemon CLI 表仍預期 protocol 1.5 的兩列；本機設定測試與完整 CLI 表重驗通過。

- 2026-10-07：12D 補正式 daemon serve 的雙程序 active shutdown／restart：扣住 HTTP 回覆後 SIGINT 等待收據，重啟僅續剩餘段；零真 API／模型。Retry 與真 forum 等剩餘範圍不變（PR #156）。

- 2026-10-07：12D 補正式 supervisor／holder 的本機 mobile Retry 成功、重啟不再啟動，以及 Stop 先排時 Retry 取消不誤報 Accepted。workspace 另發現 testkit protocol mismatch golden 漏 1.6，已修正（PR #156）。

- 2026-10-07：第 12 關原生驗收入口由 Claude 擴充至已整合的 OpenCode／Telegram 與 G4；先建置 consumer 使用的正式 binary／假 producer，仍明示 GitHub forge／剩餘真測未認證（PR #156）。

- 2026-10-07：PR #156 補自由文字 reply 受控探針，本機正式 daemon→ask／單筆 inbox 通過並確認 holder 清空；Telegram 真回覆待使用者操作。另保留 native capture 偶發空白與 macOS outer PTY 320 ms 超過 300 ms 的失敗證據，未宣称修復或整體通過。

- 2026-10-07：PR #156 終端延遲補外層 parser 首次可見時間戳，保留 trigger 前起點與 300 ms；daemon capture cadence 改以開始時間計算，移除 RPC 後額外等待。外層 8 項本機通過，尚不宣稱已定位 CI 320 ms 根因。

- 2026-10-07：12D 整體驗收仍重現 terminal 300 ms 超標；分段紀錄及 CPU 取樣顯示大 frame 的 Content 中間樹成本，改 holder response／client frame 直接解碼。原生兩尺寸 24 bursts 初測通過，尚待反例、全新覆核、完整驗收及新 head CI，不以先前 CI 成功覆蓋本機失敗。

- 2026-10-07：`a69ac49` 全新解碼覆核實跑發現十八例未知欄位拒絕退化（surrogate／數值溢位／深度），目前不可合併；正式整體驗收另在 startup capture 二十案中的兩案僅收到空白 frame 而停止。增加僅限該測試檔的 native fixture 隔離，保留期限、斷言與失敗證據，修正後驗證待完成。

- 2026-10-07：獨立真 Screen 100×24 微測不支持 RawValue decoder 優化，已回復原解析規則並加入十八個未知值拒絕反例；轉向 holder／daemon bounded writer 外包 8 KiB BufWriter，任何 serialization／flush 失敗仍整段拒絕。微測 byte equality 與界線通過，正式原生／整體驗收待完成。

- 2026-10-07：holder 完整 61 tests 與嚴格解碼四項回歸通過；外層 PTY 仍有背景啟動取樣 315.887 ms 超過 300 ms，完整 12D 未通過。曾試將 frame 編碼移出 holder mutex，未解決超標，已撤回該候選；保留原界線與失敗證據，不擴大控制／回覆交錯範圍。

- 2026-10-07：固定 `e0da767` binary 的 startup capture 隔離重驗 20／20 通過（329.79 秒），過程未替換 binary。TUI 正試將已解碼 frame mailbox 的檢查與 50 ms tick 分離，空輪不繪圖或發維護請求；原 300 ms 時效與控制權契約仍待驗，完整 12D 尚未完成。

- 2026-10-07：TUI mailbox 候選的 89 tests 通過，包括三個真 parser／Source spy 反例；debug outer 7／8，100×24 背景啟動取樣仍有 389.290 ms。相同原生 final_dirty 測試另以 release 診斷，36 筆為 62.769–118.094 ms、2 tests 通過；不以此取代 debug 驗收，原 300 ms 斷言與失敗證據保留，完整 12D 待最終覆核／CI及真測。

- 2026-10-07：PR #156 固定 `8c0538b` 的原生 accept 12 exit 0、四個雙平台 CI jobs 全綠；使用者多行 Telegram 回覆逐字一致且單次投遞，真 forum 的 Needs you／team 摘要分流與第二次 boot 收據不變通過。自有訊息、程序及 home 已清理；保留首次回覆內容不完整與歷史延遲失敗，等待最終覆核／合併。12C 使用者已選定嚴格 up-to-date 分支保護，後續實作不得以 head CAS 代替 base 保護。
