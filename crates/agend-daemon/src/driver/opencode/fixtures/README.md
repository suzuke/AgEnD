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

`1.18.34-busy-history.json` 來自相同固定 binary／模型的 busy smoke v2：三筆 user 皆 Confirmed，第一輪 aborted，最後 interrupt 回覆完成；原生紀錄只有兩筆 terminal assistant，不能把 queued user 宣稱為另一個已完成回合。`busy-smoke-v1` 在第一筆 POST 後尚未 busy 就過早斷言而停止；v2 明確等 busy 後送第二、三筆，無重送。計畫／checkpoints／結果／清理在 ops `busy-smoke-v2`。

`1.18.34-pages.json` 是零模型、無帳戶的四筆 noReply 捕獲，兩頁各兩筆；保存原生 session／opaque cursor／single message。計畫與清理在 ops `pagination-capture-v2`；測試僅重播捕獲，不重新執行 CLI。

`1.18.34-permission-{once,reject}-{pending,history}.json` 來自相同固定 binary／模型的兩則權限真測；沒有 SSE subscriber，REST 取得原始請求、核准一次的 printf 完成、拒絕一次的工具為 error。只證明後端 API 與工具結果，不宣稱 daemon attention／重啟端到端真測。ops `permission-capture-v1` 保存固定計畫與清理；共享 auth 雜湊未變，自有 root、程序及 port 均清除。
