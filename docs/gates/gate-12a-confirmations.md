# 第 12A 使用者確認紀錄

> **TL;DR**
> - 本頁記錄 2026-10-03–04 的對話確認；以 [D40](../decisions/d40.md) 為目前設計。
> - P3＝A、P4＝A、P5＝A；P7 已改為明確 ACK 與保守崩潰處理。
> - 下一步：文件驗證後由使用者確認 merge，實作及真測另行安排。

## 逐項確認

| 範圍 | 說明的方向 | 使用者回覆 |
|---|---|---|
| P1 helper 到 daemon | 既有版本化 Unix socket，daemon 唯一 DB owner | 同意 |
| P1 離線 | hook 存 pending、成功入庫才刪；Stop 回 `{}`；恢復核目前 session／畫面 | 同意 |
| P1 backend 路徑 | 閒置 channel；忙碌排隊 Stop hook | **「閒置走 channel、忙碌排隊走 Stop hook」** |
| P1 程序 | 同一 agend binary 的 channel／hook 內部子命令 | 同意 |
| P2 設定來源 | project,local＋專用 settings；不讀個人 settings，沿用登入 | ok |
| P2 檔案歸屬 | 只更新 AgEnD 擁有且未修改的 `.mcp.json`／CLAUDE.md；衝突停止啟動 | ok |
| P2 fake 範圍 | 只套 push Claude；inbox fake-worker 維持既有流程 | 同意 |
| P3 權限 | A：bypass，不加原建議 B 的 deny | **A** |
| P4 gh | A：所有 backend 共用 gh shim，防直接 merge／approve 等誤操作 | **A** |
| P5 啟動 | A：自動處理已知信任與 development channels 提示 | **A** |
| P6 狀態 | PromptSubmit busy；Stop 未續行 idle／續行 busy；啟動完成初始 idle；工具事件不切換 | 同意 |
| P6 等級 | Queue→Stop；Steer→Interrupt；Esc 成功後 channel、不等 Stop；遵守人工控制權 | 同意 |
| P7 定義 | queued／sent／confirmed 分開，無對應訊息證據就未確認 | 同意 |
| P7 收件 | channel／Stop 都用 `agend_ack`；核訊息 id、投遞識別碼與 session | 同意 |
| P7 崩潰 | 先持久化投遞紀錄；結果不明不自動重送，顯示並交人處理 | OK |
| P7 ACK 離線 | 先保存、待同步；daemon 入庫才 confirmed 並刪 pending；ACK 可重試，內容不重送 | 同意 |
| P7 事件 | 一般 hook 事件保留 14 天；未同步檔等入庫才刪 | 同意 |
| P7 訊息期限 | 未終結訊息與必要投遞紀錄不按 30 天刪，確認或明確放棄後依原規則 | 同意 |
| P8 PATH | Claude 不加 ZDOTDIR，驗所有 shim 路徑作接入條件 | 好 |

## 剩餘項目一併依建議

使用者在 P9 說明準備期間更新指示：**「剩下都的按照你的建議」**。此次剩餘指本輪第 12A 提案的方向，包含以下原建議；不覆蓋先前 P3 的 A，也不授權未寫定的 B／C／D 細案或真模型預算。

| 項目 | 採用的建議 |
|---|---|
| P1 參數 | 下一個 client minor 1.5、hook timeout 10 秒；精確 wire schema 留實作稿 |
| P9 清掃 | A 段沿用第 7 關 pgid／holder 死亡／完整 argv marker 檢查與清掃時機，補 Claude marker |
| P10 驗證 | CI fake／native、DRV 跨 process 四次開機、真 CLI 版本一致性；選做真 smoke 另核版本／命令／回合預算；完成後清理殘留並保留必要證據 |

## 與舊提案的差異

原提案保存於 [eaab1eef 的第 12A 文件](https://github.com/suzuke/AgEnD/tree/eaab1eef2e9a7710ad0e365e2230a647b8d587a5/docs/gates)，F1–F10 原始實測正文不修改。原 P7 以 UserPromptSubmit 中的 id 或下一個 active Stop 作 confirmed、並籠統稱「當掉後不重送」；目前改為指定 `agend_ack`、ACK 持久化及結果不明停送。原 P6 例子的 active Stop confirmed 因此不再採用。

不把這次設計確認寫成真 backend 通過、不漏不重／exactly-once 保證或已完成第 12 施工關。詳見 [D40 邊界](../decisions/d40.md#收件與恢復的邊界)。

## 下一步

回[第 12 施工關](gate-12-adapters.md)核對 P1–P10 入口與尚未執行的驗收；文件保持 draft，merge 等使用者明確確認。
