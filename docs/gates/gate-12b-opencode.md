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

傳輸層以 ureq 3.4 的有界 HTTP client 實作，固定五秒整體期限及 16 MiB JSON 上限；只接受自行組裝的 API path，拒絕 redirect、proxy 與路徑跳脫。歷史核對要求 session、user message id 與每個 part 歸屬一致，重複 id 或外來 part 拒絕。這兩部分已接原生假 OpenCode producer 測試，已接 supervisor／正式 Driver 的初版；尚未完成 holder 整合與權限驗收，不宣稱 12B 完成。

2026-10-07 真 1.18.34 隔離 `noReply` 捕獲證明 client 指定 messageID、中文與換行完整保留；零 assistant message，自有程序及目錄已清。捕獲輸出納入 parser 回歸；相同文字但不同 id、不符內容、外來 session、synthetic／ignored／額外 part 均不得誤認為確認。session API 已區分 POST 接受與歷史確認，resume 遺失 session 回錯，不建立新對話，也不因 status map 缺少 entry 就把遺失 session 視為 idle。

## 驗收

啟動封裝已具備 holder 綁定版本／endpoint、原子 session handoff、私人目錄與不進 argv 的密碼；新 holder 輪替密碼，避免舊請求打到重用 port，daemon 單純重連則讀原紀錄。SQLite 在既有 messages 表以原子條件更新取得一次投遞資格，保存 session／message 綁定；關閉再開後，結果不明的 attempt 仍禁止重送。這些基礎通過八個相關測試；supervisor 與正式 Driver 已串接初版；權限與完整恢復流程尚待驗證。

完成後須涵蓋：一次寫入與斷線對帳、人工訊息不能誤認、busy queue／interrupt、遺失 session、daemon／holder 重啟、permission 漏事件與過期回覆、跨 backend 互傳、全新 verifier、雙平台 CI。真測使用固定版本／模型／有限訊息與時間預算；舊結果不替代真測。

2026-10-07 daemon worker 已保存原 session、先記 attempt 再 POST，以 REST 原生歷史確認後發布持久事件；斷線不重送。舊 attempt 採每頁 128 筆循環核對，新工作獨立取 32 筆，140 筆 unknown 前綴不阻塞新工作。REST 錯誤立即轉 unknown；idle 需持續五秒。啟動前拒絕改寫仍存活 holder 的私人檔案；缺 session 不允許 resume。daemon 單元測試 108 項通過；這不等於 holder／真模型完整驗收。

權限 API 已依本機 1.18.34 `/doc` 接上新版 reply route；回覆前比對 session 與完整原始請求，只支持 once／reject。migration 0010 保存觀察、決策與單次 HTTP attempt；重開 SQLite 再觀察同一請求不重設 attempt。原生 producer 測試覆蓋外來 session、內容變更、重複 id、過期回覆與重啟後不能再 claim。worker 已輪詢保存；「需要你」發布與 operator-only AnswerAsk 已接線。只接受明確選項，free text 拒絕；回覆不明時保留無重送動作的提示。完整 holder／協定端到端真測尚未完成。

原生 `opencode_bridge` 兩個整合案例已通過：真 daemon／holder／wrapper／socket，daemon 重啟保留 holder 與 permission ask；operator 拒絕一次、重複拒絕失敗；正常停止或 SIGKILL 自有 holder 後，以原 session 恢復且歷史僅一筆 user message。結束確認 server port 關閉，lab 目錄已清。未知 process group 不清除 attribution、不覆寫 runtime 檔案；attach 以完整 URL＋session 辨識。這些是零模型 fixture 證據，尚未完成真 OpenCode／三 backend 互傳驗收。

Driver 事件已補 REST terminal assistant 回填（中途 tool-calls 不算回合完成）、持久 busy／idle 與 HTTP 429 usage 訊號。schema 0011 另保存已觀察 message id，事件 14 天清理後不重新發布同一回合。204 接受先記 Sent，完整歷史核對後才 Confirmed；running instance 的新投遞最多等五秒取得真實回條，不偽造 Sent。原生測試覆蓋正常完成、abort、外來 assistant part 拒絕與事件刪除後不重複回填；完整 DRV suite 10/10 已通過（實際 Driver／Worker、原生 REST producer、每次 boot 重開 SQLite）；429 原生錄製真證據仍待完成。

固定 SHA-256 的真 1.18.34 已完成零 session／零 prompt provider inventory：可用 opencode-go 與 openrouter，測試帳戶只複製至私有 namespace，原始 auth digest 不變，自有 process group／目錄已清。證據在 `AgEnD-ops/g12b-api-20261007/model-inventory`；尚未送模型 prompt。

真模型 smoke v2 已通過：固定 1.18.34／`opencode-go/gpt-6-luna`，正式 daemon→holder→REST→模型送一則訊息，原生歷史僅一筆 user，回覆 `AGEND_G12B_MODEL_OK`；DB 有 Confirmed 與單一 TurnCompleted。v1 在送出前因驗證腳本讀取獨占 SQLite 失敗；v2 改以 API 觀察，停止後讀 DB。初次清理漏 `--yes`，已重新接回 daemon 刪除本次 instance，確認 port 關閉、程序及目錄消失、共享 auth 未改。證據 `AgEnD-ops/g12b-api-20261007/model-smoke-v2`，原生歷史已納入零模型回歸；只證明一則基本回合，不替代 busy／跨 backend 驗收。

OpenCode 同樣使用 daemon 的 `ZDOTDIR`，避免 login zsh 的系統 profile 把 shim PATH 移到後方。沿用共享 `.zprofile`，不改使用者 shell 設定。

原生 busy queue→steer／interrupt 反例先重現失敗：abort 後 backend 立即開始既有排隊工作，等待 idle 五秒會把新訊息留成 unknown。修正為 abort 成功回覆後單次提交，不要求可觀察的 idle 空窗；測試核三筆完整 user message、只有首輪被中斷、各筆 Confirmed 與後續 tick 無重送。此為既有原生 producer 邊界證據；1.18.34 真 busy smoke v2 已核三筆 Confirmed、首輪 aborted 與最後 interrupt 回覆，程序及暫存已清，共享 auth 不變；queued user 沒有獨立 assistant 完成，不把收件與回合完成混為一談。v1 在 busy 尚未發布時過早斷言停止，未重送；兩份結果均保留。

## 下一步

完成 holder 原生整合、權限請求與恢復測試，再進行受控真測與 fresh verifier。
