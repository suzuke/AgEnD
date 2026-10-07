# 第 12D：Telegram

> **TL;DR**
> - 已實作通知完整分段、持久 outbox 與逐段收據；已接 daemon worker 與 inbound 操作防護，手機已讀／確認、多行回覆及真 forum 分流／重啟已驗收；已於 #156 合併（9dbfac7）。
> - 專用 bot 已做 getMe 與三則限額文字真測，三則均已刪除；token 設定檔未改動。
> - 下一步：12D 已完成並清理；繼續 12C 整合及第 12 關收尾。

## 設定與憑證

core `Config`／`TelegramConfig` 只保存 token 的環境變數名稱或絕對檔案路徑；不接受 inline token。daemon parser 不把 TOML 錯誤原文印出，以免 malformed 設定含密鑰。token 檔案須由目前 uid 擁有、禁止其他使用者存取，拒 symlink／非 regular file／過長檔案。

`allow_user_ids` 同時比對 chat ID、sender ID 與非 bot；空清單不允許 inbound 控制，後續 doctor 必須明確提示。needs-you topic 與 team topic 沿用 D13；私訊測試可不指定 topic。daemon 啟動讀取 `$AGEND_HOME/config.toml`，沒有檔案時不啟用 Telegram；設定或 token reference 錯誤會在 holder 啟動前拒絕 boot。

## 傳輸

固定 `https://api.telegram.org`，ureq 3.4.2 加 rustls 驗 TLS；不走 ambient proxy、不跟隨 redirect、不重試 mutation。每次請求限 35 秒，request／response 各限 1 MiB。HTTP／Telegram 錯誤只顯示安全代碼及 retry-after，不列 URL、token 或 server description。

Telegram `sendMessage` 長度有限，通知全文需完整分段保留，不能截斷；細節依 [Bot API](https://core.telegram.org/bots/api#sendmessage)。TLS 功能依 [ureq 官方文件](https://docs.rs/ureq/3.4.2/ureq/)。API 傳輸本身不認定已讀。

## 持久通知

`TelegramNotifier` 先將通知全文、bot／chat／topic 與全部分段寫入 SQLite，再逐段保存送出意圖；回覆的 bot、chat、topic、全文及 message id 都符合，才確認該段。送出前以 getMe 核對 immutable API token 的 bot 身分，錯誤身分不 claim／不送出。固定 delivery id 再次入列不重送已完成通知；未知結果保留 in-flight，重啟後不自動重送。這會犧牲未知段落的自動重試，避免失去回覆後重複通知。

真 Telegram 會刪除裸文字兩端空白，因此每段使用可見首尾標記保護原文，以 4000 UTF-16 units 分段、不拆 UTF-8 scalar；繁中、emoji、換行及尾端空白保留。第二、三則真測回傳全文逐字相同。outbox 目前保留全部紀錄；未知通知可由本機操作員明確 Abandon，保留期限尚待接入。

## Daemon 通知流程

worker 每秒觀察「需要你」，以持久 source id 對帳，不用 boot-local 事件游標或等待時間辨識通知。相同內容保留原 delivery；內容改變、或已觀察到解除後再出現，建立新 delivery。解除／替換時取消未完成舊通知的後續段落，未知段落仍保留意圖。snapshot 對帳與 outbox 建立同一 SQLite transaction，重啟不會把同一事項當新通知。

worker 將完整 recap、請示對話與可用動作送到 needs-you topic；HTTP 在 blocking pool 執行，不佔用主 engine。每次只送一段，下一段前重新對帳；停止時只等待當前有限期限呼叫與收據完成，再釋放 DB。HTTP 結果未知只記安全錯誤，不自動再送。最後一段附上由已保存選項產生的互動按鈕；team topic 另送任務摘要；未知通知的操作員處置見下節。

## 手機操作 checkpoint

`dc9ded8`：update 在執行前保存指紋與意圖；只有 allowlist 使用者回覆本 bot 已確認完整送達且仍有效的通知，才可進入 operator 路徑。按鈕帶通知 id 與選项編號，動作由 DB 保存的 snapshot 取回。每份通知只能 claim 一次實際操作，未知結果不重放；要求修改先提示回覆原因，收到文字後才操作。

Task 通知另保存 CAS version／attention revision；CLI 或 TUI 清除再開相同原因仍增加 revision，pipeline 排隊執行時重新比對。Instance failure episode 保存 reason 與單調時間，boot 沿用；Retry 由 supervisor 檢查並完成處理後回報，停機丟棄 queue 不會誤回成功。Inbound 與 outbound 各自執行，避免多段送出延後手機操作。

## 驗證與限制

十個 notifier 測試涵蓋設定／憑證、native HTTP 邊界、全部 NTF 契約、長 Unicode 全文、多段收據中斷與 DB 重開不重送、兩個 notifier 同 id 競爭只送一次。store 測試驗 immutable destination、逐段 CAS、schema migration／golden；worker 測試驗真 Fleet → SQLite → native HTTP → 收據，停止後等待時間改變不新建通知。native socket fixture 明確將 accepted socket 設為 blocking，避免 macOS 繼承非阻塞模式造成假失敗。

getMe 與 Message fixture 來自 2026-10-07 真 Telegram，僅替換識別資料。三則 sendMessage 各搭配一次 deleteMessage，三次刪除均確認；未重試 mutation。計畫及必要證據保留於 AgEnD-ops。

真 daemon serve 的 Retry／排隊停機組合已補。下方保留各 checkpoint 當時限制；多行回覆、forum 與整體原生驗收的最新結果見文末，合併仍待最終覆核。

## 下一步

真測僅使用使用者提供的專用 bot／chat，固定命令、預算與清理範圍；不修改共享帳戶或 Claude trust entries。驗收結果見文末，後續只處理覆核發現及合併收尾。

Schema v14 是新的 forward migration；已提交的 v13 保持原樣，舊 outbox 可直接升級。扣住第一段回覆的測試涵蓋停機、事項解除與替換，確認不開始舊通知第二段。

原生 inbound checkpoint：真 HTTP transport → SQLite → production pipeline 已驗未授權使用者拒絕、task-failed 確認、不同 update ID 重用舊通知拒絕與合法失效按鈕回饋。關機反例以丟棄 supervisor queue／待回覆事件驗重試不誤報成功，未將它當作實際 daemon 程序重啟證據。doctor 已加入本機設定／token reference 檢查，空 allowlist 報 fail 並明示 inbound 停用；不呼叫網路。

共用已讀 checkpoint：schema v16 保存 read key，protocol 1.6 以 fleet／事件同步兩個 TUI；Telegram 最後一段提供 Mark read。已讀不 claim 通知動作、不關閉事項，後續核准／確認仍可使用。真 daemon 的雙 TUI 與重啟測試通過；native HTTP 驗收與跨 crate 回歸另列檢查紀錄。沿用 T17 的 ID＋問題次數：非問答同 ID 重現不產生新的已讀識別；此批未改成 episode 語意。

Topic checkpoint：每個已設定 team topic 保存任務 ID、完整標題、status 與 current stage；只在內容改變時新建摘要，重啟保留原 delivery。摘要無操作按鈕。路由在送出前依當前設定重核，既有通知 destination 不搬移；改 topic 後舊未完成通知會拒送，內容改變才建立新的通知。辅助 Action result 在 enqueue 後、claim 前當機，worker 能依 bot／chat／topic 恢復；已 claim 未知結果與外來 destination 均不送。

## 手機 callback 驗收（2026-10-07）

專用私訊 bot 經正式 daemon／Telegram transport，先送一則 task-failed 通知，再停止／重啟 daemon。使用者點 Mark read 後，正式 Client 與 TUI App 狀態模型均觀察到 read key（此探針未渲染終端畫面），attention 仍開啟；再點 acknowledge，attention 關閉，停止後 SQLite 保存已讀與 failure_acknowledged=1，update outcome 依序為 read、accepted。未啟動模型。

必要證據保留在 `AgEnD-ops/g12d-telegram-20261007/mobile-live-v3/`。自有通知已 deleteMessage 確認刪除，兩次 daemon boot 正常退出，自有 home 已移除。這不代表真 forum topic、所有 ask／approval 動作、完整故障恢復或整體 12D 通過。

先前 v1 因驗證腳本在 daemon 持有 SQLite exclusive lock 時讀 DB 而失敗，已改為停機後讀取及明確關閉連線；v2 在 120 秒內未收到 callback 而失敗。兩輪失敗紀錄保留，兩則自有通知與 home 均已清理。v3 在使用者再次表示方便後，用 300 秒期限成功，未把前兩輪改記為通過。

本機零網路重驗：先 build `agend` binary 與 `telegram_mobile_probe` example，再執行 `python3 -B crates/agend/examples/support/telegram_mobile.py --target "$CARGO_TARGET_DIR" --out <新的證據目錄> --local`。真測改用 `--credentials <私人 env 檔>`，僅供專用私訊 bot／chat；一次通知，先已讀再確認，結束刪除自有通知。

原生問答補驗：正式 pipeline 建立 Ask 與 FollowUp，經本機 HTTP Bot API producer → poll → SQLite → guarded pipeline，分別選第二個選項與回覆完整多行自由文字；核 AnswerSource::Telegram 與提問者 inbox 每輪各一筆。每次 sendMessage 回不同 message ID，重複輪詢不再投遞，舊通知不能回答新追問。此例使用 inbox，未啟動模型，也不是 Telegram 真問答操作證據。

原生 human approval 補驗：無 repo 的 research 結果交付後，HTTP 通知按鈕經正式 pipeline 核准完成。要求修改須再回覆非空理由，提示按鈕與空白回覆不提前改 state；完整多行理由保存並退回 work。舊 callback 不重做動作。這是 bind_head=false 的原生路徑，不代替真 Telegram 核准或 Git head／merge 驗收。

## 未知通知的本機處置

送出取得 claim 後傳輸／收據失敗，保存 outcome_unknown；正在進行的正常請求不視為未知。daemon 取得 DB 後、啟動 worker 前，將上次留下的 in_flight 標為未知。即使停用 Telegram 或憑證已移除，本機 TUI／CLI 仍可看到 `telegram-delivery:<id>`，只能由操作員 Abandon；不能 Retry，不宣稱收到，也不補送後續段落。

Abandon 保存明確理由與原始 payload／收據前綴；保留 in_flight 和 outcome_unknown 作為未知證據。自動來源消失／換版不等於操作員處置。這類 attention 不再送回 Telegram，避免故障通知自我循環。

正常 Worker::stop 等候有限期限 HTTP 與收據，不 abort future。若直接取消公開 notifier future，或傳輸後保存 unknown 的 DB 寫入本身失敗，當次程序可能只有 in_flight、防重送但尚無 attention；重啟時恢復。此限制不被當作已確認送達。

原生三次開機驗證通過：無 Telegram 設定／token 仍發布未知通知，agent 被 Forbidden 拒絕、operator Abandon 持久化；最後一次開機持續觀察三秒未復活，DB 保留原 payload、空 receipt 與處置理由，不可重新 claim。core／daemon 共 399 passed、2 項既有 ignored；直接取消公開 notifier future 的限制仍依上段記錄。

Active shutdown checkpoint：兩個獨立子程序執行正式 daemon `serve`。本機 HTTP producer 扣住第一段回覆；SIGINT 移除 socket 後程序仍存活，收據放行後正常退出，SQLite 保存 receipt 500 且無未知意圖。第二次 boot 僅送原第二段，保存 receipt 501，全文與兩次請求逐段一致。測試不使用真 Telegram，也不涵蓋 pending Retry 取消；該項仍待補。

Retry lifecycle checkpoint：正式 daemon serve／poll／supervisor／holder 啟動本機 inbox 測試程序一次，停後 SQLite 為 Running、session_started 與 accepted；第二次 boot 重新連線而沒有第二次 launch 或 Accepted。排隊取消案例用 cfg(test) 消費屏障固定 Stop → RetryConfirmed 順序，正式 shutdown 丟棄 queue，保存 refused、原 Failed 保留、不產生測試程序。這不是 Telegram 真 Retry 或 Claude push／模型驗證。測試自有 holder 以 Shutdown 停止，確認鎖已釋放才刪 home。

驗收入口：`cargo xtask demo adapters` 已加入 Telegram notifier、daemon lifecycle、shared_read、telegram_unknown 與 doctor 原生案例；`cargo xtask accept 12` 包含 G4 跨 crate checks。兩者均不呼叫真 Telegram／模型，不能取代真 forum／自由文字回覆驗收。

自由文字探針準備：`telegram_reply_probe` 配合 `support/telegram_reply.py`，正式 daemon 保存多行答案與單筆 inbox，檢查來源、ask turn delivered、無存活 holder 後清理。`--local` 零網路重驗已通過；真測尚未執行，不能以 CLI source 代替 Telegram source。等待窗本機 30 秒／真測 300 秒；正常預期兩則 bot 訊息（提問、結果），並非 transport 硬上限，非預期 attention 即停止。收據刪除最多三則且逐筆核對本 bot／chat 與持久完整收據，未知結果保留 home，不自動重試。

本機命令：先 `cargo build -p agend --example telegram_reply_probe`，再 `python3 -B crates/agend/examples/support/telegram_reply.py --target "$CARGO_TARGET_DIR" --out <新的證據目錄> --local`；真測以 `--credentials <專用私人 env 檔>` 取代 `--local`，由使用者回覆通知中的兩行文字。


## 最終原生與真測證據（2026-10-07）

固定程式碼 `8c0538b` 的 `cargo xtask accept 12` exit 0，包含 startup capture 20／20、外層 PTY 8／8 與實際 no-std；四個 macOS／Ubuntu CI jobs 全數成功。全新 mailbox verifier 另完成 App 21／21、ready 3／3 與獨立 outer 8／8。先前 debug 389.290 ms 失敗與 release 診斷均保留，不能以這次通過宣稱所有負載下皆無長尾延遲。

使用者以 Telegram「回覆」送出完整兩行繁中、換行、é 與 emoji。正式 daemon 保存 `AnswerSource::Telegram`，ask turn delivered 一次，提問者 inbox 一筆且內容完全一致（`reply-live-v2`）。先前 `reply-live-v1` 只收到「Telegram 回覆」，exact-answer 失敗；兩輪均清理自有程序、home 與兩則 bot 通知，使用者訊息保留。

真 forum 測試使用兩個已由使用者建立的 topic。正式 daemon 將 Needs you 與 Team general 各送到指定 topic，Telegram 回覆的 bot／chat／topic／全文經正式 transport 驗證後保存收據；第二次 boot 的兩份 delivery 與 message IDs 完全不變，沒有重送（`forum-live-v1`）。兩次正常退出，兩則通知刪除確認，home 已移除，零模型程序。這輪停用 inbound，證明 topic 路由與重啟去重；手機操作證據是先前私訊的 callback 與自由文字回覆，不把它寫成 forum 按鈕驗收。

必要原始證據位於 `AgEnD-ops/g12d-telegram-20261007/`：`accept12-8c0538b-result.json`、`ci-8c0538b.json`、`reply-live-v2/result.json`、`forum-live-v1/`。forum 探針經全新只讀覆核，固定腳本與 binary 雜湊後才執行。未合併 worktree 與供收尾驗證的 debug target 暫留，合併後清理；Claude trust entries 未動。

2026-10-08：最終文件 head 16b7085 的四個 Ubuntu／macOS CI jobs 全部成功；#156 合併為 9dbfac7，自有 worktree、local／remote branch 與 target 已移除，必要證據保留。第 12D 交付完成；第 12C 整合與整關收尾繼續。
