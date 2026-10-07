# 第 12B：OpenCode

> **TL;DR**
> - 依第 12 關持續授權實作；沿用 D16：queue 用 prompt_async，steer 視為 interrupt，abort 後送新工作。
> - holder 持有 serve 與 attach；daemon 經有密碼的 loopback API 管理既有 session，不用 PTY 輸入工作。
> - 下一步：完成傳輸／歷史核對、啟動與恢復、權限、三 backend 互傳及固定版本真測。

## 邊界

- 1.18.34 本機 `/doc`／health 已由隔離 serve 讀取，無 session／prompt，已清理；相較舊 1.18.31 錄製重新核 API，不把舊證據當新版本驗收。
- server 僅 bind 127.0.0.1，每個 instance 使用獨立密碼、資料與 session；禁止 HTTP redirect／環境 proxy。密碼不放 argv、不寫 log。
- POST 發出一次；先持久化 attempt。回覆遺失時只查原 session 的 messageID／完整內容，不以相同文字認領外來訊息、不盲目重送。
- SSE 不作唯一真相：重連與定期 REST 核 session／messages／status；permission 固定輪詢，不靠可能漏掉的事件。
- 尊重 backend 權限要求，未核准不執行；需要操作者的請求呈現在「需要你」，回覆只作用於同一 session／request。新 API `/permission/{requestID}/reply` 取代已標 deprecated 的舊 session route。
- daemon 重啟只重新連線；holder 死亡先清自有程序，保留原 session 恢復。未知或遺失 session 不偷偷建立替代上下文。
- 不改共享 OpenCode 設定／帳戶檔、不停止外來 serve；驗證只清自己的 namespace、程序、target、worktree。

## 實作中

傳輸層以 ureq 3.4 的有界 HTTP client 實作，固定五秒整體期限及 16 MiB JSON 上限；只接受自行組裝的 API path，拒絕 redirect、proxy 與路徑跳脫。歷史核對要求 session、user message id 與每個 part 歸屬一致，重複 id 或外來 part 拒絕。這兩部分已接原生假 OpenCode producer 測試，尚未接 supervisor／正式 Driver，不宣稱 12B 完成。

2026-10-07 真 1.18.34 隔離 `noReply` 捕獲證明 client 指定 messageID、中文與換行完整保留；零 assistant message，自有程序及目錄已清。捕獲輸出納入 parser 回歸；相同文字但不同 id、不符內容、外來 session、synthetic／ignored／額外 part 均不得誤認為確認。session API 已區分 POST 接受與歷史確認，resume 遺失 session 回錯，不建立新對話，也不因 status map 缺少 entry 就把遺失 session 視為 idle。

## 驗收

完成後須涵蓋：一次寫入與斷線對帳、人工訊息不能誤認、busy queue／interrupt、遺失 session、daemon／holder 重啟、permission 漏事件與過期回覆、跨 backend 互傳、全新 verifier、雙平台 CI。真測使用固定版本／模型／有限訊息與時間預算；舊結果不替代真測。

## 下一步

串接 holder 啟動、session 持久化與 Driver；先以原生 producer 驗證，再進行受控真測。
