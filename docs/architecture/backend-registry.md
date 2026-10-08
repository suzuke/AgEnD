# Backend 公開版本查詢

> **TL;DR**
> - `backend latest` 只讀 npm 公開 metadata。
> - 固定來源、套件身分、期限與大小上限。
> - 不取代 canary；每日排程與通知尚待接入。

## 公開最新版查詢

`agend backend latest claude --json`（亦接受 codex／opencode）讀取 npm 官方 registry 的對應 package `/latest` manifest，核 `name` 與受限版本字串。來源固定 https://registry.npmjs.org，TLS 驗證、拒轉址與 proxy 環境繼承，5 秒整體 HTTP deadline、256 KiB response 上限。不呼叫模型、不安裝、不改 fleet。

來源介面：[npm registry API](https://github.com/npm/registry/blob/main/docs/REGISTRY-API.md#getpackageversion)。latest 是 registry 的 tag，不保證大於目前使用版本，也不代表 AgEnD 支援；必須另做 import／canary／明確 switch。此入口尚未接每日排程或「需要你」提醒。

## 驗證

三個 npm 原始 manifest 保存在 daemon 的 tests/fixtures/backend_registry，來源、時間及 SHA-256 一併記錄。測試透過本機 HTTP 重播，另變造套件名稱／版本及測試轉址、5xx、非 JSON、超量與逾時。

## 下一步

接持久觀測紀錄、每日查詢與 daemon attention；查詢失敗不能推進為已知最新版，也不能影響正在工作的 backend。
