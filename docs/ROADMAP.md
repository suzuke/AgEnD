# 施工路線圖：13 個施工關

> **TL;DR**
> - 依 crate 由下往上分 13 個施工關；每個施工關單獨驗收，使用者確認後才開下一個施工關（D22）。
> - 目前狀態：第 1–12 施工關已完成並合併。#157 已合併為 `3f406f5`，自有 worktree／branch／target 已清理。
> - 下一步：第 13 施工關安裝與發布準備；先完成 home／設定與服務生命週期，再處理版本管理、Telegram 配對及發布驗收。

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
| 12 `adapters` | [完成（#154–#157 已合併）](gates/gate-12-adapters.md) | A claude、B opencode driver、C forge github、D Telegram | 先對假實作，再做真 backend smoke test |
| 13 `install` | [施工中（13A home／設定）](gates/gate-13-install.md) | 安裝與發布（最後一個施工關）：服務註冊、`agend uninstall`、`agend telegram setup`（由 daemon 配對）、`xtask release`、brew、GitHub release、`cargo install` | CI 用全新 HOME + 假 agent，從安裝到第一個 task 完成 < 5 分鐘；每個 `doctor` 檢查都有「故意弄壞 → 看到修正指令」的測試 |

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

第 1–12 施工關已完成。使用者 2026-10-08 指示自行安排優先順序並建立 goal，開始第 13 關安裝與發布準備；[施工順序](gates/gate-13-install.md#施工順序2026-10-08)。

## 進度紀錄

- 2026-10-08：13C OpenCode 接入獨立回合證據，核對 native parentID／literal input／成功回覆及查詢前後 session／holder／endpoint；真 1.18.34 capture 反例與原生 fake CLI canary 驗證，保留不准入與不切換 fleet 的邊界。Claude outcome 與整體版本切換仍待完成（feat/g13-install，未合併）。
- 2026-10-08：13C canary 新增獨立 message_outcome；Codex 以正式 thread history 核對單一輸入、訊息身分、成功回合與非空白回覆，排除 confirmed／idle 誤認成功；原生流程與異常證據覆核持續驗證。Claude／OpenCode outcome、版本准入與切換尚未完成（feat/g13-install，未合併）。
- 2026-10-08 13C 補 client 1.7 操作員唯讀 delivery 收據查詢，供 canary 區分受理／送出／確認；canary 執行器與升級流程尚未完成（feat/g13-install）。

- 2026-10-08 13C 加入原生 backend 匯入／inspect 與 agent 更新環境隔離；daemon 建立 holder 前共用核對受管內容，未驗版本拒絕並保存失敗原因。版本漂移、canary、切換／回退仍待完成，見[版本管理](architecture/backend-versions.md)（feat/g13-install，施工中）。

- 2026-10-08 第 13A 完成預設 home／安全初始設定與 290 項 agend 回歸；第 13B 唯讀 service plan 通過 macOS plist／Ubuntu systemd 原生解析。仍在施工，尚未服務註冊或整關驗收（feat/g13-install）。
- 2026-10-08：#157 合併 `3f406f5`，完整 accept 12／獨立覆核／四個最終 CI jobs 通過，自有 worktree、分支、target 與測試程序清理；必要證據與 Claude trust entries 保留。依使用者新 goal 開始 13A：預設 home 與初始化設定，後續服務／版本／配對／發布仍待實作。

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

2026-10-08：12D #156 四個最終 CI jobs 通過後合併為 9dbfac7；自有 worktree、branch、target 已刪除，保留必要證據。12C 嚴格 base protection、production Forge 真 GitHub submit／merge／重開收據／405 拒絕／cleanup 已通過獨立覆核；暫存遠端 repo 與 local lab 已刪。正整合已發布 Telegram v13–16，GitHub migration 改為 v17，再跑整合驗收／CI。第 12 關尚未標完成。

- 2026-10-08：#157 整合 migration 17 的獨立覆核通過；完整 accept 12／四個 CI jobs 揭露終結 action 吞掉本機 WIP 存檔錯誤，修正恢復原 Refusal，保留背景 wake 及遠端收尾行為。原始失敗保留，修正後獨立回歸與完整驗收另核。

- 2026-10-08：13B Linux 真服務流程與所有權反例通過：daemon 停止／重啟保留 holder、解除安裝不載入外來 ExecStop、不停止恢復設定後仍執行的外來 MainPID；另完成全新 home 直接安裝／狀態／解除安裝。13 項 Linux 服務測試、clippy 與無 std 依賴檢查通過；資料刪除確認、macOS 真驗收及 13C–E 仍待完成（feat/g13-install，尚未合併）。

- 2026-10-08：13B 加入刪資料的完整 home 確認與保留鎖 inode 的清理；macOS 15 項服務／6 項 CLI、Linux 16 項服務及另跑的真 bind mount 反例通過，外部 symlink／掛載資料保持不變。獨立覆核修正掛載判定與提前釋放 SQLite 鎖；容器／專用映像／建置暫存已清，施工 worktree／target 保留待合併。macOS 真服務、13C–E 與整關驗收仍未完成（feat/g13-install）。

- 2026-10-08：13B macOS live PID 防護加入 native executable 映射 inode、argv／home、UID／世代；自有 C producer 與 SDK ABI 比對、同路徑替換反例通過，整組服務 16 tests 通過。僅程序檢查，未註冊 host service；loaded launchd 定義捕獲計畫已備妥待授權，13C–E 仍未完成（feat/g13-install）。

- 2026-10-08：13C canary 加入隔離執行器及原生程序清理反例；修正 macOS zombie-only 群組 EPERM、回收後不再 signal 與 daemon 結束等待上限。三項程序測試通過，完整 fake canary 因 fleet unknown 逾時仍未通過，另待正式操作員投遞與脫離群組後代清理；未執行真模型／未准入版本（feat/g13-install，未合併）。

- 2026-10-08：13C 補 1.7 操作員 send_message／driver_status，真人 sender 使用 instance 不允許的 `@operator` 命名空間，歷史同名 agent 收據衝突拒絕且不改寫舊資料；身分邊界通過獨立覆核。送訊息沿用 driver 收據去重。原生 fake Codex 三訊息 canary 與版本不符案例通過，權限／內容衝突／未知 instance／重啟收據測試通過；尚待 detached descendant 清理、真 backend 身分隔離與准入切換（feat/g13-install）。

- 2026-10-08：13C 版本探測加入 macOS process-fork／Linux seccomp 限制，保留執行緒而拒絕建立子程序；未回收 Child 正 PID 停止涵蓋探測程序自行切換 session。macOS 四項原生反例、fake canary 與 Linux ARM64 production 模組驗證通過，獨立覆核關閉版本探測 detached-child 缺口；容器與建置 lab 已清，Linux x86_64 真跑、實際 backend 與准入切換仍待驗（feat/g13-install）。

- 2026-10-08：13C 完成判定增加 Claude ACK／PostToolUse／Stop 與原生 prompt ID 綁定，跨生命週期、重複與缺失證據拒絕；保存原生事件反例、Store 重開、fake canary／收據回歸、fmt／clippy／無 std 依賴檢查通過。此範圍獨立覆核完成，測試暫存與程序無殘留；完整 Claude canary、真模型與版本准入尚未完成（feat/g13-install，未合併）。

- 2026-10-08：13C 完整 fake canary 擴至 Claude／Codex／OpenCode，4 項測試通過；Claude 經保存的 Ready 畫面、正式 channel／hook helper 跑三次 ACK→PostToolUse→Stop，Unknown 在期限內等待，Completed 必須具 execution ID。缺失／重用 execution ID 拒絕，legacy fake 3 項與 conformance 6 項通過。真模型、認證隔離、版本准入與切換仍待完成（feat/g13-install，未合併）。

- 2026-10-08：13C startup executable binding 加入實際 running inode 核對，拒絕 pathname 已被替換或後續替換；macOS 兩項 binding、4 項 canary／5 項匯入回歸通過。獨立覆核指出 preflight→exec 仍有替換空窗，准入維持拒絕，下一步固定實際執行檔再開放；Linux binding 與完整版本管理尚未驗收（feat/g13-install，未合併）。

- 2026-10-08：13C 正常 daemon 已接入私有固定啟動副本；原始 binary 被替換後仍能啟動 holder，daemon 停止保留 holder／副本，最後自有 Lab 清理通過。4 項 binding／snapshot 測試通過；三 backend canary 回歸為 3 通過／1 失敗：Claude 報告覆核抓到同時建置造成的 binary 身分改動，須固定產物重驗；准入仍關閉（feat/g13-install，未合併）。

- 2026-10-08：固定 AgEnD 啟動副本整批重驗完成：4 項三 backend fake canary、6 項服務安裝、原生 holder 替換／存活、shim ownership 反例及 daemon 生命週期回歸通過，workspace clippy／無 std 依賴檢查通過。覆核確認 D3／D5 不要求同 build holder；准入尚缺存活 backend 與持久啟動身分對帳。自有程序／暫存無殘留，工作樹未合併保留（feat/g13-install）。

- 2026-10-08：holder 1.3 加入不可補認的原生啟動 UUID 回報；65 項 holder、149 項 core（另 2 項既有 ignored 未執行）、workspace clippy／check-deps 通過。覆核發現停止時第二次 Spawn 競態，已以永久 spawned 與 stopping guard 修正；原生 HUP 重疊測試通過，移除防護的 mutant 實際產生第二個 PID 並失敗。daemon 持久綁定仍待接入、准入關閉，本批 holder 暫存已清理（feat/g13-install，未合併）。

- 2026-10-08：holder 啟動 UUID 與關閉競態修補提交 `715bdf0`；65 項 holder、149 項 core（另 2 項既有 ignored）、runtime 回歸與 clippy／check-deps 通過。Ubuntu canary 三回合完成後超出 60 秒測試預算，整合測試改採正式 180 秒預設；本機三 backend 原生 fake canary 4 項於 87.23 秒通過，包含程序／暫存清理。遠端 CI 與 daemon 持久啟動紀錄仍待完成，不宣稱整關或真模型通過。

- 2026-10-08：修正固定 launcher 重啟時的重複雜湊，重用快照驗證產生的檔案身分 binding；保留 running image／摘要／ownership 檢查與 10 秒 CLI 等待期限。原失敗 CLI table 原生重跑通過（38.92 秒），snapshot 2 項與 pinned launcher 1 項、clippy、check-deps 通過，聚焦獨立覆核未發現 blocker。同時更新 client 1.7 協商斷言；整關 CI／持久啟動紀錄仍待完成（feat/g13-install，未合併）。

- 2026-10-08：13C migration 0018 新增受管啟動意圖，UUID、artifact 與啟動參數在同一 SQLite transaction 以 instance 快照及舊 binding CAS 保存；明確移除 instance 時 cascade，重建不繼承。原生 SQLite 新測試 2 項、既有 store 42 項及 core 149 項通過（2 項既有 ignored 未執行），fmt／clippy／check-deps 通過；聚焦覆核無 blocker，測試暫存已清理。supervisor／runtime 串接與准入仍未完成（feat/g13-install，未合併）。

- 2026-10-08：13C reserved runtime 接入 SpawnBound／GetLaunchBinding，持久 UUID／PID 核對後才發布 writer；重連不送 Spawn，不符時保留 holder。未驗證 Exited 暫存，intentional close 不誤報失敗且核對可取消。原生 holder 7 項、runtime 19 項、終端 hub 8 項與 fmt／clippy／check-deps 通過，聚焦覆核缺口已修正；清除重複驗證 logs。supervisor 的受管啟動／版本准入決策尚未串接（feat/g13-install，未合併）。

- 2026-10-08：Ubuntu CI `37710056778` 定位至 CLI 兩 Codex agent 重啟的 10 秒連線逾時，尚未判定根因；補上該測試失敗時停止自有 daemon 並保留 stderr 的診斷。本機原生重跑通過（恢復 6.1 秒，20 則各一次），fmt／clippy／check-deps 通過；未放寬 timeout，也不宣稱 Ubuntu 已修復（feat/g13-install）。

- 2026-10-08：13C supervisor 接入受管啟動准入與持久 UUID；canonical 匯入程式進 launch argv，先核舊 holder／orphan 已離開再保存意圖。重連核原 artifact／設定／UUID，不重跑新版 canary；首次 Codex／OpenCode 原生 session 發現與啟動時指定 session 分開。三 backend 原生 canary＋fleet 啟動／重連／錯 UUID 保留程序共 4 項通過（102.21 秒），supervisor 8 項、匯入拒絕回歸 5 項、fmt／clippy／check-deps 通過；聚焦覆核兩項缺口已修正。版本切換／回退與整關驗收仍待完成（feat/g13-install，未合併）。

- 2026-10-08：補受管 canary build 身分失配反例，正式報告改成不匹配 digest 後，新啟動准入拒絕，三 backend 的原 holder 仍以持久 UUID 重連；原生 4 項通過（99.98 秒），fmt／clippy／check-deps 通過，自有程序／lab 清理完成。更新版本管理文件移除已失效的「全部拒絕准入」敘述；不宣稱已驗證所有跨版本 driver 相容性（feat/g13-install）。

- 2026-10-08：13C migration 0019 保存每 instance 最近一次 BackendSwitch，prepare 不改 program，commit／rollback 與 phase 同交易；設定、舊 switch 或來源啟動意圖變更拒絕提交。原生 SQLite 3 項、store 42 項、core 149 項通過（2 項既有 ignored），fmt／clippy／check-deps 通過；聚焦儲存契約覆核無 blocker，測試暫存與重複 clippy log 已清理。尚未接操作入口、idle 排空、停止／啟動與 Prepared 取消／恢復，不宣稱完整版本切換完成（feat/g13-install）。

- 2026-10-08：13C Prepared 切換可取消，保留原 program 與 agent PID，取消後可建立新請求；Committed 拒絕取消，須走 rollback。原生 SQLite 4 項、fmt／clippy／前後 check-deps 通過，測試暫存與重複 log 已清理。這批只完成儲存契約，CLI、投遞排空及 supervisor 恢復仍未接入（feat/g13-install，未合併）。

- 2026-10-08：13C OpenCode runtime 保留尚未退出的取消／舊代 worker，以實際 thread completion 提供停止查詢；重連及重複取消不會丟失舊代。新增原生 worker 生命週期 1 項、既有 OpenCode 20 項通過，fmt／clippy／check-deps 通過；對應測試暫存皆不存在。尚未接入 supervisor 版本切換，worker 停止不代表 backend 回合結束（feat/g13-install，未合併）。

- 2026-10-08：13C Prepared 在正式 Claude channel／Stop、Codex、OpenCode reservation 暫停新的 push attempt，取消後恢復，原回執可確認；6 項切換測試、16 項 Codex 回歸、daemon 單元 185 項通過（1 項既有 ignored）。修正舊 store 測試以精確保留永久 maintenance lock 並核 inode／權限；fmt／clippy／check-deps 通過，85 種相關測試目錄無殘留。inbox、在途寫入排空、idle 與 supervisor 切換編排仍未完成（feat/g13-install，未合併）。

- 2026-10-08：13C agent inbox 讀取與 Prepared 檢查在同一 DB 工作執行，準備中明確拒絕送出內容，操作員歷史與其他 instance 不受影響；取消恢復最後筆數／after 游標。7 項切換測試、3 項正式 pipeline context、既有 handler 回歸及 fmt／clippy／check-deps 通過。supervisor 的在途回覆排空、idle 與版本切換編排仍待接入（feat/g13-install，未合併）。

- 2026-10-08：13C Codex activity 追蹤涵蓋建立中的連線與 bounded close 逾時後的舊 worker；disconnect 返回不當成停止證據。原生 Unix socket 握手阻塞與 fake app-server Gone callback 超時反例，連同既有 Codex 共 18 項通過；fmt／clippy／check-deps 通過。初版測試誤用 duplex=false（仍可完成握手）已改成原生無回覆 socket，保留失敗證據；supervisor 切換編排仍未接入（feat/g13-install，未合併）。

- 2026-10-08：13C client 1.8 接入操作員 prepare／status／cancel，RPC 不重送；原生 daemon／CLI 驗 Prepared 查詢、精確取消、重啟保留、agent／舊 ID 拒絕，以及 once decoder 的有紀錄／null。Client 52 項、core 149 項（2 項既有 ignored）、daemon 單元 185 項（1 項既有 ignored）與 switch 7 項、fmt／clippy／check-deps 通過；自有 switch Lab 無殘留。成功 prepare 准入整合、實際換版／回滾仍待完成。遠端 3e3867f CI 兩平台重啟連線逾時，已保存失敗 log，未宣稱整體通過（feat/g13-install，未合併）。

- 2026-10-08：重啟逾時追查補 executable 驗證耗時日誌；固定 binary 的本機空 fleet 重測，debug sha2 最佳化使重啟指紋核對 7296→520 ms、CLI 全程 11.518→1.114 秒，所有驗證與 10 秒重連期限保留。CLI 原 18 項通過，協定預期更新 1.8 後完整表通過；client protocol 9 項、pinned launcher 1 項、雜湊保護 4 項及 fmt／clippy／check-deps 通過。自有 g8／g9／pin／timing 暫存無殘留；兩平台遠端 CI 尚待本次 head 驗證，不宣稱逾時已全面修復（feat/g13-install，未合併）。

- 2026-10-08：13C 新增 stop_reserved，停止前核 holder PID，並在同一連線核持久 UUID／instance／agent PID 後 Shutdown；錯身分、legacy 與替代 holder 保留。三項原生反例含 binding 回覆時替換 socket 路徑，核另一 holder 存活；holder runtime 全 10 項、fmt／clippy／check-deps 通過，g6／bound-stop 測試暫存無殘留。回合結束與在途排空、supervisor 換版編排仍待接入（feat/g13-install，未合併）。

- 2026-10-08：13C prepare 在持久暫停投遞後，等待先前 Claude／inbox 回覆完成 socket 寫入；逾時保留 Prepared，後來的輪詢不延長排空範圍。原生背壓完整送出／斷線／逾時測試、daemon 185 項（1 項既有 ignored）、switch／channel／Stop 回歸通過。這只證明本機回覆結束，尚不代表 backend 回合完成或換版可安全啟動。遠端 c263492 兩平台失敗均為測試仍預期協定 1.7；更新目前 daemon 的 1.8 斷言後，Claude 控制權測試與 terminal hub 8 項通過，保留舊版相容案例。client protocol 全 10 項及 fmt／clippy／check-deps 通過，自有 g8／g12b／g11h／g11stop／switch-rpc 暫存均不存在（feat/g13-install，未合併）。

- 2026-10-08：13C 將改 program 與啟動完成分開：Committed／Restoring 跨重開維持 push／inbox 暫停，精確 Running／PID／session／新 managed launch 與 artifact 快照才可完成為 Activated／RolledBack；進行中不可被新 prepare 覆蓋。三 backend × 啟用／回滾 Store 反例及既有切換共 8 項、core 149 項（2 項既有 ignored）、daemon 單元 185 項（1 項既有 ignored）、CLI 回歸與 fmt／clippy／check-deps 通過。這是持久狀態契約，真 native readiness 與 supervisor 切換編排仍待接入（feat/g13-install，未合併）。

- 2026-10-08：13C 正式終端 acquire／resize／input 與 legacy input 接入持久暫停及在途排空，唯讀／release 保留；原生 PTY 背壓核完整 bytes，啟動前 Prepared 經正式 cancel RPC 恢復輸入。terminal hub 序列 10 項通過；並行跑有 holder 5 秒未建 socket 的啟動失敗，已加失敗日誌並保存證據，尚未解決，不以序列通過宣稱整體穩定。初版測試另開已鎖 DB 被拒，已改成啟動前建狀態。daemon 單元 185 項（1 項既有 ignored）、fmt／clippy／check-deps 通過。另發現兩個逾時後才啟動的自有 holder 重建已刪 home，保存日誌並移除自有 home 觸發 watchdog；啟動生命週期缺口仍待修正。native 回合完成與 supervisor 啟用／恢復仍待接入（feat/g13-install，未合併）。

- 2026-10-08：holder 啟動改為只在既有 AGEND_HOME 建立 run／holders 子目錄，避免延遲啟動重建已清理 home。原生子程序在 exec 前停住、刪 home 後放行，核拒絕且無目錄復活；holder 65 項、holder_process 8 項、holder_runtime 10 項及 clippy／check-deps 通過。兩個已發現的自有晚啟動 holder 與 home 已確認消失；此修正不宣稱解決並行啟動 5 秒逾時（feat/g13-install，未合併）。

- 2026-10-08：遠端 b54099a 的 Ubuntu／macOS CI 均停在 testkit 的舊 Hello 版本清單斷言；同步為目前 1.3–1.8，保留精確錯誤內容與 EOF 檢查。agend-testkit 全套 116 項、clippy／check-deps 通過；新 head 完整 CI 尚待執行（feat/g13-install，未合併）。

- 2026-10-08：13C Codex 新增 thread_idle，透過既有連線查完整回合，核 session／連線物件／generation／instance 未變，只認 completed／failed／interrupted；分頁缺 data 或 nextCursor 拒絕，不當空閒。原生 fake app-server 驗空 thread／busy／完成／session 變更／斷線與 producer 變異反例，Codex driver 19 項通過；daemon 單元 185 項通過（1 項既有 ignored）。這是閒置觀察，尚須 supervisor 暫停／排空、受管 holder 身分與完整換版／恢復接入（feat/g13-install，未合併）。

- 2026-10-08：13C OpenCode session_idle 讀 REST 狀態，前後核 instance／holder PID／session handoff／endpoint／憑證，錯 session 或 holder 消失拒絕。原生 daemon／holder／wrapper＋fake REST 兩條重啟路徑驗 idle／busy／abort 後 idle，共 3 項 native 與 20 項 OpenCode 回歸、fmt／clippy／check-deps 通過。初輪測試缺 Tokio runtime 已修正並保留失敗 log；g12open 程序與暫存無殘留。尚未接 supervisor 換版編排，不代表已驗證完整停止／啟動／回復（feat/g13-install，未合併）。

- 2026-10-08：13C 將 Claude startup 按鍵納入暫停／本機排空：Prepared 在 reservation 同交易拒絕新鍵，原操作在 server tracker 追蹤至返回；Committed／Restoring 允許新 launch 走啟動選單。SQLite 9 項及原生 Prepared 無鍵／正式 cancel 後三鍵測試通過；完整 startup 並行 5 過 6 失敗（holder 5 秒未啟動），序列 11 項通過，保留兩份證據，不宣稱並行穩定。Claude 閒置證明與 supervisor 完整換版仍待接入（feat/g13-install，未合併）。

- 2026-10-08：定位私有 executable 首次執行延遲：8 份新複本並行 --version 最慢 5.734 秒，暖啟動 7–13 ms，皆 exit 0。daemon 現在於準備階段對已驗 binding 的私有 launcher 執行 --version（30 秒等待、清空環境、前後核身分），不延長 holder 5 秒連線期限。先前失敗的 startup 預設並行 11 項及 terminal hub 預設並行 10 項全過；launcher 原生成功／失敗／逾時／替換拒絕、fmt／clippy／check-deps 通過，暫存已清。初版單元成功案例 100 ms 太短，改 5 秒，故意逾時案例仍 100 ms；保留原失敗證據。完整最新 head CI 尚未完成（feat/g13-install，未合併）。

- 2026-10-08：13C Claude 閒置觀察綁定 live hook 的 session 與原 holder connection；重連、工具活動、session 結束撤銷舊候選，初始 Ready 另核完整畫面與 generation。原生 hook／holder 反例通過，startup 回歸 11 項通過，最終 clippy／check-deps 通過；尚未接 supervisor 換版編排。21cd574 的兩平台 CI 均停在 xtask 兩個舊 Hello 清單斷言，修正後 xtask 42 項本機通過，完整新 head CI 待驗（feat/g13-install，未合併）。

- 2026-10-08：13C 一般 boot start／death restart／延遲 restart／operator retry 遇到持久 pending switch 時保留現況，讀取失敗也不停止 holder；由換版恢復流程決定後續啟停。原生 daemon 驗 Prepared／Committed／Restoring 跨 boot 精確保留 instance、managed launch、switch，連同既有 switch RPC 共 2 項通過；fmt／clippy／前後 check-deps 通過。這批未完成換版專用恢復／啟用／回滾（feat/g13-install，未合併）。

- 2026-10-08：13C Codex 閒置查詢改在同 worker 串行查原生 queue 與完整 turns；queue 非空、缺 data／nextCursor 或 continuation 不可認閒置。真 fake app-server 驗第二筆待執行訊息與消化後空 queue，producer 變異反例、Codex driver 全 19 項及 fmt／clippy／前後 check-deps 通過。仍是換版閒置前置條件，啟用／回滾編排未完成（feat/g13-install，未合併）。

- 2026-10-08：13C 接上明確 Activate／Rollback RPC 與 CLI，目的准入、reply fence、native idle、worker 結束及 exact holder stop 後提交版本並啟動；定期核新 holder binding／readiness 後釋放投遞。Activated 回退先持久 RollbackPrepared，boot 對 Committed／Restoring 缺 holder 的合格目的可重啟。Codex 雙原生假版本經正式 canary 完整啟用／回滾，核 session 保留與兩次 holder 更換通過；Store 9 項、holder runtime 10 項、switch RPC 2 項、client 52 項、core 149 項（2 ignored）、clippy／check-deps 通過。前兩輪新測試錯用外部 SQLite 查 live daemon，被獨占鎖拒絕；已改正式 RPC＋停機後核 DB。新 store 測試誤把 inbox 暫停當 None，改為正式拒絕後通過；保留失敗證據。尚未認證三後端完整往返、全部 crash 切點與自動失敗回滾（feat/g13-install，未合併）。

- 2026-10-08：本批換版回歸補 daemon 單元 186 項（1 ignored）、terminal hub 10 項、Claude startup cancel 1 項。前一批 boot 保護使兩個啟動前種 Prepared 的 shell fixture 不再啟動，改為先啟動原 holder 再停 daemon 種 pause，重連時僅保留 pause 測試所需資料，不宣稱這些 shell 具 managed admission；取消操作保留有效 terminal，僅恢復缺少的 driver。保留原回歸失敗與編譯錯誤證據，最新 fmt／clippy／check-deps 通過；自有測試目錄檢查無殘留，移除被最終證據取代的成功 logs（feat/g13-install，未合併）。

- 2026-10-08：13C Claude 雙假版本往返找出身分查詢另開 socket 會取代 holder 唯一 client，導致 Ready 觀察失效；改用既有連線查 binding 並核原連線仍有效後，完整 canary／啟用／回滾與 session 保留通過（74.77 秒）。fixture 閒置不再 10 秒退出。35b787d CI 的 Codex 初次 idle 查詢早於 worker 就緒，改限時等待初次連線後全 19 項通過；完整 CI 尚待重跑。OpenCode 新版准入及完整 crash／自動回滾仍未完成（feat/g13-install，未合併）。

- 2026-10-08：13C OpenCode 新版 canary 與 fleet 版本核對改依精確匯入 artifact／私有 CanaryScope；scope 綁 home inode/device、instance、workspace、program 與來源雜湊，不是 fleet 成功報告。正式 import＋scope producer 的跨 home／錯 instance／額外 argv／修改 artifact 反例通過；OpenCode 雙假版本完整 canary／啟用／回滾及 session 保留通過（66.22 秒）。未執行真模型，三 backend 全組回歸及全關驗收尚待完成（feat/g13-install，未合併）。

- 2026-10-08：三 backend 的 canary 全組 8 項並行通過（75.27 秒），包括各雙版本正式 runner、啟用／回滾及 session 保留；fmt、clippy、check-deps 通過。仍僅為原生假 backend 證據，不代表真新版模型相容或完整 crash 驗收。

- 2026-10-08：13C 三 backend 新增 Committed／新 holder 未就緒切點：fixture 在自有 workspace 等待，硬殺自有 daemon 後重開，核 holder PID 未變再繼續啟用／回滾與 session 保留；三項並行通過（75.97 秒）。fmt／clippy／check-deps 通過。尚未涵蓋 Ready 已出現、還原途中再中斷與自動失敗回滾；沒有執行真模型或改主機服務（feat/g13-install，未合併）。

- 2026-10-08：b4f9032 macOS CI 揭露 probe SIGKILL 後、waitable exit 前群組 EPERM 的競態；改先限時等待未回收 Child 退出（PID 仍固定），再清理群組／回收／核群組不存在。4 項原生 probe 通過；setsid 測試首輪未及寫 marker，改採正式 5 秒 probe 預算，仍核 marker。另驗 Codex Committed 下 daemon 硬中斷、目的 holder 由 Lab 停止後重開，核新 holder／正確版本／原 session／完整回滾通過；不代表所有 backend 或所有 crash 切點已完成（feat/g13-install，未合併）。

- 2026-10-08：13C 目前代 Committed 目的 holder 確認消失且 launch 精確吻合時自動回退；Committed／Activated 先持久 RollbackPrepared，server 就緒後續行重啟前意圖。三 backend 原生假 canary／往返／holder 消失全組 12 項通過；其後補 Codex 正式 Store 建立回退切點、重啟恢復舊版本與 session 的 1 項通過。Store 9 項、daemon 單元 186 項（1 ignored）、fmt／clippy／check-deps 通過；自有 switch／managed／canary 暫存無殘留。存活但未 ready 的 backend、Ready 已觀察後中斷、Restoring 再失敗與通知政策仍待完成；未宣稱全 crash matrix 或真模型驗收（feat/g13-install，未合併）。
