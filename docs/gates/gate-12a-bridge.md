# 第 12A：protocol 1.5、channel／Stop 與 ACK spool

> **TL;DR**
> - #148 store 已合併；本批接真 daemon／holder／helper，提供完整內容投遞與明確收件回報。
> - 先存投遞意圖再寫出；已開始但結果不明不重送；離線只補事件／ACK。
> - 下一步：#149 已合併並完成使用者重驗；接 gh 防護，再接完整 Claude Driver。

## 狀態與範圍

2026-10-04，[#149](https://github.com/suzuke/AgEnD/pull/149) 已經使用者確認合併為 `6dd552e`；head `c7e398c` 全新 verifier r2 CONFIRMED，push／PR 雙平台 CI 通過，merge tree 與驗證 head 相同。使用者另重驗 16 native cases 全過，自己的暫存 target／驗證 worktree 已清理；原 feature worktree／branch 亦已刪除。原 head `b4c6b46` 的 r1 REFUTED 與 race 證據仍保留；本批不是完整第 12A 驗收。真 Claude、模型回合與錄製均未執行。

| 本批已有 | 尚未完成 |
|---|---|
| client 1.5、同步 one-shot helpers | 完整 Claude Driver trait、pipeline／fleet busy 事件整合 |
| MCP stdio channel、Stop block／防迴圈、agend_ack | 自動產生 settings／.mcp.json／來源說明及檔案 ownership hash |
| 真 SQLite 預約／Written／ACK、原子 spool 與 daemon 自動 ingest | Interrupt／Steer 的 holder Esc 與人工控制權；目前 Stop 只取 Queue |
| live hook／五秒 idle／目前 holder 畫面分類器 | 新版本啟動提示 fixture／自動接受、[gh shim 待驗證](gate-12a-gh-shim.md)、Claude orphan sweep |
| native 自動驗證及四次 daemon 開機 | 新版本 FakeClaude 主動 ACK、完整 DRV-6／9、真 CLI 版本一致性／smoke |

helper 可直接供已配置的 Claude push instance 使用，但本批不會自動把 Claude 設定接上。一般 CLI 及 Claude inbox fixture 的既有路徑不變。

## Client protocol 1.5

hello 提供 1.5／1.4／1.3；一般 client 仍需 1.3、完整終端需 1.4。新操作只經 `exchange_once(..., V1_5, ..., deadline)`，generic Safe／Never 均拒絕 Claude RPC；選到 1.4 以下的連線回 `not_supported`。

request 共用 `type=claude`，data 是 `ClaudeRequestData { request_id, instance_id, operation }`。operation 以 `operation` 欄位區分：

| 操作 | 欄位與效果 |
|---|---|
| attach | 無其他欄位；取得目前 session，不宣告 idle；channel 只固定一次，不隨 instance 替換改 session |
| poll | session_id；只取同 session 的穩定 idle 訊息，先 commit 新預約才回完整內容 |
| hook | event_id、session_id、event、payload（native JSON object 的字串）、occurred_at_unix_ms、replayed；先保存事件 |
| written | receipts（1–32）；核原 instance／投遞 tuple，記成功寫出 Sent，沒有確認收件 |
| ack | receipts（1–32）；核原 instance／message／delivery／session，原子保存單筆 Confirmed；重複冪等 |

`ClaudeReceipt` 是 `{ message_id, delivery_id, session_id }`；instance 由 socket caller 與 data.instance_id 比對。核的是投遞保存的身分，移除／替換 instance 後的舊有效 ACK 仍接受。ACK／Written **每一 receipt 各一交易，整批不是單一原子交易**；後方 receipt 拒絕時，前方已提交者不回滾，重試仍冪等。

reply data 是 `ClaudeReplyData { request_id, session_id, messages, committed }`；messages 每筆 `{ receipt, content }`，content 是既有 `From:`／選填 `Task:`＋完整 body。committed 只表示該操作已提交；不把 hook 入庫當成訊息收件。

Unix socket／owned home 沿用同 UID、0600／0700 的信任邊界，caller 比對不是密碼認證；不防刻意以同 UID 偽造 caller／spool。沒有 token 或跨使用者通訊。

## 路由與寫出

- SessionStart 建立初始 idle；UserPromptSubmit／SessionEnd 即時撤銷 idle；Pre／PostToolUse 只保存事件。
- Stop 只有明確 `stop_hook_active=false` 可取 Queue；有內容回 `decision=block`／完整 reason 並保持 busy；active 或缺欄位不續行、不 ACK。
- idle channel 至少穩定五秒。daemon 必須取得目前 live holder 畫面，既有 classifier 命中提示就暫停內容投遞；重啟後不以歷史事件建立 idle。
- duplicate／replayed／來源時間與入庫時間相差超過五秒的 hook 不取 queue、不建立 idle；新補入且較新的同 session 路由事件會撤銷舊 idle。延遲的 live 路由事件也不能覆蓋較新 source time 的 busy／idle；重啟要有新的 live hook 才能恢復 idle。holder 畫面查詢不持有路由鎖；查詢後必須核 session 與 revision 未變，才預約內容。
- 單次回覆最多 32 則、約 1 MiB（含標頭餘裕）；合法 1 MiB 單則可整份送出，沒有截斷。
- reservation commit 先於 reply／stdout；只有新 Started 可寫出。stdout 完整寫完後另送 Written；若 reply／stdout／Written 遺失，保留原 intent 或 Sent，等待 ACK／人處理，不自動重送。

## Helpers 與 spool

`agend channel --instance <id>` 需要 `AGEND_INSTANCE=<id>` 及 AGEND_HOME；使用 MCP JSON-RPC stdio，stdout 只放協定，診斷在 stderr。initialize 宣告 experimental `claude/channel` 與 tools，initialized 後開始操作。通知 `notifications/claude/channel` 帶完整 content 與 receipt meta。

`agend_ack` 收 receipts，先保存接受的整批，再嘗試 daemon。離線回「已保存、待同步」；拒絕 tuple 或本機保存失敗回工具錯誤；只有 daemon 入庫才宣稱 confirmed。模型可能漏呼叫 ACK，這時仍未確認；ACK 不代表 task 完成。

`agend hook <event>` 支援 SessionStart／UserPromptSubmit／Stop／PreToolUse／PostToolUse／SessionEnd；stdin 是 testkit／Claude native hook JSON，讀到 EOF，整體期限十秒。在同一次 flock 持有期間先落磁碟，再嘗試首次 live RPC，避免 ingest 搶先把新事件當歷史；離線 Stop 回 `{}`。RPC 保留 stdout 回空決定的時間；磁碟／OS 排程不是硬即時保證。

| 持久資料 | 行為 |
|---|---|
| `$AGEND_HOME/spool/{hooks,acks}` | 0700、JSON 0600；`ClaudePendingRecord { version:1, request }` |
| counter／序號＋UUID 檔名 | flock 下先 fsync counter，再 temp write／fsync／rename／directory fsync；crash 可留空號 |
| home-wide spool/lock | helper 在原 deadline 內等待；daemon scan 不阻塞等待；拒絕 symlink／非普通檔案 |
| replay | 只有 Hook／Ack；hook 強制 replayed=true，不能做 Attach／Poll 或重新投遞 |
| 刪除 | daemon commit 回覆後才 unlink／directory fsync；不按日期刪未同步檔 |
| daemon ingest | 每秒掃 hooks／acks，每批 32；跨 batch 旋轉，保留的壞檔／拒絕檔不阻擋後方 ACK |

helper 不開 agend.db／SQLite／Tokio；daemon 是唯一 DB owner。channel EOF 退出，daemon 仍可自動入庫留下的 spool。未發布的 own `.tmp` 可在 lock 下清掉；已發布但拒絕的 JSON 保留供診斷。

## 自動驗證

`agend/tests/claude_bridge.rs` 共 16 個 cases：native helper／真 daemon／holder／SQLite，MCP initialize／ACK 與 hook payload 使用 testkit producer。包含 Sent／Confirmed 分界、Stop 防迴圈、離線 spool、helper 退出後 ingest、四次開機與延遲 ACK、拒絕身分／版本／session、保存失敗、壞檔前綴及真 stdout 背壓。另用 native flock interposition 在首次解鎖後停止真 helper，驗 busy 事件先 live 入庫；helper 發布後死亡的歷史事件也只能撤銷舊 idle。

四次開機 case 以 native service 已提交回覆模擬 helper 尚未寫出即死亡；另有實際 Stop helper 的 unread pipe 逾時，驗部分 stdout 後 intent 仍 unknown 且下一 Stop 不重送。這不代替真 Claude 已讀或完整 Driver 的四次開機契約。

初輪 workspace 抓到測試在 stdout 到達後先停 daemon、Written 尚未提交的 race；改以 MCP ping 完成 barrier，保留原失敗。第二輪 CLI 表仍期待 real daemon 1.4；只更新 real producer 版本參數為 1.5，fake 1.3 保留。最終 head 的 r2 CONFIRMED／CI 通過與使用者重驗紀錄見頁首；原失敗不改寫成成功。

## 可重驗指令

```bash
# 固定已驗 head，自行建立／清理驗證 worktree 與 target。
bash /private/tmp/g12a-bridge-review/reverify.sh
```

fixture 會停自己的 daemon／holders 並刪 home；trap 清掉自己的 target。正式驗證另跑 workspace tests、fmt／clippy、accept core 與實際 no-std；本批未呼叫真 Claude／Codex／OpenCode。

## 下一步

#149 已合併；接續 [共用 gh 防護](gate-12a-gh-shim.md)、完整 Claude Driver／啟動設定與 Interrupt。真 CLI 版本檢查／重錄／模型回合另取明確授權。
