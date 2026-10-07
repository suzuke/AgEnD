# 第 12D：Telegram

> **TL;DR**
> - 已實作通知完整分段、持久 outbox 與逐段收據；已接 daemon worker 與 inbound 操作防護，完整端到端與手機驗收尚未完成。
> - 專用 bot 已做 getMe 與三則限額文字真測，三則均已刪除；token 設定檔未改動。
> - 下一步：原生 inbound 端到端、受控手機操作、topic 路由與 G4 共用已讀。

## 設定與憑證

core `Config`／`TelegramConfig` 只保存 token 的環境變數名稱或絕對檔案路徑；不接受 inline token。daemon parser 不把 TOML 錯誤原文印出，以免 malformed 設定含密鑰。token 檔案須由目前 uid 擁有、禁止其他使用者存取，拒 symlink／非 regular file／過長檔案。

`allow_user_ids` 同時比對 chat ID、sender ID 與非 bot；空清單不允許 inbound 控制，後續 doctor 必須明確提示。needs-you topic 與 team topic 沿用 D13；私訊測試可不指定 topic。daemon 啟動讀取 `$AGEND_HOME/config.toml`，沒有檔案時不啟用 Telegram；設定或 token reference 錯誤會在 holder 啟動前拒絕 boot。

## 傳輸

固定 `https://api.telegram.org`，ureq 3.4.2 加 rustls 驗 TLS；不走 ambient proxy、不跟隨 redirect、不重試 mutation。每次請求限 35 秒，request／response 各限 1 MiB。HTTP／Telegram 錯誤只顯示安全代碼及 retry-after，不列 URL、token 或 server description。

Telegram `sendMessage` 長度有限，通知全文需完整分段保留，不能截斷；細節依 [Bot API](https://core.telegram.org/bots/api#sendmessage)。TLS 功能依 [ureq 官方文件](https://docs.rs/ureq/3.4.2/ureq/)。API 傳輸本身不認定已讀。

## 持久通知

`TelegramNotifier` 先將通知全文、bot／chat／topic 與全部分段寫入 SQLite，再逐段保存送出意圖；回覆的 bot、chat、topic、全文及 message id 都符合，才確認該段。送出前以 getMe 核對 immutable API token 的 bot 身分，錯誤身分不 claim／不送出。固定 delivery id 再次入列不重送已完成通知；未知結果保留 in-flight，重啟後不自動重送。這會犧牲未知段落的自動重試，避免失去回覆後重複通知。

真 Telegram 會刪除裸文字兩端空白，因此每段使用可見首尾標記保護原文，以 4000 UTF-16 units 分段、不拆 UTF-8 scalar；繁中、emoji、換行及尾端空白保留。第二、三則真測回傳全文逐字相同。outbox 目前保留全部紀錄，操作員處置與保留期限尚待接入。

## Daemon 通知流程

worker 每秒觀察「需要你」，以持久 source id 對帳，不用 boot-local 事件游標或等待時間辨識通知。相同內容保留原 delivery；內容改變、或已觀察到解除後再出現，建立新 delivery。解除／替換時取消未完成舊通知的後續段落，未知段落仍保留意圖。snapshot 對帳與 outbox 建立同一 SQLite transaction，重啟不會把同一事項當新通知。

worker 將完整 recap、請示對話與可用動作送到 needs-you topic；HTTP 在 blocking pool 執行，不佔用主 engine。每次只送一段，下一段前重新對帳；停止時只等待當前有限期限呼叫與收據完成，再釋放 DB。HTTP 結果未知只記安全錯誤，不自動再送。最後一段附上由已保存選項產生的互動按鈕；team topic 路由與未知通知的操作員處置仍待完成。

## 手機操作 checkpoint

`dc9ded8`：update 在執行前保存指紋與意圖；只有 allowlist 使用者回覆本 bot 已確認完整送達且仍有效的通知，才可進入 operator 路徑。按鈕帶通知 id 與選项編號，動作由 DB 保存的 snapshot 取回。每份通知只能 claim 一次實際操作，未知結果不重放；要求修改先提示回覆原因，收到文字後才操作。

Task 通知另保存 CAS version／attention revision；CLI 或 TUI 清除再開相同原因仍增加 revision，pipeline 排隊執行時重新比對。Instance failure episode 保存 reason 與單調時間，boot 沿用；Retry 由 supervisor 檢查並完成處理後回報，停機丟棄 queue 不會誤回成功。Inbound 與 outbound 各自執行，避免多段送出延後手機操作。

## 驗證與限制

十個 notifier 測試涵蓋設定／憑證、native HTTP 邊界、全部 NTF 契約、長 Unicode 全文、多段收據中斷與 DB 重開不重送、兩個 notifier 同 id 競爭只送一次。store 測試驗 immutable destination、逐段 CAS、schema migration／golden；worker 測試驗真 Fleet → SQLite → native HTTP → 收據，停止後等待時間改變不新建通知。native socket fixture 明確將 accepted socket 設為 blocking，避免 macOS 繼承非阻塞模式造成假失敗。

getMe 與 Message fixture 來自 2026-10-07 真 Telegram，僅替換識別資料。三則 sendMessage 各搭配一次 deleteMessage，三次刪除均確認；未重試 mutation。計畫及必要證據保留於 AgEnD-ops。

尚未完成原生 inbound 端到端驗證、doctor、topics 實際路由、TUI／Telegram 共用已讀、受控手機操作真測、整體全新覆核／CI／合併。這批不是第 12D 完成認證。

## 下一步

先驗原生 inbound 的操作與關機邊界，再做受控手機 callback 真測。所有真測僅使用已提供的專用 bot／chat，先固定命令、預算與清理範圍；不修改共享帳戶或 Claude trust entries。

Schema v14 是新的 forward migration；已提交的 v13 保持原樣，舊 outbox 可直接升級。扣住第一段回覆的測試涵蓋停機、事項解除與替換，確認不開始舊通知第二段。
