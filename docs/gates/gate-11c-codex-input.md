# 第 11 施工關 C 段：Codex 0.159.3 輸入開放

> **TL;DR**
> - 真 U17 原始證據經全新 verifier 核實後，使用者同意只開放 Codex CLI 0.159.3；未知或其他版本仍唯讀。
> - 版本許可綁 holder 的實際啟動記錄；thread 曾允許人工輸入後永久只用自己的 clientId 對帳。
> - 下一步：核本批新 head 的完整 checks／CI 與全新 verifier，再逐步人工驗收；merge 與額外真模型另行確認。

## 當下輸入許可

production daemon 使用 CodexInputPolicy::approved()；Default 仍拒絕所有版本。固定 wrapper 在 app-server 啟動前呼叫同一 executable 的 --version，成功輸出與 holder PID 以 0600 暫存檔／rename 寫入 run/holders/<id>.codex-version。版本字串只接受 codex-cli 0.159.3；其他、空白、預覽版或多行未知輸出都拒絕。

daemon 接回 holder 時讀原記錄並核正在持有 lock 的 holder PID，不改查目前磁碟上的新版 executable。重新 Spawn 前清除舊版本記錄；沒有記錄的舊 holder 保持唯讀，需重啟 instance 才能辨識。這是防止意外版本混用的門檻，不是防止刻意偽造 executable 的安全邊界。

Acquire／Input 與 legacy terminal_input 都經 live driver 核准，agent caller 仍 forbidden。driver 沒有連線、holder 身分改變或歸屬無法存入 DB 都不給輸入。RPC transport 重連撤銷原許可，保持唯讀；重新啟動 daemon 後重新核 holder 記錄，再按 i 取得控制。原 owner／generation／FIFO 與 resize 完成確認規則保持有效。

## 永久 thread 歸屬

migration 0006 新增 codex_input_threads(thread_id)，沒有 instance 外鍵，永久保留；同一 thread 可能換 instance 或換版本接回。獲准輸入時先 INSERT OR IGNORE 並 commit，再寫 GO 啟動 frontend、再 reconcile；DB 失敗不開放輸入。

接回既有 thread 時，在 thread/resume 前讀永久事實，避免 resume 期間的 notification 使用舊文字 fallback。Worker live／reconcile／flush 與 Driver.events 都用這個事實；版本失去許可時不刪除、不降級。人工 item 沒有 clientId 或帶別人的 ID，不能確認 daemon row，即使文字相同且 row 已記 attempted_at。

未曾允許人工輸入的 legacy thread 保留既有 lost-reply 契約；本次沒有開放其他 CLI 版本，也沒有增加真模型回合。

## 原生回歸

所有新測試都使用真 SQLite 與 holder／wrapper／PTY／fake Codex producer，不呼叫模型。

- 一般 daemon 的完整 App 路徑開放 0.159.3：人工 busy／idle、Queue／idle Send、兩個自己的 durable receipts、重啟、草稿、caller／其他版本與清理。
- holder 存活期間修改 executable 的 --version 輸出為 0.159.4，重啟 daemon 仍以原啟動的 0.159.3 辨識。
- 0.158.0／0.159.4／未知輸出保持唯讀；缺失或過期 holder PID 記錄也拒絕。
- 人工 producer 產生 clientId=null 的相同文字，attempted Queued row 重啟後仍不誤確認；新的 own-clientId turn 才能確認。
- DB 拒寫不產生 GO／輸入許可；歸屬讀取失敗在 resume 前拒絕。
- store 歸屬跨重開、重複插入與 prune 保存；只有歸屬資料的 DB 也會做 snapshot；v1–v6 fixtures 前向升級到 SQLite-produced golden。

本批 targeted 回歸：codex_u17 11、daemon lib 79、既有 Driver 契約 15、store 40 passed。初跑的測試封裝錯誤、fixture 外鍵順序與缺 retention 反例保留於 /private/tmp/g11c-implementation-logs/version-input-*；修正後 regressions-r3 全部通過。完整 acceptance 初跑另抓到兩個舊拒絕訊息 consumer：tui_daemon 的逐字斷言與 client_source 的 U17 字串斷言；版本拒絕 code／路由保護保持有效，訊息斷言同步為 approved CLI 0.159.3。原失敗保留於 accept-r1／r2；完整 acceptance／新 head CI／獨立報告另核。

## 原證據與同意

首次真 U17 使用四個已核准的 gpt-6-luna／low 回合，前三個有最終回覆，第四個只核自己的 receipt。[原範圍](gate-11c-u17-live-validation.md) 與四回合授權不擴大；固定 68e15c0 的 verifier report 在 /private/tmp/g11c-u17-live-verifier-r1/report.md，不冒充這次版本開放的新 head 認證。

使用者於 2026-10-03 明確「同意」只開放 Codex CLI 0.159.3。[D39](../decisions/d39.md) 保存確認；#145 仍為 draft，人工驗收與 merge 等另行確認。

## 下一步

全新 verifier 在自己的固定 head worktree 重跑、嘗試推翻版本與永久歸屬；通過後一次一步提供人工驗收指令。
