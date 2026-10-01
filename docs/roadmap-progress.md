# 施工路線圖進度紀錄

> **TL;DR**
> - 這是 13 個施工關的歷史進度；新的紀錄在前。
> - 目前狀態以 [ROADMAP](ROADMAP.md) 的狀態欄為準。
> - 下一步：依 ROADMAP 的下一步完成目前施工關驗收。

## 進度紀錄

- 2026-10-02 `ea975c3` 的 macOS PR CI（job 110607655251）在 content-filter fixture 的準備斷言失敗；有效 stat cache 下普通 git add 沒套用新 filter。已確定性重現，改真 Git --renormalize 強制建立轉換後 blob，六個回歸與 clippy 通過；原 CI／101 log 保留，產品碼不變，待最新 CI（draft PR #143）。
- 2026-10-02 文件 verifier r13 在 `ea975c3` 找到索引的 target／PATH 與人工驗收頁不一致（REFUTED）；統一 CARGO_TARGET_DIR 與 binary PATH，實際初始化選到 `agend 0.0.0`。其餘文件、證據與產品 tree 核對通過，產品碼不變；人工驗收及 merge 仍待使用者（draft PR #143）。
- 2026-10-02 全新 verifier r12 CONFIRMED `dfe5bc6`：完整 workspace 771 passed／2 ignored、accept 713 passed／2 ignored與三組組合探測通過；原 PATH／等待 fixture 失敗保留。push／PR 共四個 Ubuntu／macOS CI job 成功。README、ROADMAP 與 Gate 10 驗收文件已同步／分頁，待人工驗收、未 merge（draft PR #143；[證據](gates/gate-10-verification.md)）。
- 2026-10-02 第十一輪 verifier 的六項本機 mandatory 通過（workspace 767 passed／2 ignored，accept 含四次開機），但 novel 以真 daemon／shim 重現強制 color 使 archive 無法 git apply、不完整 nested metadata 遺失 staged bytes，故仍 REFUTED；另重現 diff.noprefix 問題。封存／patch-id 固定無色 a/／b/ 全 repo diff，完整 filesystem 掃描 Git metadata；新增原 bytes 還原／保留回歸。修正待全新 verifier／CI，未 merge（draft PR #143）。
- 2026-10-02 `29fb454` 的 Ubuntu push／PR CI 在 nested repo fixture 建 commit 時因沒有 local Git identity 失敗；補 user.name／email 並以沒有全域 Git 設定的環境重跑。產品碼不變，原紅燈保留；修正仍待 CI／全新 verifier，未 merge（draft PR #143）。
- 2026-10-02 第十輪 verifier 的 unresolved-index 回歸失敗，其目錄數量斷言也包含重試中 staging；修正為檢查已發布 .patch。同時真 daemon 重現 nested untracked repo 取消後資料遺失，保存前拒絕 nested repo／gitlink／特殊檔案，no-index 錯誤不算成功；補原始資料保留與一般子目錄／空檔／symlink 還原回歸（draft PR #143）。待重新驗證，未 merge。
- 2026-10-02 第九輪 verifier 另以真 daemon／shim 重現 `core.autocrlf=input` 取消後 CRLF 被封存為 LF；保存前檢查行尾轉換設定／attributes，無法保證原始 bytes 時保留 worktree／index 並回報 Failed，新增六個回歸（draft PR #143）。修正待全新 verifier 與 CI，未 merge。
- 2026-10-02 第九輪 verifier 核對 `f34c782` 的 Ubuntu CI，一次通過、一次 binary archive 還原舊 bytes；取消已刪原 worktree，不能以重跑綠燈抵銷。封存改從 blob／mode 重建無 stat cache 的私有 index，新增確定性 racy-file 機制回歸；正在驗證修正（draft PR #143），未 merge。

- 2026-10-02 第八輪 verifier 中斷前，真 daemon/shim 在 `f29e667` 重現 external diff／textconv 讓 archive 為空、取消仍刪 WIP；封存與 patch-id 改明確停用顯示轉換，補 staged／unstaged／untracked 還原回歸，包含 ignored 資料；content filter 無法保證原始 bytes 時保留原 worktree 回報 Failed。該輪沒有完整通過結論；修正交由新的 fresh-context verifier（draft PR #143），未 merge。

- 2026-10-02 第七輪 verifier REFUTED `a35b117`：真 shim 可設定 `skip-worktree`／`assume-unchanged`，diff 隱藏實際修改後取消會丟失 WIP；改以私有 index 副本清除旗標並封存，原 index 在失敗時完整保留。修正重新驗證中（draft PR #143），未 merge。

- 2026-10-02 第六輪 verifier REFUTED `5a20fb0`：真 shim 的 staged-only 檔案在工作目錄刪除後，取消時未存 archive；改分別保存 index／worktree patch，6 MiB bytes 與 AD 狀態還原通過，未解 index 衝突保留原資料；另拒絕單一 operator 無法完成的 human count≠1。前版雙平台 CI 全綠仍不視為驗收完成；修正重新驗證中（draft PR #143），未 merge。

- 2026-10-02 第五輪 verifier REFUTED `660dd08`：format-patch 略過 merge commit，取消後遺失其獨有衝突解法；補 first-parent merge diff 保存與真 git apply round trip。Ubuntu CI 通過，macOS CI 的 CLP-14 重現 instance-add 回覆早於 fleet 投影，改先發布 Starting；修正重新驗證中（draft PR #143），未 merge。

- 2026-10-02 第四輪 verifier REFUTED `9851bda`：6 MiB binary WIP archive 經診斷輸出 cap 截斷後仍刪原檔、checks 孤兒清理越過名稱命名空間。改用檔案串流、完整同步後發布與嚴格 task 前綴；macOS CI 另重現 handoff 測試重複送 auto-reviewer receipt，改暫停 reviewer。新增 bytes round trip／I/O 故障／foreign worktree 回歸；修正驗證中（draft PR #143），未 merge。

- 2026-10-02 Ubuntu CI 確認 sandbox 逃逸回歸通過；headless review 測試因自動 reviewer 搶先完成而漏讀中間狀態，改由暫停的 reviewer 驗證等待、重啟與人工回報，三個 context 回歸通過；重新跑 CI 與第四輪 fresh-context verifier（draft PR #143），未 merge。

- 2026-10-02 第三輪 verifier REFUTED `5887643`：計畫內容未傳遞、無 head 的 role review 失敗、merge proof 錯誤阻止 boot；補真程序回歸通過。另補 Linux 遮蔽 tmpfs 唯讀、Work 作者歷史投影，並以精確注入重現及修正 runtime 停止到 exec 前遺失 Ctrl-C；最終驗證重跑中（draft PR #143），未 merge。

- 2026-10-02 第二輪 verifier 另重現多 Branch 角色交接遺失前段 commits；以 core `BindingRelease::Handoff` 保留原 branch，真程序跨排隊／重啟的雙作者 merge 回歸通過，最終重新驗證待完成（draft PR #143）。
- 2026-10-02 第二輪 verifier 重現 Failed 派工仍占用 agent；補即時清理與每次 wake 重試、單 writer 與真 FS 故障回歸。Ubuntu CI 重現 `/tmp` 下 canonical repo 可寫，補明確唯讀掛載；修正重新驗證中（draft PR #143），未 merge。

- 2026-10-02 修正版 `360ca2d` 已 push 至 draft PR #143；完整 workspace 測試、clippy、check-deps（no_std 無跳過）通過，holder 回歸與 TUI 180 fd 限制通過；Gate 10 自動驗收與全新 verifier 待完成，merge 等使用者確認。
- 2026-10-02 第 10 施工關初輪 verifier 對 `2b7d4a7` 判定 REFUTED；修正多人審查、planned 交接、boot 隔離與 core ports，補整條 queue 的五種 fake 測試；重新驗證中，未 merge。worktree 統一放 `/Users/suzuke/AlphaCR-worktrees/`（[draft PR #143](https://github.com/suzuke/AgEnD/pull/143)）。
- 2026-10-01 在獨立 worktree 實作第 10 施工關：pipeline、冷 checks 沙箱、LocalForge recovery、team/workflow/agent 命令與 TUI attention；尚待 fresh-context verifier 和使用者驗收（[feat/gate-10-pipeline](https://github.com/suzuke/AgEnD/tree/feat/gate-10-pipeline)）。

每完成一件事加一行（日期 + 一行 + commit／PR），新的在上面。

- 2026-10-01 第 11 施工關 B 段 #140 squash merge（`462822a`）；最新 head 的 ubuntu／macOS CI 通過。2026-09-29 verifier 第 4 輪 CONFIRMED、T19–T35 已追認、使用者親自驗收 7 步通過；C 段（完整重現 agent CLI、滑鼠滾動）未開始，等第 10 施工關完成後提案。
- 2026-09-29 第 9 施工關 #136 merge（`f5c32ec`）；L1–L21 已追認、使用者親自驗收 9 步通過；第 1–9 施工關里程碑（兩個假 Codex agent 互傳、重啟不漏不重）已達成，第 10 施工關前置條件已滿足。
- 2026-09-29 第 13 施工關補版本漂移與 canary 規劃（#141，`15dd178`）；尚未開工。
- 2026-09-28 第 12 施工關 A 段（claude）開工前提案 draft PR #138 寫定；P1–P10 待使用者確認，提案未 merge、實作未開始。
- 2026-09-28 第 7 施工關 #132 merge（`7f326de`）；K1–K18 已追認、verifier CONFIRMED、使用者親自驗收 8 步通過（含授權 agent 跑真 codex 0.158.0）。第 11 施工關 B 段提案 #133 同日 merge（`2fbd53e`）。

- 2026-09-26 第 10 施工關開工前提案（#130，`0faad17`）merge；P1–P11 使用者已確認（P6 改成 checks 在寫入沙箱裡跑）；狀態改為提案中，等第 7–9 施工關完成再開工。
- 2026-09-26 第 9 施工關開工前提案（#129，`9178717`）merge；P1–P10 使用者已確認（P3 改成 `AGEND_HOME` 一律必須設、P6 對使用者叫 `name`、操作者也能 `task create`）；狀態改為提案中，等第 7、8 施工關都 merge 後開工。
- 2026-09-26 第 8 施工關 client（draft PR #131，`5470ba1`）merge；C1–C14 使用者已追認、使用者親自驗收 7 步通過、fresh-context verifier r1 CONFIRMED；狀態改為完成。
- 2026-09-26 第 7 施工關開工前提案（#126，`bf1415f`）merge；P1–P9 使用者已逐題確認（P2 推翻第 4 施工關 P8 的 `SpawnSidecar`、P4 選 A `ZDOTDIR` 並修改第 6 施工關 H3 白名單）；狀態改為實作中（開工於此提案之後）。
- 2026-09-26 第 11 施工關畫面層提前（#120，`d677f35`）merge；A 段（腳本假來源＋假 daemon）使用者親自驗收 5 步通過，T1–T18／G1–G4 使用者全部追認；狀態維持實作中（畫面層完成），B 段（接真 daemon）留給第 8 施工關之後。
- 2026-09-26 第 6 施工關 daemon-holder（#125，`3e28e06`）merge；P1–P9 使用者已確認、H1–H16 使用者已追認、fresh-context verifier r2 CONFIRMED；使用者親自驗收 9 步通過，狀態改為完成。
- 2026-09-26 第 6 施工關開工前提案（#119，`9955309`）merge；P1–P9（含 P3 夜間更正：daemon 起 holder 不能設 `process_group(0)`）使用者全部確認或追認；狀態改為實作中（branch `feat/gate-06-daemon`）。
- 2026-09-26 第 5 施工關 store（#122，`599a882`）merge；P1–P9 使用者已確認、S1–S23 使用者已追認、fresh-context verifier CONFIRMED；使用者親自驗收 10 步通過，狀態改為完成。
- 2026-09-26 第 4 施工關 holder（#118，`a467b54`）merge；P1–P9 使用者已確認、G1–G11 使用者已追認；使用者親自驗收 10 步通過，狀態改為完成。
- 2026-09-26 第 3 施工關 merge（#107，`6ead942`），verifier r13 CONFIRMED；使用者親自驗收通過，狀態改為完成。
- 2026-09-25 第 5 施工關開工前提案 P1–P9 使用者逐題確認；狀態改為提案中（等第 4 施工關完成後開工）。
- 2026-09-25 第 4 施工關開工前提案 P1–P9 使用者逐題確認；狀態改為提案中（等第 3 施工關完成後開工）。
- 2026-09-25 錄製器 + 真 CLI 一致性檢查（使用者決定）：錄下 claude、codex、opencode 各 5 個情境，假 agent 照錄製檔修正；第 7、12 施工關把一致性檢查列為必要完成條件（`feat/backend-recorder`，見 [RECORDER.md](../crates/agend-testkit/RECORDER.md)）。
- 2026-09-25 第 2 施工關使用者親自驗收通過，狀態改為完成。
- 2026-09-25 第 2 施工關 merge（#108，`6d7b540`）；verifier r6 CONFIRMED（規則表 56 條、80 個故意弄壞的實作、四次啟動的重啟生命週期）；狀態改為驗收中。
- 2026-09-25 第 1 施工關使用者親自驗收通過，狀態改為完成。
- 2026-09-25 第 1 施工關 merge（#105，`90794d4`）；verifier r6 CONFIRMED；狀態改為驗收中，等使用者親自驗收。後續 #106。
- 2026-09-25 第 1 施工關 verifier r5 推翻（dc2d6db）後修正：目前關卡的結果被作廢或換人產出就開新的 attempt 並重發要求；merge 送出中 branch 被重設時丟棄待處理的變更。
- 2026-09-25 第 1 施工關 verifier r4 推翻（02aca89）後改成結構性的事件身分：結果事件帶 stage、attempt、head，身分不符一律 `StaleResult`；merge 送出中只接受它的結果與 head 變更；pick 人數湊齊才定下。
- 2026-09-25 第 1 施工關 verifier r3 推翻（3e8b3a3）後修正：沒有 merge 的 workflow 也不允許最後的 branch work 之後再有 work；pick fanout 重跑要重新挑（4b05a60）。
- 2026-09-25 第 1 施工關 verifier r2 推翻（843a235）後改成結構性解法：存檔檢查以 `step` 做可完成證明、pick fanout 與 branch work 文法收斂、隨機 workflow 產生器成為常駐測試（ba30886）。
- 2026-09-25 第 1 施工關 verifier r1 推翻（832a4dc）後修正：merge／command／綁 head 的 approval 前面必須有產出 branch 的 work 且需要 repo；merge 送出後的 head 變更等 forge 結果（`MergeFailed`）（f458545、ebdac60）。
- 2026-09-25 rebase 到 v2（#104），第 1 施工關文件改用名詞表的詞（832a4dc）。
- 2026-09-25 使用者決定 D34–D37（`planned` workflow、對話式請示、請示排序、context recap），並核准 D32 擴充到 workflow 定義型別（以 golden TOML 測試鎖格式）；實作（7468ba0、d91865a）。
- 2026-09-25 第 1 施工關 fresh-context verifier 推翻幾個窄點，已修：merge 須為最後關卡、綁 head 的關卡須在最後的 branch work 之後、`on_fail` 只能指向 work、merge 送出後不可取消、`PipelineState` 不可偽造、第二個探索器（13c0dbc、6ae6364）。
- 2026-09-25 使用者決定第 1 施工關 Q1，記為 D33（一個 agent 一個 task、返工回 task 持有者）；`policy::assign` 照此改寫（ba1fe59）。
- 2026-09-25 使用者確認第 1 施工關提案 P1–P7，記為 D26–D32。
- 2026-09-25 第 1 施工關第 2 輪 review 修正：head 變更不跳過關卡、返工不遺失、要求修改退回 task 持有者、merge 門檻逐個關卡檢查、取消獨立狀態、佔位符存檔檢查、reviewer 同 backend fallback；新增狀態機探索器（2a6e29b、4f78b31、e867d52）。
- 2026-09-25 草稿原樣匯入 `feat/gate-01-core`（80c4de9）；草稿作者未經確認的 P1–P7 勾選先更正為未確認（b480311）。
- 2026-09-25 （草稿作者）review 修正 pipeline、assignment、protocol 相容與 check-deps 自我檢查；workspace tests（66 core tests、5 protocol compatibility tests）、clippy、check-deps、accept core 通過；待 fresh-context verifier 與使用者親自驗收（工作樹，尚未提交）。
- 2026-09-25 新增名詞表 docs/GLOSSARY.md；施工階段統一稱「施工關」、workflow 步驟稱「關卡」（#104）
- 2026-09-24 `agend-core` 草稿初次自動驗收通過：workspace fmt/clippy、49 core tests、no-std check 與 demo；草稿作者自行對照 P1–P7（不是使用者確認），P6 由第 5 施工關 Store 落地（工作樹，尚未提交）。
- 2026-09-24 在 P1–P7 確認前開始 `agend-core` 實作草稿（codex/gate-01-core 工作樹，尚未提交）。
- 2026-09-24 AGENTS.md 加入必守的 Git 工作流程：branch + worktree、只經 PR 合併（#103）
- 2026-09-24 第 1 施工關提案中（#102）
- 2026-09-24 README 系統圖改為 SVG（#101, e893877）
- 2026-09-24 CI 首次通過（ubuntu + macOS，8a5b0fd，[run 35979128418](https://github.com/suzuke/AgEnD/actions/runs/35979128418)）
- 2026-09-24 骨架與文件 push 到 v2（8a5b0fd）
- 2026-09-24 spike 完成（codex／claude／opencode + claude 追加；紀錄在 [research/](research/README.md)：[spike-codex](research/spike-codex.md)、[spike-claude](research/spike-claude.md)、[spike-claude-f](research/spike-claude-f.md)、[spike-opencode](research/spike-opencode.md)、[runtime-spike](research/runtime-spike.md)）

## 下一步

回 [ROADMAP](ROADMAP.md) 查看目前狀態。
