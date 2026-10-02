# 施工路線圖：13 個施工關

> **TL;DR**
> - 依 crate 由下往上分 13 個施工關；每個施工關單獨驗收，使用者確認後才開下一個施工關（D22）。
> - 目前狀態：**第 1–9 施工關完成並已合併**；第 11 施工關 A、B 段已驗收並合併（B 段 #140，2026-10-01），C 段提案 P1–P6 待確認，尚未實作。第 10 施工關已完成自動／獨立／CI 與人工補驗，使用者已確認合併（#143，2026-10-02）；第 12 施工關 A 段提案 #138 待確認；第 13 施工關未開始。
> - 下一步：確認 [第 11 施工關 C 段 P1–P6](gates/gate-11c-proposal.md)，確認後再實作。第 12 施工關 A 段 P1–P10 在 #138，仍待使用者確認。

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
| 11 `tui` | [提案中（A、B 段完成並已 merge；C 段 P1–P6 待確認）](gates/gate-11-tui.md) | attention-first TUI：畫面層、接真 daemon；C 段完整重現 agent CLI 與滑鼠滾動 | A 段假事件、B 段真 daemon 驗收通過；C 段提案與驗收計畫已整理，待確認後實作 |
| 12 `adapters` | [提案中（A 段 draft PR #138 待確認，尚未 merge 或實作）](gates/gate-12-adapters.md) | A claude、B opencode driver、C forge github、D Telegram | 先對假實作，再做真 backend smoke test |
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

第 10 施工關已完成驗收，使用者於 2026-10-02 明確確認 merge（[PR #143](https://github.com/suzuke/AgEnD/pull/143)）。下一步為確認 [第 11 施工關 C 段 P1–P6](gates/gate-11c-proposal.md) 後實作；第 12 施工關 A 段 #138 的 P1–P10 仍待使用者確認。 人工主流程與事件補驗見 [人工紀錄](gates/gate-10-manual-record.md)。

## 進度紀錄

- 2026-10-02 依使用者「繼續往下推進」開 `docs/gate-11c-proposal` 專屬 worktree，整理 C 段 P1–P6：完整畫面、holder frame／協商、控制／resize、mouse／paste／歷史、Codex U17 與驗收矩陣。只改文件，待全新 verifier、CI 與使用者確認；尚未實作或 merge。

- 2026-10-02 文件 verifier r18 REFUTED `044f36e`：363 個非 Markdown entries 與 `430478d` 的 blob／mode 完全相同，但四份 crate README 與名詞表六列仍有未標歷史的「待驗收」舊狀態；已同步完成狀態，產品碼未改。最終文件驗證與 CI 見 #143，原報告 `/private/tmp/g10-r18-report.md` 保留。

- 2026-10-02 第 10 施工關完成驗收，使用者確認 merge（#143）：`430478d` 全新 verifier r17 CONFIRMED，完整 accept 718 passed／2 ignored，實際 no-std 與四個 Ubuntu／macOS CI job 通過；人工事件補驗確認完整 stage、一次正確 Approve resolution、單次 Git merge 及 teardown。舊反例與 frozen-head 紀錄保留；收尾文件與最新 CI 結果見 PR。

- 2026-10-02 verifier r16 REFUTED `eac09cf`：六項完整 baseline（workspace 774 passed／2 ignored、accept 716 passed／2 ignored）與四個 CI 全綠，但真 SQLite post-CAS note 故障留下 durable approval、resolved unknown 與停住的 merge，重啟才恢復一次。attention_reason 清除改與 CAS 同交易，fake／SQLite 契約及真程序反例回歸補齊；原失敗 log 保留，修正待全新驗證及人工補驗，未 merge（#143）。

- 2026-10-02 使用者完成 `45e957e` 的 11 步人工主流程；main 防護、WIP patch、checks 中重啟與沙箱拒絕通過，home／repo／原 holder 清理完成。發現 watch 缺 stage、timeout 核准項目重現與 resolved unknown；兩個真程序回歸已在原版本重現 exit 101，修正與新一輪驗證進行中，待補驗及確認，未 merge（#143；[人工紀錄](gates/gate-10-manual-record.md)）。

- 2026-10-02 `ea975c3` 的 macOS PR CI（job 110607655251）在 content-filter fixture 的準備斷言失敗；有效 stat cache 下普通 git add 沒套用新 filter。已確定性重現，改真 Git --renormalize 強制建立轉換後 blob，六個回歸與 clippy 通過；原 CI／101 log 保留，產品碼不變，待最新 CI（draft PR #143）。
- 2026-10-02 文件 verifier r13 在 `ea975c3` 找到索引的 target／PATH 與人工驗收頁不一致（REFUTED）；統一 CARGO_TARGET_DIR 與 binary PATH，實際初始化選到 `agend 0.0.0`。其餘文件、證據與產品 tree 核對通過，產品碼不變；人工驗收及 merge 仍待使用者（draft PR #143）。
- 2026-10-02 全新 verifier r12 CONFIRMED `dfe5bc6`：完整 workspace 771 passed／2 ignored、accept 713 passed／2 ignored與三組組合探測通過；原 PATH／等待 fixture 失敗保留。push／PR 共四個 Ubuntu／macOS CI job 成功。README、ROADMAP 與 Gate 10 驗收文件已同步／分頁，待人工驗收、未 merge（draft PR #143；[證據](gates/gate-10-verification.md)）。

完整紀錄見 [施工路線圖進度紀錄](roadmap-progress.md)。
