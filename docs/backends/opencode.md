# opencode 1.18.31

> **TL;DR**
> - `opencode serve`：HTTP + SSE；server 原生排隊，abort 乾淨，沒有 steer。
> - 記住：**SSE 沒有 replay**，重連後用 REST 補狀態；授權以 `GET /permission` 輪詢為準。
> - 下一步：driver 在 `crates/agend-daemon/src/driver/opencode.rs`（第 12 關）。

來源：spike-opencode.md（O1–O6），2026-09-24，`opencode 1.18.31`。

## 結論表

| 項目 | 結果 | 說明 |
|---|---|---|
| O1 重連 | 可行 | turn 在沒有 SSE client 時照樣完成；新 `GET /event` 只收到 `server.connected`，不重播；用 `GET /session/:id`、`GET /session/:id/message`、`GET /session/status` 重建 |
| O2 TUI + server | 可行 | `/event` 是全域廣播：`opencode attach` 送出的 turn，外部 SSE client 也收到完整事件與 token delta |
| O3 排隊 | 可行 | 忙碌時再 `POST /session/:id/prompt_async` 回 204，server 依 FIFO 在前一個 turn 後執行 |
| O3 中斷 | 可行 | `POST /session/:id/abort` 回 200 `true`，立即 idle，可立即送新 prompt；被中斷的訊息帶 `MessageAbortedError` |
| O3 插入 | 不存在 | OpenAPI 全部路徑都沒有 steer／insert |
| O4 以 id resume | 可行 | 硬殺 serve 後以同 `XDG_DATA_HOME` 重起，同 session id 歷史與上下文都在 |
| O5 授權 | 可行，有例外 | `permission: {"bash": "ask"}` 會真的擋住；`GET /permission?directory=<dir>` 可查；`POST /session/:id/permissions/:permissionID {"response":"once"}` 回答 |
| O6 啟動提示 | 無 | 全新目錄與資料目錄直接進主畫面；沒有 trust 概念 |

## 陷阱

- [ ] `permission.asked` SSE 事件兩次試驗漏發一次，原因未查明 → 不能只靠 SSE。
- [ ] 專案層設定要用 `GET /config?directory=<project>` 查；`GET /global/config` 看不到。
- [ ] v1 送 `"model": "<字串>"`，但 OpenAPI 要 `model: {providerID, modelID}`（v1 缺陷，未進一步驗證）。
- [ ] `GET /session/status` 閒置時回 `{}`，只列出忙碌的 session。
- [ ] 公開 issue `anomalyco/opencode#46842` 回報某版本忙碌時 `prompt_async` 會卡住不排程（injection.md）；1.18.31 實測是正常排隊。升版時要重測。
- [ ] 隔離做法：`XDG_DATA_HOME`／`XDG_CONFIG_HOME` 指到專屬目錄並複製 `auth.json`，不需重新登入。

## 另一套 API

`/doc` 裡另有 V2 permission（`/api/session/:id/permission`、`permission.v2.asked`），未測。舊的那套已端到端驗證。

## 下一步

```bash
cat docs/V1-LESSONS.md
```
