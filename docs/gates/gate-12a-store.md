# 第 12A：Claude 持久化基礎

> **TL;DR**
> - #147 client 已合併；本批提供 DB-thread 的事件、投遞、ACK 與人工放棄 API。
> - 訊息 id 是唯一訊息冪等層；投遞已開始但缺結果時保留 queued 與 metadata，不自動重送。
> - 下一步：本批驗證與使用者確認後，接 protocol 1.5、channel／Stop helper 與 Claude driver。

## 狀態與範圍

實作中（2026-10-04），分支 `feat/gate-12a-claude-store`；以 #147 merge `8dfccf8` 為基線。本批 API 可直接對真 SQLite 使用；尚無新的 socket 請求、MCP 工具或 operator CLI，不能用本批宣稱 Claude 接入／完整第 12A 完成。通訊路徑仍提供既有 1.3／1.4。

## Schema 與原子邊界

migration `0007_claude_delivery.sql`（schema v7）只新增，不改舊 migration。

| 資料 | 保存與限制 |
|---|---|
| `driver_events` | event UUID、instance、session、kind、原生 JSON object（≤1 MiB）、source／ingest 時間、replayed；AUTOINCREMENT seq，不由事件重播確認訊息 |
| `claude_deliveries` | 訊息 FK、原 instance、單次 delivery UUID／session／route、started／sent／confirmed、人工放棄時間及原因；不依賴 instance FK |
| 分類 marker | messages INSERT trigger 同一 statement 捕捉 Claude `delivery=push`；migration backfill 既有 Claude push 訊息；instance 刪除不抹去分類 |
| 新預約 | `reserve_claude_delivery` 同交易寫投遞開始與 attempted_at，commit 後才回 `Started`；要求 running、session_started、目前 session 相符 |
| 重複預約 | 同 tuple 回 `Existing`，絕不授權寫出；不同 delivery／session／route／instance 拒絕，未提供人工重送介面 |
| 寫出結果 | `claude_delivery_written` 同交易存 sent 與寫出觀察時間；缺結果維持 queued，`outcome_unknown` 為 true |
| ACK | `acknowledge_claude` 核 message／delivery／instance／投遞 session；有效 ACK 可補 queued→sent→confirmed，同交易保存 metadata；重複 ACK 不刷新 terminal 時間 |
| 一般 receipt | generic message advance 不接受 Claude push，不能靠 task result／普通事件取代 ACK |
| 人工終結 | `abandon_claude_delivery` 要非空原因，queued／sent→failed；重複放棄不刷新時間；confirmed 不能再放棄 |

ACK API 要求其呼叫者先認證 instance；本批是 store 邊界，不能把任意資料庫函式呼叫視為已認證的 socket／MCP caller。核對的是投遞保存的 session，所以新 session、instance 移除或重建不拒絕舊有效 ACK。

legacy Claude push 訊息若已有 attempted_at 卻無投遞 tuple，視為結果不明、禁止預約；保留給人處理。legacy confirmed／failed 仍依原 terminal 狀態清理。

## 事件與保留

同 event id 的原 instance／session／kind／payload bytes／source time 不變則回原 seq；replayed 是傳输 metadata，重送時可不同，第一次保存值不變。不同內容拒絕。讀取依 seq、limit 1–1024；事件去重只在 14 天紀錄存在期間成立，不能替代未同步 spool 或 ACK 資料。

| 資料 | 保留規則 |
|---|---|
| driver_events | 入庫時間起 14 天，邊界當下保留；來源時間很舊的補送不立即被刪 |
| Claude push queued／sent | 持續保留；含未預約、結果不明、instance 已刪除 |
| Claude push confirmed／failed | terminal update 起 30 天，duplicate ACK／abandon 不延長 |
| claude_deliveries | 隨 messages FK cascade；prune report 同時呈現 child 被刪數量 |
| Codex／Claude inbox 等原有訊息 | 原 created_at 起 30 天不變 |

事件刪除不阻止已保存的投遞核對延遲 ACK。快照非空判斷包含兩張新表；SQLite seq 與 tuple 經 VACUUM INTO 還原不變。既有 Codex／Claude inbox 等一般 messages 單表資料仍視為空，本批未擴張其快照行為。

## 驗證

本機首輪 store integration 7／schema 40 tests 通過，daemon crate 回歸通過；`accept core` 通過，含 fmt／workspace clippy／實際 no-std（兩個既有 explorer ignored）。後續補事件單表快照／seq 回歸；`6193ea4` fresh verifier REFUTED：事件單表 DB 被快照的舊 empty SQL 誤判，新增回歸失敗；補入 driver_events／claude_deliveries，保留原 log。修正版 daemon 162 passed／0 ignored（其中 store integration 9、schema 40、交易單元 2）；workspace clippy／fmt／實際 no-std 通過。完整 workspace、另一位全新 verifier 與固定 head CI 另核，結果以 PR #148 為準。本批不執行真 Claude、模型回合或錄製。首個 crash probe 在 store commit 後硬殺自己的 child；四次獨立程序重用同 home，另驗新 home 必須失敗。這是持久化探測，未代替完整 Claude DRV-6／9。

可重驗（暫存編譯目錄在離開 shell 時清掉）：

```bash
cd /Users/suzuke/AlphaCR-worktrees/AgEnD-v2-claude-store
(
  set -e
  gate12_store_target=$(mktemp -d /tmp/agend-g12a-store-check.XXXXXX)
  export CARGO_TARGET_DIR="$gate12_store_target"
  trap '~/.cargo/bin/cargo clean' EXIT
  ~/.cargo/bin/cargo test -p agend-daemon --test claude_store
  ~/.cargo/bin/cargo test -p agend-daemon store::claude::tests
  ~/.cargo/bin/cargo test -p agend-daemon --test store
)
```

## 下一步

完成本批 fresh verifier／CI 並報告可重驗指令，merge 等使用者確認。後續 helper 使用本批 API，在內容寫出前取得新預約，在 ACK 入庫 commit 後才刪待同步檔；忙閒與控制權、spool 原子發布、protocol 身分與真 CLI 版本驗收另接。
