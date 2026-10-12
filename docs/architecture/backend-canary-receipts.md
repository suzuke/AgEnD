# Backend canary 收據與版本探測

> **TL;DR**
> - canary 核對正式投遞收據及完成 outcome，不以 idle 代替模型成功。
> - 每次三訊息、固定 artifact／build；失敗或清理未完成不能准入。
> - 認證來源見 [canary 認證](backend-canary-auth.md)，真測範圍見 [目前狀態](../gates/gate-13-status.md)。

## Canary 收據查詢（1.7）

操作員 `message_delivery` RPC 讀取 daemon 保存的 queued／sent／confirmed／failed，
保留訊息 ID、sender／target、turn ID 與 attempted／updated 時間；不存在回 None。
agent 禁止使用，回覆不含訊息 body。查詢不會推進任何狀態。
client 使用一次性、有共同期限的 exchange；舊 daemon 拒絕此能力，未知狀態不算 confirmed。
這個 API 供 `backend canary --allow-model` 對帳；只有完整通過且綁目前 build／artifact 的收據才可用於新啟動准入。
目前隔離 daemon home 與程序 HOME、核對 `--version`，並先發布失敗狀態的 canary.json；不改 fleet 或 active version。
版本探測有 5 秒期限、8 KiB 輸出上限；工作期限與結束清理期限分開。
版本探測只允許執行緒，不允許建立子程序：macOS 使用 sandbox-exec 的 process-fork 拒絕規則，
Linux x86_64／aarch64 使用 seccomp，拒絕 fork／vfork 與非執行緒 clone；clone3 回 ENOSYS，允許 libc 回退到可檢查旗標的 clone。
架構不符、工具缺失或限制安裝失敗時拒絕探測，不降級成無限制執行。
這只保護探測程序的生命週期，不是檔案／網路沙箱；不修改父程序、其他 session 或系統服務。
探測程序即使自行切換 session，也透過未回收的 Child PID 停止；回收後不再送訊號。
原生測試涵蓋 fork／spawn 拒絕、執行緒允許、逾時、自行切換 session，以及在連線前拒絕外部 run 目錄。
macOS 四項與 Linux ARM64 實際 production 模組已通過；Linux x86_64 分支本輪僅做原碼覆核，待該平台實跑。
Linux syscall 規則依[核心 seccomp 文件](https://kernel.org/doc/html/latest/userspace-api/seccomp_filter.html)。
1.7 新增操作員 `send_message`（固定 sender=@operator、queue、必填 UUID v4）與 `driver_status`。
投遞沿用正式 driver 的去重／內容衝突檢查；一次性 client 不重送結果未知的請求。
`@operator` 使用 instance 名稱不允許的 `@`，與既有名為 operator 的 agent 及歷史訊息分開。
新操作員 RPC 不得採用同 ID 的舊 agent 訊息；衝突拒絕，不更名、刪除或改寫歷史收據。
Codex 狀態取自已連線 driver；未連線回 unknown，不把 fleet 的 unknown 當 idle。
其他 backend 的狀態與 outcome 分別核對；真測及認證來源見 [目前狀態](../gates/gate-13-status.md)。
原生 fake Codex 的三則 confirmed、逐次 idle、版本不符拒絕及暫存清理已通過；不是三家真模型認證。
新增操作員 `message_outcome` 唯讀查詢；Codex 必須在同一 thread 的唯一 turn 中找到相同 client ID 與原訊息內容，
並確認 completed、無 error，且 user item 之後有非空白 assistant 回覆。failed／interrupted 不因已 confirmed 或 idle 而通過。
canary 的三筆 outcome 必須各自對上收據的 message／instance／turn；查詢不重送或確認訊息，也不回傳訊息內容。
OpenCode 另核對 literal user、hashed message ID、assistant parentID、完成時間、finish=stop、無錯誤及非空白回覆；
唯讀取得最新 16 筆歷史，前後核對 instance、holder 與 endpoint，較舊或缺失證據回 unknown。
Claude 從同一資料庫快照核對 confirmed delivery、持久 ACK、單筆精確 receipt 的 native PostToolUse，
以及相同 prompt_id 的 Stop 與非空白最後回覆；任一 SessionStart／SessionEnd 橫跨該投遞即拒絕，包含延遲／replay 紀錄。
不讀 transcript 或共享設定。execution_id 保存 native prompt_id；delivery turn_id 仍維持 None，不偽造 backend turn。
報告的三筆 execution_id 不得重複。Claude 另以獨立 fake CLI 經真 daemon／channel／hook helper 跑三筆 ACK→PostToolUse→Stop，初始 Ready 畫面來自保存的真 CLI 錄製。這不代替新真模型、認證隔離或啟動選單驗收。Unknown 可暫無 execution_id；Completed 必須有 ID，未完成仍受同一 deadline 約束。
三則訊息預算不等於模型內部工具／token 的硬上限。OpenCode 1.18.33／1.18.34 已通過正式真模型 canary；Codex 0.159.3／0.160.0 亦已通過；Claude 依使用者指示暫緩。

## 下一步

返回 [版本管理](backend-versions.md) 查看正式切換／回退流程。
