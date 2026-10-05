# 施工路線圖：13 個施工關

> **TL;DR**
> - 依 crate 由下往上分 13 個施工關；每個施工關單獨驗收，使用者確認後才開下一個施工關（D22）。
> - 目前狀態：**第 1–11 施工關完成並已合併**；第 11 施工關 C 段 #145 經全新 verifier、雙平台 CI、實機及後續自動驗收與清理，使用者於 2026-10-03 確認合併（`b2152db`）。第 12 施工關 A 段設計 #138 已 merge，client 基礎 #147 已 merge，持久化 #148 已 merge，protocol 1.5／channel／Stop／ACK spool 基礎 #149 已 merge，共用 gh 防護 #150 已 merge，完整 Claude Driver／啟動設定與 Interrupt 實作中，Claude 接入未完成；第 13 施工關未開始。
> - 下一步：依第 12A [D40](decisions/d40.md) 完成 [Claude Driver／啟動設定與控制](gates/gate-12a-driver.md)；每批以全新 verifier、可重驗指令及使用者確認收尾。

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

- 2026-10-04：使用者確認 #149，`c7e398c` 全新 verifier r2 CONFIRMED、push／PR 雙平台 CI 通過，合併為 `6dd552e`；使用者另重驗 16 native cases 全過。已清理 feature／verifier worktree、branches、targets；另移除已合併且乾淨的舊 `docs/gate-12-proposal` worktree／branch。有未提交變更的舊 worktree 保留。
- 2026-10-04：下一批在 `feat/gate-12a-gh-shim` 完成 D40 P4 的 [共用 gh 防護](gates/gate-12a-gh-shim.md)，首輪 3 unit／5 native cases 與 workspace clippy 通過；完整驗證、fresh verifier 與 CI 另核，未 merge（`d4853ad`／[draft PR #150](https://github.com/suzuke/AgEnD/pull/150)），完整 A 段仍未完成。

- 2026-10-04：使用者確認 #147，合併為 `8dfccf8`；`5a4047c` 全新 verifier CONFIRMED、46 client tests 通過，push／PR 雙平台 CI 通過。PR macOS 首次未改動的 TUI 時序測試超過 300 ms，原失敗保留，重跑通過且門檻未改。舊 worktree／branch 已清理；在 `feat/gate-12a-claude-store` 開始 [投遞／ACK／retention 基礎](gates/gate-12a-store.md)，Claude 接入尚未完成。

以下是各批次**當時**的進度原紀錄；其中「draft」「待驗證」「尚未 merge」只描述該批次，不是目前狀態。目前以頁首完成狀態及最新合併紀錄為準。原失敗與驗證範圍不改寫成成功；歷史證據僅列封存檔名，不公布本機暫存位置。

- 2026-10-04 第 12A P1–P10 設計確認記為 [D40](decisions/d40.md)（[draft PR #138](https://github.com/suzuke/AgEnD/pull/138)）：閒置 channel／忙碌 Stop、明確 agend_ack、P3／P4／P5＝A；剩餘依建議。只改文件，最新 head 全新 verifier／CI 另核；未 merge、實作或新增真模型回合。


- 2026-10-03 使用者「同意」只開放 Codex CLI 0.159.3：實作 holder 啟動版本辨識與 migration 0006 的永久 thread 歸屬，未知／其他版本仍拒絕；live／reconcile／events 不因版本降級退回文字匹配。原 head 68e15c0 經全新 verifier 核實 892 passed／2 既有 ignored；新實作另驗，不增加真模型或 merge 授權（draft PR #145；[版本政策](gates/gate-11c-codex-input.md)）。


- 2026-10-03 使用者明確核准四回合真 Codex：0.159.3／gpt-6-luna／low 的 U17 首次通過，同 thread／holder、busy Queue、idle Send、重啟後 code word 與兩個獨立 durable receipts 有原始證據；第四回合只核 receipt，沒有最終回覆斷言。[live 證據](gates/gate-11c-u17-live-validation.md)；版本開放、全新 verifier 與人工驗收仍待完成（draft PR #145）。


- 2026-10-02 C 段控制／runtime（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：holder 實際 resize／input ack、FIFO 交接與 5 秒 native write；runtime 能力／連線 epoch、背景配對、取消 grant 失效、8 MiB bounded reader。holder 53 passed、完整 daemon crate 與 16 個真程序回歸通過；原失敗保留。daemon／client／TUI 與 U17 仍待完成，未獨立／人工驗收或 merge；[實作進度](gates/gate-11c-progress.md)。


- 2026-10-02 C 段第一個實作提交 `a13d31c`（[draft PR #145](https://github.com/suzuke/AgEnD/pull/145)）：holder 1.1 的結構化 frame、request id、generation／revision、色彩／游標／mode、歷史 viewport 與 8 MiB 整份拒絕。holder 47 passed；accept core 含 workspace clippy／實際 no-std 通過，兩個既有 deep explorers ignored。完整 C 段與 U17 仍在實作，尚未獨立／人工驗收或 merge。

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
