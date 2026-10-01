# Gate 10 進度紀錄

> **TL;DR**
> - 此頁保存實作、驗證反例與修正的歷史紀錄。
> - 狀態與驗證證據見 [Gate 10 入口](gate-10-pipeline.md)。
> - 下一步：回 [Gate 10 入口](gate-10-pipeline.md) 依目前狀態驗收。

## 進度紀錄

- 2026-10-02 文件 verifier r13 在 `ea975c3` 找到索引的 target／PATH 與人工驗收頁不一致（REFUTED）；統一 CARGO_TARGET_DIR 與 binary PATH，實際初始化選到 `agend 0.0.0`。其餘文件、證據與產品 tree 核對通過，產品碼不變；人工驗收及 merge 仍待使用者（draft PR #143）。
- 2026-10-02 全新 verifier r12 CONFIRMED `dfe5bc6`：完整 workspace 771 passed／2 ignored、accept 713 passed／2 ignored與三組組合探測通過；原 PATH／等待 fixture 失敗保留。push／PR 共四個 Ubuntu／macOS CI job 成功。README、ROADMAP 與 Gate 10 驗收文件已同步／分頁，待人工驗收、未 merge（draft PR #143；[證據](gate-10-verification.md)）。
- 2026-10-02 第十一輪 verifier 的六項本機 mandatory 通過（workspace 767 passed／2 ignored，accept 含四次開機），但 novel 以真 daemon／shim 重現強制 color 使 archive 無法 git apply、不完整 nested metadata 遺失 staged bytes，故仍 REFUTED；另重現 diff.noprefix 問題。封存／patch-id 固定無色 a/／b/ 全 repo diff，完整 filesystem 掃描 Git metadata；新增原 bytes 還原／保留回歸。修正待全新 verifier／CI，未 merge（draft PR #143）。
- 2026-10-02 `29fb454` 的 Ubuntu push／PR CI 在 nested repo fixture 建 commit 時因沒有 local Git identity 失敗；補 user.name／email 並以沒有全域 Git 設定的環境重跑。產品碼不變，原紅燈保留；修正仍待 CI／全新 verifier，未 merge（draft PR #143）。
- 2026-10-02 第十輪 verifier 的 unresolved-index 回歸失敗，其目錄數量斷言也包含重試中 staging；修正為檢查已發布 .patch。同時真 daemon 重現 nested untracked repo 取消後資料遺失，保存前拒絕 nested repo／gitlink／特殊檔案，no-index 錯誤不算成功；補原始資料保留與一般子目錄／空檔／symlink 還原回歸（draft PR #143）。待重新驗證，未 merge。
- 2026-10-02 第九輪 verifier 另以真 daemon／shim 重現 `core.autocrlf=input` 取消後 CRLF 被封存為 LF；保存前檢查行尾轉換設定／attributes，無法保證原始 bytes 時保留 worktree／index 並回報 Failed，新增六個回歸（draft PR #143）。修正待全新 verifier 與 CI，未 merge。
- 2026-10-02 第九輪 verifier 核對 `f34c782` 的 Ubuntu CI，一次通過、一次 binary archive 還原舊 bytes；取消已刪原 worktree，不能以重跑綠燈抵銷。封存改從 blob／mode 重建無 stat cache 的私有 index，新增確定性 racy-file 機制回歸；正在驗證修正（draft PR #143），未 merge。

- 2026-10-02 第八輪 verifier 中斷前，真 daemon/shim 在 `f29e667` 重現 external diff／textconv 讓 archive 為空、取消仍刪 WIP；封存與 patch-id 改明確停用顯示轉換，補 staged／unstaged／untracked 還原回歸，包含 ignored 資料；content filter 無法保證原始 bytes 時保留原 worktree 回報 Failed。該輪沒有完整通過結論；修正交由新的 fresh-context verifier（draft PR #143），未 merge。

- 2026-10-02 第七輪 verifier REFUTED `a35b117`：真 shim 可設定 `skip-worktree`／`assume-unchanged`，diff 隱藏實際修改後取消會丟失 WIP；改以私有 index 副本清除旗標並封存，原 index 在失敗時完整保留。修正重新驗證中（draft PR #143），未 merge。

- 2026-10-02 第六輪 verifier REFUTED `5a20fb0`：真 shim 的 staged-only 檔案在工作目錄刪除後，取消時未存 archive；改分別保存 index／worktree patch，6 MiB bytes 與 AD 狀態還原通過，未解 index 衝突保留原資料；human count=2 會永久等待，runtime admission 改拒絕 count≠1。前版雙平台 CI 全綠仍不視為驗收完成；修正重新驗證中（draft PR #143），未 merge。

- 2026-10-02 第五輪 verifier REFUTED `660dd08`：format-patch 略過 merge commit，取消後遺失其獨有衝突解法；補 first-parent merge diff 保存與真 git apply round trip。Ubuntu CI 通過，macOS CI 的 CLP-14 重現 instance-add 回覆早於 fleet 投影，改先發布 Starting；修正重新驗證中（draft PR #143），未 merge。

- 2026-10-02 第四輪 verifier REFUTED `9851bda`：6 MiB binary WIP archive 經診斷輸出 cap 截斷後仍刪原檔、checks 孤兒清理越過名稱命名空間。改用檔案串流、完整同步後發布與嚴格 task 前綴，新增 bytes round trip／I/O 故障／foreign worktree 回歸；修正驗證中（draft PR #143），未 merge。

- 2026-10-02 Ubuntu CI 確認 sandbox 逃逸回歸通過；headless review 測試因自動 reviewer 搶先完成而漏讀中間狀態，改由暫停的 reviewer 驗證等待、重啟與人工回報，三個 context 回歸通過；重新跑 CI 與第四輪 fresh-context verifier（draft PR #143），未 merge。

- 2026-10-02 第三輪 verifier REFUTED `5887643`：planned summary/output 未交人工核准與 dev、無 head role review 誤建 git worktree、main ref 損壞時 merge proof 中止整個 boot；已補成果內容、logical review 與逐 task 隔離，真程序回歸通過。Linux CI 的 tmpfs 父目錄另改唯讀；CLI Ctrl-C 精確注入重現 runtime→exec 訊號窗口，補 signal-context flag。完整最終驗證待完成，未 merge（draft PR #143）。

- 2026-10-02 第二輪 verifier REFUTED：Failed 派工仍占 capacity、多 Branch 向前角色交接遺失 commits；補 terminal cleanup 每次 wake 重試與保留 branch 的 handoff，單 writer、真 FS 故障、雙作者跨排隊／重啟 merge 回歸通過。另修 Ubuntu CI 的 `/tmp` canonical repo 唯讀掛載；最終驗證與使用者驗收待完成，未 merge（draft PR #143）。

- 2026-10-02 初輪獨立驗證對 `2b7d4a7` 判定 REFUTED（2 High、3 Medium）；修正多人審查的 recipient id、planned 回報與角色交接、缺 git 與逐 task boot 失敗隔離，補 core ports 與五種 fake 的完整 queue 測試；修正 recorder 通知 drain、PTY 快速退出輸出，重新驗證中。實作與 verifier worktree 移至 `/Users/suzuke/AlphaCR-worktrees/`（[draft PR #143](https://github.com/suzuke/AgEnD/pull/143)）。
- 2026-10-01 在獨立 worktree 接通 pipeline、LocalForge、沙箱、持久化快照與命令；新增真程序／adapter／snapshot 驗證。尚待 fresh-context verifier 與使用者親自驗收（`feat/gate-10-pipeline`）。

日期 + 一行 + commit／PR，新的在上面。

- 2026-10-01 核對前置條件：第 7 施工關 #132、第 9 施工關 #136 已完成並 merge；第 11 施工關 B 段 #140 已 merge。本關提案 #130 已確認並 merge，可以開工；實作尚未開始。

- 2026-09-26 沙箱第 3 輪 review（HIGH：經 symlink 連到 codex app-server）後改成預設拒絕：macOS `(deny network-outbound (remote unix-socket))`，只留 DNS 的 mDNSResponder（本機實測：socket 與 symlink 都被擋、HTTPS 200）；Linux 用 `--tmpfs` 藏 `/tmp`、`/run/user/<uid>`、`$AGEND_HOME/run`，其他看得到的 socket 列為已知風險；需要 unix socket 的 checks 列為已知限制；setsid 與測試的說法跟著改。
- 2026-09-26 沙箱第 2 輪 review（1 MEDIUM、2 LOW）後修正：沙箱擋掉 `$AGEND_HOME/run/` 的 unix socket（macOS profile `deny network-outbound`、Linux `--tmpfs` 與 `--unshare-pid`），每次 check 後停掉整個 process group；標記檔只 `lstat`、被 check 刪掉算 checks 失敗；`agend doctor` 的沙箱一列改由本關加。
- 2026-09-26 沙箱 review REFUTED（1 HIGH、1 MEDIUM、數個 LOW，在 macOS 重現）後修正：`.git/worktrees/<run>/` 與 `<worktree>/.git` 改成唯讀、daemon 碰 checks worktree 的 git 加 `-c core.fsmonitor=false`（擋逃逸）；拿掉跨 checks 的共用快取，全部放每次的暫存目錄（擋假綠，代價是慢）；macOS `mktemp` 與真實路徑、每次 check 的標記檔（沙箱自己失敗算 `sandbox-missing`）、CI 先裝 `bubblewrap`；範圍與分工表補上 `sandbox-missing` 與 `agend doctor`。
- 2026-09-26 使用者確認 P1–P11：P6 改成 checks 在寫入沙箱裡跑（macOS `sandbox-exec`、Linux `bwrap`，網路照常，沒有工具就不跑、出現 `sandbox-missing`）；P3 拒絕刪除持有者（與 D33 不同）、P7 `--ff-only` 與手動 merge 記成完成、P8 `no-role`（與 D18 不同）、P11 `delivery = inbox` 只給假 agent，都照提案；要正式的 `agend task cancel`（語法第 9 施工關、daemon 端本關）；「你親自驗收」加一步沙箱，改成 11 步。
- 2026-09-26 跟上第 9 施工關的使用者決定與已 merge 的第 7、8 施工關：操作者可以 `agend task create`（要 `--team`），驗收改用它、拿掉 `pipeline_probe task` 與那題待你決定；每個步驟都設 `AGEND_HOME`；使用者看到的 instance 寫成 name；migration 編號（`0003` 第 8、`0004` 第 7、本關下一個空號）與協定版本（1.1 第 8）更新。
- 2026-09-26 第 4 輪 review（1 MEDIUM、2 LOW）後修正：分派把整個 team 的 running instance 都交給 core（`held_task` 照實填），返工與 `NoEligibleReviewer` 才正確；逾時計時器由誰排、開機重報的通知逾時回 `StaleResult` 是正常；`command` 的 `on_timeout` 也在 task create 與 `workflow check`／`apply` 被拒。
- 2026-09-26 第 3 輪 review REFUTED（2 MEDIUM、3 LOW）後修正：手動 merge 記成完成標出與 pipeline.md 不同、改寫成「最舊一個包含 head 的 commit」；開機逾時排除 merge 關卡；`command` 關卡寫 `on_timeout` 在建立時被拒；`no-role` 只在缺角色或只有作者時出現。
- 2026-09-26 第 2 輪 review REFUTED（2 HIGH、2 MEDIUM、4 LOW）後修正：手動 merge 改記 `MergeCompleted`（`StageFailed` 在 merge 送出中會被 core 拒絕）；checks 逾時改餵 `CommandFinished{exit_code: None}` 回 work，demo workflow 寫明 timeout；請示的 attention id 用 ask id；merge 中不能取消、`merge-blocked` 的出路；`delivery = inbox` 與 `--ff-only` 標出與架構頁不同；假 agent 固定 `claude` backend；步驟 6 改用 `find`；`task create --role` 與審查者排除作者。
- 2026-09-26 fresh review REFUTED（2 HIGH、7 MEDIUM、6 LOW）後修正：與第 9 施工關的分工寫進範圍（請示、task 類命令的 daemon 端、`agend team`／`workflow` 歸本關）；派工訊息與「需要你」用 ticket；`inbox` 讀取不算確認；驗收用 `pipeline_probe task` 開 task、`g10h` 放停住的 task 並可取消；空 branch 在 `done` 被拒、已 merge 靠 trailer 找回；demo 審查路徑一致；checks 每次新 worktree；D33 衝突標出；timeout、`waiting_since`、每日對帳、`worktree list` 檢查、rebase 措辭；failpoint 只留兩個；新增 P10（本關的命令）與「待你決定」。
- 2026-09-26 開工前提案 P1–P10 寫定（draft PR），待使用者確認；「你親自驗收」改成 10 步；狀態改為提案中。

## 下一步

回 [Gate 10 入口](gate-10-pipeline.md) 依目前狀態驗收。
