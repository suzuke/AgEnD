# 第 12A 提案：P10 與驗收計畫

> **TL;DR**
> - P10、真 CLI 待查項、A 段限制與未來驗收案例。
> - 本頁是完整接入計畫；client 已 merge、store 已實作待驗證，A 段尚未完成。
> - 下一步：依已確認 D40 寫定實作與驗收指令；真 CLI／模型回合另外確認版本與預算。

## P10：什麼是假的、什麼是真的

- 問題：CI 不跑真的 Claude；fake、錄製一致性與真 CLI 各負責什麼？
- 已採用：CI 對現有 `fake-claude` 補 bridge、spool、設定與 DRV 契約，包含 DRV-6／DRV-9 四次開機。真 CLI 一致性是必要完成條件，使用者或另獲授權的 agent 檢查 `claude --version` 與 transcript header；版本不同，重錄後修 fake，再跑 conformance。完整 `claude_live` smoke 原提案約 3 個短回合、選做，驗收紀錄註明有無執行；不以 fake 結果代認證真 backend。
- 理由：CI 驗邏輯；只有真 CLI 能驗啟動旗標、hook 形狀、PATH 與選定版本真的接受訊息。
- 替代方案：CI 跑真 Claude，需登入、花費與外部服務，不作預設。
- 例子：未來 demo 要列每個 DRV case 與真正的結果，不承諾目前不存在的 `9/9 pass` 字樣。
- 關係：D9、第 7 關的真測授權方式，以及 2026-09-25 使用者要求的版本一致性。
- [x] 使用者確認（剩餘依建議；[確認紀錄](gate-12a-confirmations.md)）

原錄製檔 2.1.282、2026-09-28 實測 2.1.283。這次只讀文件與程式，沒有執行真 Claude、重錄或新增模型回合；當時 5 回合的授權不延伸至新測試。將來如選做真 smoke，須先報版本、完整命令與預算，另獲授權。

## U1–U4 待查

| 項目 | 未證明的行為 | 對應 |
|---|---|---|
| U1 | 兩條路徑的 `agend_ack` 工具、訊息 id／投遞識別碼／session 關聯可核對 | P7；精確 schema、重錄／conformance |
| U2 | Ctrl+Enter 對 channel 訊息是否有效 | P6 仍不用；若要改方案先真測 |
| U3 | P2 設定與 P3 A bypass 能啟動，P4 共用 gh shim 正反例與 PATH | P3／P4 已選 A；不認證絕對路徑／直接 API 攔截 |
| U4 | 信任與 development channels 提示在不同 terminal 寬度的 fixture | P5；選定版本至少兩種寬度 |

2026-10-03 查核的官方參考：[hooks](https://code.claude.com/docs/en/hooks)、[permissions](https://code.claude.com/docs/en/permissions)、[channels](https://code.claude.com/docs/en/channels)。文件會變，不能取代指定 CLI 版本的真測。

## A 段原提案的限制

- 人在 TUI 按 Esc 中斷 Claude，F7 沒有後續 Stop，daemon 可能仍當它忙；queue 等下一次符合條件的 Stop，或依 P6 的 daemon interrupt 處理。
- 首次啟動、第一則訊息前死掉且 Claude 未存 transcript，F9 的 resume 失敗；照第 6 關只 resume 的規則，重試可能仍 failed，需要移除再建立。
- 未知提示、授權轉問人、`PermissionRequest` 自動回答、卡住／usage limit 偵測與自動選忙碌等級不在 A 段。
- P5 已選 A；未知啟動提示仍交人。原 C 不用 channel 只是比較選項，未採用。

## 自動驗收計畫

下列是完整 A 段接入案例；client #147 已 merge，[store 基礎](gate-12a-store.md) 已實作並跑本批持久化回歸，未代替 bridge／runtime 或 DRV 驗收。現在 `cargo xtask accept adapters` 只跑 daemon 檢查，印 `demo not implemented yet`，不代表第 12 關完成。

- [ ] core：P1 協定新增／舊 peer、P5 選定規則與真 fixture；`cargo test -p agend-core`、check-deps、`cargo xtask accept core`（D22）
- [ ] bridge／hooks：instance 歸屬、version negotiation、write receipt、timeout；spool 有序補送、重複事件去重、daemon 不在時 Stop 回 `{}`
- [ ] 送達：idle channel、busy Stop 取 queue、防止 hook 迴圈、steer→interrupt、Esc 後不等 Stop；背景／其他 prompt 的 Stop 不能代確認
- [ ] ACK：兩條路徑、批次／部分 ACK、錯 instance／session／訊息／投遞識別碼拒絕；普通 hook 不代確認；重複 ACK 冪等
- [ ] ACK 離線：保存後回待同步、寫檔失敗工具錯誤；DB 寫入前後崩潰／補送不重派內容；入庫回覆後才刪 pending
- [ ] crash：投遞紀錄持久化、channel／Stop 寫出前後與確認前後中斷；DRV-6／DRV-9 四次開機；結果不明顯示且停送，有效 ACK 可補 sent 再 confirmed，未確認不假成功
- [ ] 設定：非 agend 檔案雜湊不符拒絕覆蓋；inbox fake-worker 不套 push 設定；選定 P3／P4／P5 的正反例
- [ ] runtime／pipeline：Claude Driver 接 task dispatch／review；第 11C owner／resize／人工輸入不破壞 queue 與確認；shim PATH 與 sweep 不誤殺
- [ ] DB／spool 整合：0007／schema v1–v7 fixtures、14／30 天邊界、未終結例外與延遲 ACK／人工終結已由 store 回歸覆蓋，本批驗證結果見 #148；接入後仍需核未同步 spool 不到期刪與入庫後刪檔；既有 Codex retention 不變，不改已發布 migration
- [ ] fmt、workspace clippy／tests、check-deps 實際 no-std、完整 adapters demo、crate README／TESTING 更新、全新無相關 context verifier

## 真測與使用者可重驗

原提案的驗收目標保留：版本一致性、可選的約 3 回合 `claude_live`、兩個真 Claude 的閒置與忙碌送達、既有 CLAUDE.md 不覆蓋、最後無 holder／channel 孤兒。兩個真 instance 的回合數在寫定腳本後報完整預算；不是由此計畫預先核准。

依使用者要求，可自動化的 fake／native／多視窗行為由 agent 與 fresh verifier 執行，附可複製的重驗命令及預期結果。真測命令等選定版本、完整命令與預算另獲授權後再提供；每個終端都用自己的 worktree、絕對 `CARGO_TARGET_DIR`／`AGEND_BIN`，避免舊 Node CLI。

完成後清掉本任務產生的程序、fixtures、編譯目錄、helper；必要驗證結果保留。待 merge 的 implementation worktree 保留供驗收，合併後移除。驗收與 merge 仍等使用者確認。

## 驗收紀錄

| 日期 | 結果 | 範圍 |
|---|---|---|
| — | 尚未執行 A 段驗收 | 文件整理不代表通過 |

## 下一步

回到[施工關入口](gate-12-adapters.md)，依 [D40](../decisions/d40.md) 寫定實作與驗收；完整 A 段案例仍待接入；已完成的 client／本批 store 檢查各以對應 PR 證據為準。
