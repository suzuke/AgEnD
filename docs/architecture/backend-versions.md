# Backend 版本管理（13C 施工中）

> **TL;DR**
> - `backend import` 保存獨立的原生 executable；`backend inspect` 核對保存的內容。
> - 匯入不執行程式、不驗證宣告的版本、不切換 fleet，也不代表 canary 通過。
> - 原生匯入、canary、新啟動准入與持久身分重連已接入；套件目錄、漂移偵測、升級與回退仍在施工。

## 目前可用

```bash
agend backend import claude --version 2.1.284 --program /absolute/path/to/claude
agend backend inspect claude --version 2.1.284 --json
```

backend 可選 claude、codex、opencode；版本名稱限 1–80 個 ASCII 英數或 `.-_+`，首字必須英數。
只供操作員使用，帶 `AGEND_INSTANCE` 的呼叫拒絕。

副本位於 `$AGEND_HOME/backends/<backend>/<version>/program`，權限 0500；
最後才發布 0600 的 `import.json`，記錄 backend、宣告版本、大小與 SHA-256。
版本目錄 0700，已有目錄一律保留並拒絕覆寫。
來源不得大於 1 GiB，必須有 ELF／Mach-O 原生格式標頭及執行權限；標頭不保證可執行或可搬移。
匯入核對複製前後來源 identity、大小、mtime 與內容 digest，來源變動則拒絕發布。
inspect 重新核對 manifest identity、檔案大小、權限與 digest；缺 manifest 的中斷匯入拒絕採用。

shell／npm wrapper 目前拒絕，避免只複製入口卻漏掉相依套件。
JSON 的 `canary` 回報 `not_run`、`passed` 或 `invalid`；`invalid` 附 `canary_reason`。
inspect 重新核對報告與目前 artifact、AgEnD binary、平台及版本的綁定，並要求清理成功、三筆唯一 confirmed 收據與合法時間順序。
超過 64 KiB、缺收據、時間越界或 overflow 均拒絕；`active` 目前一律為 false。
報告只是當下內容檢查，不防同 UID 偽造；長期運作的 daemon 必須保留啟動時的 binary digest。
runtime 已保存啟動 fingerprint 與檔案身分；macOS 核實際 executable mapping，Linux 核 `/proc/self/exe`，拒絕磁碟路徑已換成另一映像的情況。後續檢查不重新採納替換檔的 digest。
正常 daemon 啟動會在 `runtime-binaries/<sha256>/agend` 建立私有、不覆寫的副本，holder、hook、shim 與重啟預設路徑共用它；副本保留供存活的 holder 使用，不能在 daemon 停止時直接刪除。目錄 0700、程式 0500；這是正常升級隔離，不防惡意同 UID 修改私有檔案。
現有 instance 的 program 不受這兩個命令更動。

新 holder 啟動前，daemon 核對指向受管目錄的 program（包含指向該檔案的 symlink alias、依 agent PATH 選定的裸名稱與依 instance cwd 解析的相對路徑）。
內容有變或尚無 canary 的版本拒絕啟動，保存 failed 原因並顯示需要你；不先建立 holder。
核對由 daemon 與 CLI inspect 共用，hash 工作移到 blocking pool。
有有效 canary 的受管程式可由 instance 的明確 `--program` 啟動；已有準備／查詢／取消入口；啟用／回退已接 supervisor，Codex 原生假 backend 的完整往返已通過；三後端與故障恢復驗收尚未完成。
新啟動使用 canonical 匯入路徑，先持久化 artifact／設定／啟動 UUID，再送 SpawnBound。
重連核對原 artifact bytes、設定與 holder UUID；缺少意圖或不符時保留 holder 並標記失敗，不自動替換。
Canary 證明目前 daemon 與匯入 artifact 的新啟動、Ready、三輪成功回應；
既有 holder 重連另循 D3，不因新 daemon 的 canary 缺失或不匹配而重新執行 backend。
shared shim 隨 daemon 更新循 D5，不要求所有程序使用同一 AgEnD build。
裸名稱依目前使用者的 `access(X_OK)` 選擇 PATH 中可執行檔。
未以 `./` 或 `../` 開頭的相對 slash 路徑（如 `foo/bar`）在 shell／PTY 有歧義，要求改用絕對路徑或 `./`。
其他非受管 program 維持既有行為；這不是任意 shell／wrapper 的執行沙箱，也尚未偵測系統 CLI 的更新。

## 與資料刪除互斥

匯入取得 home 共享維護鎖，整段發布完成才釋放；可與運作中的 daemon 共存。
解除安裝刪資料需要排他維護鎖，因此不能與發布交錯。
遇到維護中的 home 立即拒絕，操作員可等完成後重試；不自動重送。
已發布檔案可被同一使用者修改，digest 檢查會拒絕不一致；這不是抵抗同 UID 惡意程式的隔離機制。

## Agent 程序內的更新設定

- Claude：`DISABLE_AUTOUPDATER=1` 與 `DISABLE_UPDATES=1`。
- OpenCode：`OPENCODE_DISABLE_AUTOUPDATE=1`；既有 launcher 也設定 `autoupdate: false`。
- Codex：既有 launcher 逐次傳入 `check_for_update_on_startup=false`。

這些設定只套用在 AgEnD 啟動的 agent，不修改操作員共用 CLI、設定或 Claude trust entries。
環境設定不能防止共用 executable 被其他程序更新，因此仍需隔離版本及啟動前核對。
Claude 設定語義見[官方環境變數](https://code.claude.com/docs/en/env-vars)，
OpenCode 見[官方 CLI 環境變數](https://opencode.ai/docs/cli/)。

## Canary 收據查詢（1.7）

操作員 `message_delivery` RPC 讀取 daemon 保存的 queued／sent／confirmed／failed，
保留訊息 ID、sender／target、turn ID 與 attempted／updated 時間；不存在回 None。
agent 禁止使用，回覆不含訊息 body。查詢不會推進任何狀態。
client 使用一次性、有共同期限的 exchange；舊 daemon 拒絕此能力，未知狀態不算 confirmed。
這個 API 供 canary 對帳。`backend canary --allow-model` 執行器正在施工，尚未通過完整驗證，不可用於版本准入。
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
其他 backend 目前使用 daemon fleet 狀態，真 backend 的就緒證據與隔離登入資料仍待驗證。
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
三則訊息預算不等於模型內部工具／token 的硬上限；目前未執行真模型。

## 驗證與下一步

`cargo test -p agend --test backend_import` 使用真 CLI 與原生 `/usr/bin/true` 檔案作 producer，
不執行 backend；涵蓋內容與來源保存、重複匯入、竄改、路徑轉向、非法版本、wrapper、agent 拒絕及維護互斥。
`cargo test -p agend-daemon --lib store::maintenance` 驗共享發布、store 與排他移除的生命週期。

下一步接套件匯入、版本探測與漂移提醒、明確切換／回退，之後才驗整個 fleet 的版本管理。

啟動綁定目前已完成 holder 1.3 producer：`SpawnBound` 成功時保存 opaque UUID，`GetLaunchBinding` 在重連後回原 UUID／instance／pid；沒有綁定的 legacy 程序回 None，不能被新請求補認。daemon 已接入持久 intent 與匯入 artifact 對帳；受管路徑須通過當前 AgEnD 指紋的 canary 才可新啟動。

受管啟動意圖由 SQLite migration 0018 保存，每 instance 一筆，與 instance 移除 cascade。儲存時核對 instance 快照並以舊 binding CAS；不明結果先讀回，不能盲目重試。新 holder 啟動前才可替換意圖，重連不得建立新意圖；supervisor 必須先證明舊 holder 已離開。supervisor 在確認舊 holder 不存在、orphan 已清理後保存意圖；解析受管別名後以 canonical 匯入路徑建立實際 argv，保留原始設定供重連核對。

Runtime 保留該次意圖的 UUID，首次啟動核對 Spawned 與 GetLaunchBinding 的 agent PID 一致；重連只讀回 UUID／PID，核對前不發布 writer。未驗證 Exited 不觸發生命週期處理；取消中的核對不發失敗通知。supervisor 收到當前 generation 的 binding rejection 時標記失敗並 detach，保留 holder；重連核對持久 artifact 與 instance 設定，再讀 holder UUID；缺失或不符直接標記失敗，保留 holder，不排自動替換。三種原生替身已驗新啟動、保留原 holder 重連、錯 UUID 拒絕且不替換程序。明確升級／回退正在原生驗證，真模型認證隔離尚未完成。

重連沿用既有啟動准入證據，只核對受管 bytes／設定與 holder 原 UUID；不因 daemon 升級要求重新跑 canary。新 Spawn 仍須當前 daemon 指紋的 canary。intent 的 session 是啟動時指定值：首次 Codex／OpenCode 的 None 可由正式 driver 後續建立 session；原本 Some 或 Claude 仍精確核對。

## 切換記錄（儲存層已實作，操作流程待接）

Migration 0019 保存每 instance 最近一次 BackendSwitch；prepared 只保存原啟動意圖與目標，不改 program。commit／rollback 在同一 SQLite transaction 核對完整 switch 記錄、原設定與 agent_pid 已清除，再一起改 program 與 phase。過期請求不能覆寫目前狀態；不明結果先讀回，不盲目重送。新 prepare 以先前 switch ID 做 CAS，不能覆蓋 Prepared／Committed／RollbackPrepared／Restoring 的進行中切換；明確移除 instance 時 cascade。此層不取代 supervisor 的 canary、idle／派工暫停、精確 holder 停止及重啟驗證，準備／查詢／取消 CLI 已接入，完整升級／回退流程仍未完成。

## 準備、查詢與取消（client 1.8）

```bash
agend backend switch prepare <instance> --version <version>
agend backend switch status <instance> --json
agend backend switch cancel <instance> --switch-id <id>
agend backend switch activate <instance> --switch-id <id>
agend backend switch rollback <instance> --switch-id <id>
```

由 daemon supervisor 序列處理。prepare 要求來源為 running 受管 instance、來源 artifact／設定與持久意圖一致，以及目標通過目前 daemon 的 canary 檢查；Prepared 暫停新 push reservation 與 agent inbox 讀取，不停止舊 holder，也不啟用目標。
已有完成／取消紀錄時，下一次 prepare 必須帶 `--previous <id>`，避免覆寫其他操作者的新請求。cancel 只接受精確 Prepared ID 且 instance 設定仍吻合；保留 program；holder 仍在時重連投遞，已消失的 running instance 重新啟動原版本。RPC 失去回覆時先查 status，不自動重送。

Codex 原生假 backend 整合測試已經兩個版本的正式 canary、prepare、activate 到 Activated、rollback 到 RolledBack，核 holder 更換與 session 保留。這不是 Claude／OpenCode、真模型或所有中斷恢復情境的證據；Prepared 仍不是完成換版。

換版停止原語 `HolderRuntime::stop_reserved` 要求持久意圖及精確 holder／agent PID。查 LaunchBinding 與 Shutdown 使用同一連線，後續 socket 路徑替換不會把停止送到新 peer；回覆不符或 holder PID 改變時保留程序，結果不明須對帳。呼叫前的回合結束證據、在途排空及新啟動序列化仍由 supervisor 負責，尚未完成。

Prepare 在 SQLite 暫停新 reservation 後，捕捉已開始的 Claude／inbox server 回覆，最多等待 10 秒直到既有 handler／socket write 全部完成或連線任務結束。其後的空輪詢不加入舊範圍；逾時仍保存 Prepared，呼叫者查 status 對帳。ACK、backend 完成與 worker 停止是後續獨立條件；socket 已寫完不代表模型已消費內容。


切換持久狀態分成 Prepared（尚未改路徑）、Committed（已選新版、待啟動驗證）、Activated（新版已驗）、Restoring（已恢復舊路徑、待重新啟動驗證）、RolledBack（舊版已重新驗證）與 Cancelled。四種進行中狀態 Prepared／Committed／RollbackPrepared／Restoring 都暫停新投遞，重開 DB 不會解除。`finish_backend_switch` 以精確切換紀錄、Running instance／PID／session 及新 launch UUID 做 CAS，檢查 artifact 與設定後才釋放；不能以原本已停止的 launch 意圖宣告回滾成功。呼叫者仍須先驗真正 native readiness，Store 的快照驗證本身不是程序存活證據；supervisor 定期核 native 閒置與 holder 綁定後才完成狀態。Activated 的回滾先保存 RollbackPrepared 暫停投遞，再停止目前 holder；Committed／Restoring 無 holder 的 boot 只對設定吻合且仍准入的目標啟動。失敗維持 pending，需查 status；自動失敗回滾與全部重啟切點仍待驗證。

進行中的切換也拒絕操作員完整終端 acquire／resize／input 與 legacy input，避免閒置核對期間再開始工作；唯讀與 release 可用。TerminalHub actor 在查 DB 前加入排空追蹤，完整控制請求直到返回才釋放；legacy blocking writer 自己持有追蹤，actor 取消不提前釋放。完整控制逾時／斷線與 legacy 寫出不證明 backend 已停止工作，仍須獨立 native 回合完成與身分核對。

目前代目的 holder 確認消失、且持久 launch 精確對應 Committed 目標時，自動回退先保存 RollbackPrepared，再恢復舊版本；不接受舊代退出事件作為依據。RollbackPrepared 在 daemon server 排空追蹤器就緒後續行，涵蓋回退意圖保存後重啟。三 backend 原生假版本的 holder 消失回退已驗；Codex 重啟案例用正式 Store API 建立精確持久切點。仍存活但未就緒的 backend、Ready 已觀察後的中斷、Restoring 再次失敗及完整通知政策仍待完成。

經身分驗證的目前代 `AgentExited` 也可觸發目的版本回退：先保存 RollbackPrepared、排空既有回覆及 worker，再以持久 UUID／holder PID／agent PID 停止精確 holder，恢復舊版本。driver Gone 或 StartFailed 不等於原生退出，不能走這條捷徑。Codex fixture 在 app-server 交接完成後讓 TUI 自行退出，已驗恢復舊版及原 session；app-server 單獨退出而包裝仍存活的情況尚待處理。

換版的 `problem` 保存具體失敗原因與首次等待時間；driver 斷線、啟動失敗或自動回退被拒時，在「需要你」顯示 `backend-switch:<instance>:<switch-id>`，並由 `backend switch status` 顯示原因。daemon 重啟恢復同一通知，pipeline 同步不移除它。通知沒有一般 Retry 動作；操作員先查狀態，仍循正式換版／回退入口。成功啟用、回退、取消或移除 instance 才清除通知。這不代表斷線本身授權停止存活 holder；啟動逾時政策仍待完成。
