# Backend 公開版本查詢

> **TL;DR**
> - `backend latest` 只讀 npm 公開 metadata。
> - 固定來源、套件身分、期限與大小上限。
> - 不取代 canary；受管 fleet 每日查詢並提供確認提醒。

## 公開最新版查詢

`agend backend latest claude --json`（亦接受 codex／opencode）讀取 npm 官方 registry 的對應 package `/latest` manifest，核 `name` 與受限版本字串。來源固定 https://registry.npmjs.org，TLS 驗證、拒轉址與 proxy 環境繼承，5 秒整體 HTTP deadline、256 KiB response 上限。不呼叫模型、不安裝、不改 fleet。

來源介面：[npm registry API](https://github.com/npm/registry/blob/main/docs/REGISTRY-API.md#getpackageversion)。latest 是 registry 的 tag，不保證大於目前使用版本，也不代表 AgEnD 支援；必須另做 import／canary／明確 switch。daemon 的受管 fleet 另以持久每日排程查詢；CLI 單次查詢不寫入 daemon 的每日紀錄。

## 離線設定

config.toml 的根層可設 `registry_checks = false`，停用 daemon 自動查詢；省略時啟用。`backend latest` 是操作員明確的單次查詢，不受此開關影響。native daemon 測試以此設定隔離外部網路；canary home 仍一律跳過。

## 驗證

三個 npm 原始 manifest 保存在 daemon 的 tests/fixtures/backend_registry，來源、時間及 SHA-256 一併記錄。測試透過本機 HTTP 重播，另變造套件名稱／版本及測試轉址、5xx、非 JSON、超量與逾時。

## 持久紀錄

migration 0021 的 backend_registry 最多保存三筆（每個 backend 一筆）。查詢前先保存 attempt 與開始時間；24 小時內不再預約，包含重啟與沒有完成回報的情況。時鐘倒退不提前觸發；舊 attempt 與重複完成均拒絕。失敗保留最近成功 metadata，另外保存 error，不能把歷史值當作本次成功。

結果內容或錯誤改變才增加 revision；同樣結果不重開提醒。acknowledge 必須匹配當前非零 revision；查詢本身不安裝、啟用或確認任何版本。daemon 每分鐘檢查受管 fleet，設定 program 必須仍符合 managed launch 才啟動每日查詢。canary home 不額外查最新版。查詢在 blocking worker，停機等待當前有期限的 HTTP 結束，不遺留脫離管理的查詢。ingest 恢復提醒，operator 只能 acknowledge；等待時間綁結果變動時間，確認與刷新序列化，避免舊快照重新發布已確認提醒。

## 下一步

補未受管 CLI 的被動漂移與整體原生服務驗收；查詢失敗不能推進為已知最新版，也不能影響正在工作的 backend。
