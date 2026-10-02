# 第 11 施工關 C 段：實作進度

> **TL;DR**
> - [draft PR #145](https://github.com/suzuke/AgEnD/pull/145) 持續實作完整終端；目前完成 holder／runtime 基礎，C 段尚未完成或驗收。
> - 已接通 frame／歷史及實際 resize／input ack；daemon client 1.4、完整模式、鍵鼠／貼上、Codex U17 仍待完成。
> - 下一步：完成 daemon／client／fake 契約與 TUI，再跑完整驗收、全新 verifier 與逐步人工驗收；merge 等使用者確認。

## 已實作

| 區塊 | 行為 | 驗證來源 |
|---|---|---|
| 共用型別／holder 1.1 | 結構化 cells／color／style／cursor／mode、generation／revision、viewport request id；1.0 wire 保留 | core／protocol；holder 真 PTY／socket |
| holder 歷史 | 同一個 alacritty parser、normal 1,000 列絕對 row id、固定 viewport／淘汰 clamp；alt 無歷史，resize reflow 重編 row id | `terminal_frames` 與 parser 比對 |
| frame 上限 | 含換行最多 8 MiB，超限整份拒絕；不截斷成功，holder 請求仍最多 1 MiB | holder oversized frame、runtime 邊界／partial／EOF |
| holder 控制 | Acquire／Resize／Input／Release 共用 PTY FIFO；實際 resize 加完整 frame 才 grant、實際 write／flush 才 ack；寫前驗 owner／generation | `server` 的 5 個 native control cases、writer barrier |
| 原生 PTY 壓力 | raw agent 不讀 stdin 時，整次 write 最多 5 秒；明示可能部分寫入、不重送，新 grant 等舊 write 結束 | native backpressure case |
| runtime 背景操作 | holder 能力／連線 epoch、request id 配對、有界佇列與 pending；失效請求不送新連線 | runtime unit、真 holder 的 `terminal_runtime` |
| 取消競態 | 即使 grant 回覆已到、consumer 未接收就取消，也作廢連線；取消唯讀查詢則不打斷控制 | 真 holder 的 unconsumed-grant／readonly cancellation cases |

## 驗證紀錄

- `a13d31c`：holder frame／歷史基礎 47 passed，`accept core` 通過；`24d107a` 記錄結果。該 head 的 push／PR CI 已通過。
- 2026-10-02 控制與 runtime：holder 53 passed；完整 daemon crate 測試通過。真程序 holder_process 7、holder_runtime 4、terminal_runtime 5 passed；runtime units 17 passed，含 legacy write lock 壓力。client protocol 9 與 TUI 2 個回歸通過；初跑缺 fake_codex example 的失敗保留，補建後重跑成功。最新 workspace clippy／實際 no-std 與 accept core 通過，兩個既有 deep explorers ignored，沒有 SKIPPED；結果記於 #145；C 段尚未獨立／人工驗收。
- 原取消邏輯在真 holder 回歸失敗：`an unconsumed grant survived cancellation`（exit 101）；修正後同一測試通過。
- 大行讀取原失敗與 partial fixture 的小 socket buffer 死鎖保留；最新讀取採 chunk 線性掃描、deadline poll／recv，關閉後仍讀完資料。

本機原始輸出與 SHA256 在 `/private/tmp/g11c-implementation-logs`；失敗 log 保留，不算通過證據。原生 macOS 測試不代表 Linux 或實際終端字型／游標外觀已驗收，雙平台 CI 與人工驗收仍須核最新實作 head。

## 尚待完成

- client 1.4 additive 型別／能力檢查；NEEDED 保留 1.3。
- daemon caller／attach 綁定、多視窗控制與 EOF release、每個 instance 的 50 ms dirty frame 更新；同套契約跑 fake／真 daemon。
- TUI 完整 renderer、明確 i 入口／退出、resize ack 前停輸入、斷線／控制權失效立即唯讀。
- mode-aware keys／mouse／paste、固定歷史 viewport、外層 capture／paste 的錯誤與 unwind 恢復。
- fake Codex＋真 AgEnD U17；明確 opt-in live smoke、版本及使用者確認後才開放該版本，未驗仍 not_supported。
- 完整 accept tui、workspace checks、雙平台 CI、全新無 context verifier、逐步人工驗收與 merge 確認。

## 下一步

實作剩餘項目並依 [驗收矩陣](gate-11c-validation-plan.md) 逐項取得證據。此頁的局部通過不能代替 C 段完成驗收。
