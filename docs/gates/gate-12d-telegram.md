# 第 12D：Telegram

> **TL;DR**
> - 已開始設定與 HTTPS 傳輸實作；尚未接 notifier worker 或 inbound 操作。
> - 專用 bot 已做一次唯讀 getMe，未送出訊息；token 設定檔未改動。
> - 下一步：完整通知分段、持久 outbox／inbound 去重、allowlist、topic、手機操作與 G4 共用已讀。

## 設定與憑證

core `Config`／`TelegramConfig` 只保存 token 的環境變數名稱或絕對檔案路徑；不接受 inline token。daemon parser 不把 TOML 錯誤原文印出，以免 malformed 設定含密鑰。token 檔案須由目前 uid 擁有、禁止其他使用者存取，拒 symlink／非 regular file／過長檔案。

`allow_user_ids` 同時比對 chat ID、sender ID 與非 bot；空清單不允許 inbound 控制，後續 doctor 必須明確提示。needs-you topic 與 team topic 沿用 D13；私訊測試可不指定 topic。這些型別尚未接入 daemon 啟動。

## 傳輸

固定 `https://api.telegram.org`，ureq 3.4.2 加 rustls 驗 TLS；不走 ambient proxy、不跟隨 redirect、不重試 mutation。每次請求限 35 秒，request／response 各限 1 MiB。HTTP／Telegram 錯誤只顯示安全代碼及 retry-after，不列 URL、token 或 server description。

Telegram `sendMessage` 長度有限，通知全文需完整分段保留，不能截斷；細節依 [Bot API](https://core.telegram.org/bots/api#sendmessage)。TLS 功能依 [ureq 官方文件](https://docs.rs/ureq/3.4.2/ureq/)。通知投遞與重試狀態將由持久 worker 管理，API 傳輸本身不認定已讀。

## 驗證與限制

三個測試涵蓋 explicit allowlist／拒 inline token、private token file、native HTTP 捕獲回覆／拒 redirect／malformed response／安全錯誤。getMe fixture 來自 2026-10-07 真 Telegram 回覆，只替換 bot id／顯示名稱／username；不含 token。受控呼叫預算一個 getMe，已用一次，零訊息送出。

尚未完成 Notifier NTF 契約、通知 durable outbox、inbound 操作／去重、doctor、topics、TUI／Telegram 共用已讀、受控手機真測、全新覆核／CI／合併。這批不是第 12D 完成認證。

## 下一步

實作通知與持久投遞，再接手機操作。所有真測僅使用已提供的專用 bot／chat，先固定命令、預算與清理範圍；不修改共享帳戶或 Claude trust entries。
