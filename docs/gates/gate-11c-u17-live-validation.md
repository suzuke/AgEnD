# 第 11 施工關 C 段：真 Codex U17 首次驗證

> **TL;DR**
> - 使用者明確核准後，以 Codex CLI 0.159.3／gpt-6-luna／low 執行四個模型回合，U17 工具通過。
> - 同 thread／holder、忙碌 Queue、idle Send、重啟後上下文與兩個自己的 receipt 均有原始證據；此 run 的正式輸入仍未開放；後續已同意只開放 0.159.3。
> - 下一步：原 run 已由全新 verifier 核實，使用者同意只開放 0.159.3；後續行為驗證已改採自動化，merge 仍待確認。

本頁保存首次四回合的歷史 run；原始結果與限制不改作新 head 認證。後續版本開放見 [輸入政策](gate-11c-codex-input.md)，使用者實機紀錄與自動驗證方式見 [驗收收尾](gate-11c-closeout.md)。

## 固定範圍

- 日期：2026-10-03（本機 Asia/Taipei）；原 log 使用 UTC。
- 實作 head：7b9082d0a7753f20ae27cb25176e35feafbefc86。
- 路徑：[工具](../../crates/agend/examples/codex_u17_live.rs)／[情境](../../crates/agend/tests/common/codex_u17_live.rs) → 真 App／ClientSource／client／daemon 子程序／holder／production wrapper／PTY／remote Codex／SQLite。
- 寫入沙箱：/Users/suzuke/Documents/Hack/AgEnD-ops/record-sandbox.sh。home 僅 /tmp/g11live-96406-0/h1；沒有額外真模型重跑授權。
- thread：01a0ff06-2a03-75f2-9201-3aba181e2032；holder pid 96427，兩次 daemon boot 都使用它。

## 原始結果

| 項目 | 證據 |
|---|---|
| 人工多行輸入 | turn 01a0ff06-3411-7632-9838-1306b852fd7a；真 driver 觀察 busy／idle，inbox 為空 |
| busy 正式 Send | UUID 尾碼 117 走 thread/queue/add；自己的 clientId 對應 turn 01a0ff06-6692-72a3-80a4-02a5c0d66041，inbox 只有一列 |
| daemon 重啟 | pid 96426 → 96587；holder／frame generation／thread 不變，App 重連唯讀，須明確 i |
| 實際上下文 | 人工 recall turn 01a0ff06-702c-7a83-a71b-73f11c6cb4d8 回覆 G11U17_96406，與第一個 prompt 的 code word 相同 |
| idle 正式 Send | UUID 尾碼 118 走 turn/start；自己的 clientId／durable turn 為 01a0ff06-7620-73d2-ab54-5be9d9135af0 |
| 冪等與 receipt | 117 重送印 already confirmed; not sent again；SQLite 僅兩個 Confirmed rows，turn id 各自正確；人工 turn 不增造 row |
| 清理 | App reader threads 為 0、owned holder 清理；獨立 ps 核四個自建 pids 都不在，lab 目錄已移除 |

四個回合都已啟動。原 Codex session 有前三個完成回覆；第四個在 user item／receipt 確認後即停止 holder，沒有等待或斷言 IDLE-OK 最終回覆。這次驗的是 U17 送達與人工輸入安全，不宣稱四個模型回覆均完成。

## Log 中的限制

- 原 log 的 thread/queue/start 回 queue is empty：Codex 已自動消耗 queued item，隨後自己的 identified turn 確認；沒有追加重送或第二個 row。保留原錯誤行，供 verifier 核對競態。
- record-sandbox.sh 內再啟 checks 的 sandbox-exec 被拒絕，兩次 boot 都印 checks unavailable。這次沒有跑 pipeline checks，不把 U17 通過算成 runner 沙箱通過。
- App 事件由工具送入；實體鍵盤、Terminal／iTerm2／Linux 外觀與非美式鍵盤仍由人工驗收。
- 此 run 的正常 daemon 使用 CodexInputPolicy::default()，回 not_supported；這次只有指定 instance 的診斷 daemon 可以輸入。使用者核准四回合，不等於核准版本開放或 merge。

## 可核對原始檔

/private/tmp/g11c-implementation-logs/u17-live-r1/ 保存 build／live log、選取的四則測試訊息與三則回覆、事件時間、原 session path／SHA256，以及兩個執行檔 SHA256。完整 session 仍在本機 Codex sessions；選取證據排除環境注入訊息，不以它冒充第五個模型回合。SHA256SUMS-u17-live-r1 固定本批 source／證據。

實作 head 7b9082d 的 push／PR Ubuntu／macOS 四個 CI jobs 均成功，各 892 passed／2 個既有 ignored，實際 no-std 通過；SHA256SUMS-ci-7b9082d 七 entries 保存原 logs／metadata／案例核對。新文件 head 的 CI 另核。

## 後續確認

固定 68e15c0 已由全新 verifier 核實：accept tui 597 passed／0 ignored，其餘 workspace 295 passed／2 既有 ignored，fmt／clippy／實際 thumb no-std 通過；文字 receipt mutant 被拒絕。四個 CI jobs 均成功。使用者同意只開放 0.159.3；[新實作](gate-11c-codex-input.md) 需另驗。

## 下一步

核 [驗收收尾](gate-11c-closeout.md) 的固定 head verifier／CI、自動驗證與清理結果；#145 merge 仍等使用者確認。
