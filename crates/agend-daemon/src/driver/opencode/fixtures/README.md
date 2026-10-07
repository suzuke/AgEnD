# OpenCode 1.18.34 訊息身分捕獲

> **TL;DR**
> - `1.18.34-no-reply-history.json` 是真 OpenCode REST 輸出，沒有改寫欄位。
> - 只驗指定 messageID、完整文字與 session 歸屬；沒有模型回覆，不替代完整模型 smoke。
> - 重驗 parser：`cargo test -p agend-daemon driver::opencode --lib`。

2026-10-07，以隔離 HOME／XDG、`serve --pure --hostname 127.0.0.1 --port 0`、隨機 Basic auth 密碼啟動 1.18.34；建立一個 session，送一次 `prompt_async`，指定 `noReply=true` 與 messageID，再讀 history／status。沒有複製帳戶資料，沒有 assistant message，session 維持 idle。自有程序與隔離目錄已清理。

固定 binary SHA-256：`7b63b34fafabded7d9231f6a9032755d0cdeaf8b9d2b70df8e25535471469eea`。
原始計畫、request、result、server log、cleanup 與一次性捕獲程式保留於本機 `/Users/suzuke/Documents/Hack/AgEnD-ops/g12b-api-20261007/`。捕獲程式拒絕覆寫既有證據；一般測試只讀此 fixture，不啟動真 backend。

此證據未驗模型收件、回覆遺失、duplicate messageID 的伺服器行為或權限流程。產品不得由此推定 POST 可以安全重送。

`1.18.34-model-history.json` 來自 2026-10-07 正式 daemon／holder 的單則模型 smoke，固定同一 1.18.34 binary，模型 `opencode-go/gpt-6-luna`。這是未改寫的原生 REST history，包含專用暫存 workspace 路徑、user message、terminal assistant 與 usage；沒有憑證。AgEnD message id 為 `9d65a4ec-04c2-492c-8f2b-5181513615c1`。計畫、回條、DB 事件及清理結果在上述 ops 目錄的 `model-smoke-v2`。v1 在送出前因 DB 獨占鎖失敗；v2 僅送一則，清理補上 `--yes` 後 port／程序／目錄皆消失，共享 auth 不變。此錄製僅證明基本單回合。
