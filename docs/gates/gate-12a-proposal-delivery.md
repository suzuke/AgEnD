# 第 12A 提案：P6–P9 送達與清掃

> **TL;DR**
> - 三級忙碌、送達確認、PATH 與孤兒清掃。
> - P6–P9 已確認；P7 採明確 ACK 與保守恢復，既有 Codex 行為維持已驗範圍。
> - 下一步：依 D40 寫實作與反例；目前尚未接入 Claude。

## P6：claude 的三級忙碌與忙／閒

- 問題：Queue、Steer、Interrupt 怎麼落到 Claude，daemon 怎麼判斷忙閒？
- 已確認：忙＝UserPromptSubmit；閒＝沒有要求續行的 Stop、SessionStart 且啟動完成；Stop 回 queue 要求續行則維持 busy。Pre／PostToolUse 只記事件，沿用 D30 去抖動及 hard gate 優先；舊 hook 不建立當下 idle，恢復核目前 session／畫面。
- 閒置走 channel；busy Queue 留 daemon，下一個可取 queue 的 Stop（stop_hook_active: false）將完整內容合成 reason 回 block。交出內容前持久化投遞紀錄，取出的批次不能只存在記憶體；續行時不再 block 形成迴圈。
- Steer 經既有 effective_level 改 Interrupt；只在可中斷工作狀態、holder 允許控制時送單一 Esc，成功後立即 channel、不等 Stop。有人工 owner 不搶權，拒絕／斷線保留訊息並回報。Esc 可能關閉對話框，不能只憑寫鍵成功宣稱工作已停或訊息 confirmed。
- 不用 Ctrl+Enter／Ctrl+X Ctrl+S；官方說明的是 TUI queued input，不能假定適用 channel（[interactive mode](https://code.claude.com/docs/en/interactive-mode)）。
- 理由：沿用 D16 的 idle channel／busy Stop 歷史證據與現有 core 等級對應；補上第 11C owner 限制。
- 未採用：busy 直接走 channel；尚未查證的 Ctrl+Enter 強送。
- 例子：busy Queue→queued；Stop 回 block 並成功寫出→sent；有效 agend_ack→confirmed。下一個 active Stop 或背景 task hook 不能代確認。F8 是背景工具另起一輪的歷史證據，新版仍須驗。
- [x] 使用者確認（忙閒與等級分別同意；[確認紀錄](gate-12a-confirmations.md)）

## P7：claude 的送達確認與事件

- 問題：sent／confirmed 的證據、daemon 離線及投遞途中崩潰如何處理？
- 已確認：queued＝已保存且尚未送出；sent＝channel 通知或 Stop 續行回應成功寫出；confirmed＝有效 agend_ack。兩條路徑推完整內容、訊息 id 與投遞識別碼，Claude 收到先 ACK 再工作，可按批確認；daemon 核對訊息 id、投遞所屬 session／識別碼。不把 ACK 當 task 完成，漏 ACK 留未確認。
- ACK：bridge 先持久化；daemon 離線回「已保存、待同步」，不宣稱 confirmed。恢復只補 ACK、不重送內容；daemon 同交易核對、保存收件與狀態後才回成功，待送檔才刪。重複 ACK 冪等，本機保存失敗回錯誤。
- crash：開始前保存投遞紀錄。確定未開始可送；成功寫出未 ACK 留 sent；已開始但缺結果標「投遞結果不明」，等待證據或人處理，不自動重送內容。實際未送到的訊息也可能暫停，不承諾 exactly-once。
- 狀態：沿用四個 DeliveryState；結果不明另存持久化投遞 metadata／顯示原因，不能誤走 queued 重送路徑。有效 ACK 可補 sent 再 confirmed，詳見 [D40](../decisions/d40.md#收件與恢復的邊界)。
- 事件：新增 daemon 管理的 driver_events，依入庫 seq 作 cursor，留 14 天。訊息／收件狀態獨立保存，不依賴重播日誌；未同步 hook／ACK 等入庫才刪；未終結 Claude 訊息與必要投遞資料不按 30 天刪，直到 confirmed 或人明確放棄，之後依原 30 天規則。這是 D31 的本次 Claude 接入例外；[store 基礎](gate-12a-store.md) 已實作，Codex／Claude inbox 的既有 retention 不變。
- migration：開工時取下一空號；本批已在既有 0005、0006 後新增 0007，未改已發布 migration。
- 理由：channel 寫 transport 成功沒有 backend ACK；stdout 與 DB 不能同交易；保留待同步資料，明確顯示不確定性。
- 未採用：UserPromptSubmit 或下一個 active Stop 代 ACK、transcript 格式當唯一恢復來源、結果不明開機盲目重送。
- 例子：daemon 停著時 Stop 回 {}、hook 留 spool；恢復只補事件並核目前狀態，不因此 confirmed。有效 ACK 待送檔入庫後才確認並刪除。
- [x] 使用者確認（定義、ACK、crash、ACK 離線、事件與訊息保留逐項同意；[確認紀錄](gate-12a-confirmations.md)）

## P8：claude 不需要 ZDOTDIR

- 問題：第 7 關 Codex 需要 ZDOTDIR，Claude 是否也加？
- 已確認：**不加**。F6 是 2026-09-28／2.1.283 的非 login shell 與 shim 優先歷史實測；選定版本須驗 git、kill、pkill、killall 及新增 gh 的實際 shim 路徑，function 也驗最終解析。不由歷史證據宣稱新版通過；失敗則根據證據調整啟動方式。
- 理由：沿用目前 ZDOTDIR 只給 Codex 的規則，不預先擴大環境白名單。
- 未採用：沒有新版失敗證據就先加 ZDOTDIR。
- [x] 使用者確認（「好」；[確認紀錄](gate-12a-confirmations.md)）

## P9：holder 死掉後清掃 claude

- 問題：holder 死後 Claude、channel／背景工具可能留在舊 process group。
- 已確認：納入 A 段，沿用第 7 關 P2 清掃條件、時機及已接受 race；只補 Claude 的 argv marker。--session-id／--resume 後的元素須與記錄 session 完全相等；或 agend channel 子命令的 --instance 值與記錄 instance 完全相等，不用子字串。
- pgid 範圍、holder 已死、group 仍有程序及精確身分都符合才 killpg；無 marker 不殺，保留診斷，不保證所有脫離 group 的程序可清掉。涵蓋 daemon 在線死亡、停機後開機、failed 且 holder 已死及 retry 前的既有清掃時機。
- 理由：沿用已驗機制，避免把存活的其他 instance 或 pid 重用者誤殺。
- 未採用：另開施工關才補清掃；只憑程序名稱批次 pkill。
- [x] 使用者確認（剩餘依建議；[確認紀錄](gate-12a-confirmations.md)）

## 送達仍須補證據

在 intent、完整寫出、ACK 本機落地、daemon 入庫、刪待送檔各點斷線；驗去重、結果不明停送、延遲 ACK、錯 session／識別碼、批次部分 ACK 與 14／30 天保留邊界。這些尚未執行，不把 fake 計數當真 Claude 收件。

## 既有整合要保持

- 第 11C 多視窗 owner、resize、斷線與人工輸入須回歸，Codex 的 clientId 對帳不能代 Claude ACK。
- pipeline_runtime.rs 目前組裝 CodexDriver；Claude 接入須驗 task dispatch／review，不只單獨 send。
- P9 精確 marker 與 pgid 條件須含其他 backend／pid 重用的拒絕反例。

## 下一步

回[施工關入口](gate-12-adapters.md)，依 [D40](../decisions/d40.md)整理送達與清掃實作稿。
