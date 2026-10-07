# OpenCode 1.18.34 訊息身分捕獲

> **TL;DR**
> - `1.18.34-no-reply-history.json` 是真 OpenCode REST 輸出，沒有改寫欄位。
> - 只驗指定 messageID、完整文字與 session 歸屬；沒有模型回覆，不替代完整模型 smoke。
> - 重驗 parser：`cargo test -p agend-daemon driver::opencode --lib`。

2026-10-07，以隔離 HOME／XDG、`serve --pure --hostname 127.0.0.1 --port 0`、隨機 Basic auth 密碼啟動 1.18.34；建立一個 session，送一次 `prompt_async`，指定 `noReply=true` 與 messageID，再讀 history／status。沒有複製帳戶資料，沒有 assistant message，session 維持 idle。自有程序與隔離目錄已清理。

固定 binary SHA-256：`7b63b34fafabded7d9231f6a9032755d0cdeaf8b9d2b70df8e25535471469eea`。
原始計畫、request、result、server log、cleanup 與一次性捕獲程式保留於本機 `/Users/suzuke/Documents/Hack/AgEnD-ops/g12b-api-20261007/`。捕獲程式拒絕覆寫既有證據；一般測試只讀此 fixture，不啟動真 backend。

此證據未驗模型收件、回覆遺失、duplicate messageID 的伺服器行為或權限流程。產品不得由此推定 POST 可以安全重送。
