# 訊息送達與狀態偵測

> **TL;DR**
> - 訊息內容一律走 backend 的結構化 API；PTY 只送單一控制鍵（如 `Esc`）。
> - 記住：**只有一套冪等（訊息 id）**；不能確認送達的路徑標為未確認，不假裝成功。
> - 下一步：各 backend 的實測對應看 [../BACKEND-BEHAVIORS.md](../BACKEND-BEHAVIORS.md)。

來源：規劃 r4 §4.3–§4.4、D16、injection.md。

## 送達模型

- 每則訊息有 id；狀態 `queued → sent → confirmed | failed`；以 id 冪等，只有一套去重。
- 推送一律帶完整內容。v1 截斷超過 200 字或只推標頭（`src/inbox/notify.rs:228-256`），是 inbox 被呼叫約 1.9 萬次的主因。
- 不模擬打字；不能確認送達就明確標示。

## 忙碌策略三級

daemon 依訊息緊急程度選一級，再對應到 backend 能力。

| 等級 | codex | claude | opencode |
|---|---|---|---|
| 排隊（turn 結束後送） | `thread/queue/add` | Stop hook `decision: block` | `prompt_async`（server 原生 FIFO） |
| 插入（不中斷） | `turn/steer` | 不支援 → 改用中斷 | 不支援 → 改用中斷 |
| 中斷後立即處理 | `turn/interrupt` 後送 | `Esc` 後立即經 channel 送 | `POST /session/:id/abort` 後送 |

「不支援 → 中斷」已寫在 `agend_core::policy::busy::effective_level`。

## claude 特別規則（D16）

- 互動式 TUI + hooks；狀態看 hooks。
- 排隊：Stop hook `decision: block`；先取出佇列再回傳；`stop_hook_active` 防迴圈。
- 閒置時經 channel 送。
- 中斷 = `Esc` 後立即經 channel 送；`Esc` 不會觸發 Stop，不做等待。
- 專案 CLAUDE.md 必須說明 agend channel 訊息來自使用者自己的團隊（無說明 0/3、有說明 3/3、opus 2/2）。
- 訊息內可另加 `from`／`task`／`request` 標頭。

## 狀態偵測（三層）

1. 結構化事件判斷 busy／idle／exited。
2. 螢幕分類器直接讀 holder 的畫面，只認 hard gate：usage limit、permission／approval、rate limit、auth error、context full、啟動與更新選單。hard gate 不被結構化事件覆蓋；每條規則附真實畫面 fixture。
3. 去抖動：idle↔active 穩定 N 秒才生效（v1 約兩天 75 萬次轉換）。

- TUI 顯示的 agent 狀態：working、idle、需要你、stuck、unknown。
- claude hook 事件在 daemon 不在時寫磁碟佇列，回來後補送，再以螢幕分類器確認一次（v1 直接丟棄：`src/main.rs:1243-1245`）。

## 啟動與授權提示（四層，越上面越優先）

1. 避免出現：選不會跳提示的啟動方式；啟動前預寫 trust 狀態（codex 已驗證）；盡量不用需要逐次確認的實驗旗標。
2. 結構化處理：codex app-server approval 請求、opencode permission、claude permission hook，由 daemon 政策決定或轉問人。
3. 已知提示規則是資料：每個 backend 一份規則檔（比對樣式 → 單一按鍵），附真實畫面 fixture。
4. 未知提示兜底：啟動後 N 秒沒進 ready 且畫面不動 → 標「卡在未知提示」→ 快照送 TUI／Telegram 請人處理；人的回答以單一按鍵送出，畫面存成新規則的候選 fixture。

另外：偵測到 backend 版本變了，先在暫存 workspace 起 canary instance 確認能進 ready，再讓整個 fleet 重啟；不行就提前通知。

## 細節

v1 的證據（injection.md）：

- 改走結構化 API 後「貼上沒送出」類問題幾乎消失（26 次修復中 23 次在 2026-04）。
- 剩下的不穩定：忙碌處理各自為政、三套去重並存、狀態誤判導致時機錯誤。
- v1 啟動提示靠刮螢幕：`src/agent/dismiss.rs`（44 KB）+ `dev_modal.rs`（59 KB），27 個 commit，2026-09-14 仍在修。

## 下一步

```bash
cat docs/BACKEND-BEHAVIORS.md
```
