# 第 11 施工關 C 段：驗收計畫與完成證據

> **TL;DR**
> - 此頁保留 C 段必須證明的完整行為；#145 已完成驗收並經使用者確認合併，結果見 [收尾紀錄](gate-11c-closeout.md)。
> - 測 consumer 必須餵真 producer：holder parser／協定型別、fake daemon 或真 AgEnD；不能手寫理想化 frame 當通過證據。
> - 下一步：第 12 施工關提案另等逐項決定；以下原矩陣與環境限制保留，不以合併代替 runtime 證據。

## 目前執行情況

逐列 source／證據與限制見 [矩陣執行狀態](gate-11c-matrix-status.md)；App 舊 frame／query 與 wire golden 已有 [直接回歸](gate-11c-frame-order-validation.md)。

已建立的 suites／fixture 與原始證據見 [實作進度](gate-11c-progress.md)、[App 驗證](gate-11c-app-validation.md)、[原生 renderer](gate-11c-native-validation.md)、[真 PTY App](gate-11c-native-app-validation.md) 及 [真外層 PTY](gate-11c-outer-validation.md)。CLP-23–28 已同跑 fake／native，fake 注入真正 holder parser。下表按真 producer、native／外層 PTY 的原斷言與原實機紀錄完成核對，最終固定 head 的獨立覆核、雙平台 CI、清理與使用者合併確認見 [收尾紀錄](gate-11c-closeout.md)。首次真 Codex 0.159.3 U17 的[原始證據](gate-11c-u17-live-validation.md) 與[完整 fake U17](gate-11c-u17-validation.md) 仍分列，不互相代替；實機環境限制仍保留。正常本機最後 dirty 的端到端時效已有外層 PTY 證據，包含未加 round-trip 額度的 300 ms 正向及 800 ms mutant。

## 自動驗證矩陣

| 範圍 | 實作後必驗 | producer／失敗檢查 |
|---|---|---|
| core／協商 | client 1.4↔1.3、holder 1.1↔1.0；舊請求不變、新能力未協商不送；frame golden、尺寸／長度拒絕 | 真型別 serializer；old-peer fixture；metadata 與 no-std 仍通過 |
| holder renderer | 16／256／RGB 色、bold／underline／inverse、游標、CJK／combining、寬格跨邊界；normal／alternate 切換 | 真 PTY 程序印序列，holder parser 輸出格子；切片在 UTF-8／escape 中間也相同 |
| 畫面／更新 | generation／revision 單調；最後一段輸出送到；resize 尺寸與完整畫面相符 | producer 連續輸出後停止；延遲／舊 generation frame 不蓋新畫面 |
| 歷史 | >1,000 行淘汰；捲上後新輸出不跳；回底才跟；alt screen 不捏造歷史 | 真 PTY 編號行；逐 byte 核歷史；沒有歷史就不能冒充可回捲 |
| daemon／權限 | agent caller 的控制、resize、輸入全部 forbidden；舊 attach、結束的 instance 無副作用 | 同套 client 契約跑 fake daemon／真 daemon；每項拒絕核 PTY 尺寸／實收 bytes 不變 |
| 多視窗 | 最後 i 取得控制；舊視窗唯讀、舊 resize／input 拒絕；重取控制、EOF 清理 | 兩個不同尺寸真 client，核 holder 尺寸及控制者；連續 resize 不搶回控制 |
| TUI renderer | 一列狀態列；cursor／attrs／寬格不偏；不同高度及極小視窗；退出恢復導航 | holder frame 經 ClientSource 到 ratatui TestBackend，逐 cell 核內容與 style |
| 鍵／paste | Ctrl-] 不送，q／Esc／Ctrl-C 可送；paste 中退出碼當資料；mode 編碼正確；超大整次拒絕 | 真 input 收集程序；holder mode 源自 PTY 序列，核原 bytes；雙連線 reply 不串台 |
| mouse | mouse mode開時座標與按下／放開／移動依模式；無mode滾歷史、Shift分流；status列不送 | 真 mode producer＋crossterm events，核實收 bytes 與 viewport |
| 斷線／壓力 | daemon 中斷、holder 重起、slow peer、過大 frame；重連唯讀；無 thread／fd／fixture 洩漏 | 真程序／socket；有限 timeout，超限拒絕不截斷；開關 20 次只剩既有事件／請求連線 |
| Codex U17 | 相同 thread、手動 turn／busy／idle、messages／receipt 不偽增、排隊派工與重啟 | fake Codex PTY＋app-server；真 Codex opt-in 別列結果，不用 fake 代替 live |

時效目標：新畫面最後 dirty 在正常本機環境 300 ms 加一趟 holder round-trip 內可見；50 ms 節流本身不等於端到端保證。測試用等待條件與明確 deadline，不靠固定 sleep 當成功。

既有完整命令繼續執行：

```bash
~/.cargo/bin/cargo fmt --all -- --check
~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
~/.cargo/bin/cargo test --workspace
~/.cargo/bin/cargo xtask check-deps
~/.cargo/bin/cargo xtask accept tui
```

新 acceptance demo 與 fake fixture 要納入 `accept tui`；實作前不杜撰可執行的新 example 名稱。新跨 crate 契約另依既有 CONTRACTS 表續編，不預占未存在的 CLP 編號。

## 你親自驗收（實作後，一次帶一步）

### 2026-10-03 執行方式更新

使用者已操作下表中的實機畫面／鍵鼠／貼上／歷史／多視窗，並要求後續驗證由 agent 自動化完成。剩餘行為以真 producer、native PTY、外層 PTY 與相同斷言核實；已取得實機紀錄與自行比對分開保留於 [收尾頁](gate-11c-closeout.md)。此指示改變操作方式，沒有刪除下表或自動矩陣的驗證要求；#145 的合併確認已於後續取得；其他 PR 仍須另行確認。

以下保留原人工操作清單，用來對照已取得實機紀錄與剩餘自動化斷言；不再要求使用者重做每一步。測試仍在專屬 worktree、獨立 CARGO_TARGET_DIR 與測試 home 執行，完成後提供可重跑的驗證指令。清單不是本次提案已做完的勾選。

1. 確認 Rust CLI 與固定 head，跑完整 `accept tui`；假、真 daemon demo 均成功。
2. 啟動有輸入框／色彩／游標與長歷史的測試 agent，`agend app` → `t` 唯讀 → `i` 完整模式；只留一行狀態列，輸入／退出鍵各到正確地方。
3. 改終端視窗大小，核 agent 回報實際 rows／columns；再開第二個不同大小視窗，確認原視窗轉唯讀，重取控制可恢復。
4. 滾輪看歷史、持續新輸出、回到底部；agent 開啟 mouse mode 時點選／滾輪到 agent，Shift 滾輪或唯讀方式看歷史。
5. 貼多行繁中與含 Ctrl-] 的文字；agent 看到一次貼上、AgEnD 不誤退出；故意送超限 paste，拒絕且不出現半段資料。
6. 用 agent caller 故意要求 resize／input，應 forbidden、尺寸與內容不動。中斷 daemon，holder 持續；重連畫面完整且唯讀，須再按 i 才輸入。
7. 首次真 Codex opt-in smoke 另記 CLI 版本、thread／turn／messages／忙閒與重啟證據；未驗或未獲核准的版本保持 not_supported。已核准的 0.159.3 不因額外診斷 run 未執行或失敗而自動撤銷；額外真模型回合須另獲授權。
8. Ctrl-] 後確認外層快捷鍵正常；結束測試 daemon／holders、刪自己成功 fixture，核 fd／程序無殘留。

macOS Terminal／iTerm2 與使用者實際 Linux 終端分開記錄；CI 的 headless 測試不認證實際字型、游標外觀或非美式鍵盤。至少一次故意拒絕與一次重連的原輸出必須保留。

## 已確認提案如何核對

```bash
cd "<你的 AgEnD worktree>"
sed -n '1,150p' docs/gates/gate-11c-proposal.md
```

核對六個確認勾選與使用者回覆；尤其 P3 的多視窗唯讀規則、P5 的 Codex 開放門檻。勾選只表示設計已確認，不能算 runtime 或 Codex U17 驗收通過。

## 下一步

提案 #144 與實作 #145 均已合併；最終固定 head、獨立反證、CI、原實機／自動驗收與清理見 [收尾紀錄](gate-11c-closeout.md)。下一關提案另等使用者確認。
