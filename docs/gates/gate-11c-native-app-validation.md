# 第 11 施工關 C 段：真 PTY App 驗證

> **TL;DR**
> - App 的鍵鼠／貼上、resize、多視窗與歷史已經真 daemon／holder，到 raw PTY 程序驗證；C 段仍在 draft PR #145。
> - 兩個完整情境逐 byte 比對實收輸入，20 次開關核 thread／fd；本批事件由測試呼叫 App；後續外層 PTY 已補 capture／restore 證據，實機外觀仍待驗。
> - 下一步：完成其餘壓力／時效矩陣與 Codex U17，再做全新獨立及人工驗收。

本頁保存該批次的歷史結果與當時下一步，不能作為目前待辦清單。最新版本許可見 [輸入政策](gate-11c-codex-input.md)；使用者已要求剩餘行為自動驗證，固定 head 結果、實機限制與清理見 [驗收收尾](gate-11c-closeout.md)。

## 如何取得證據

`crates/agend/tests/tui_native_app.rs` 在自己的短路徑 home 啟動真 `agend daemon`。holder 執行重啟的測試 binary；只有 holder 的 instance 身分會進 raw-mode 收件迴圈，未啟動真 backend 或 LLM。agent 印出的 ANSI 經 PTY 與正式 holder parser 變成 frame，再經 daemon／ClientSource 交給 App。

測試以 crossterm events 呼叫完整 App，agent 將實收 bytes 保存到 fixture 檔。另一條 agent 輸出路徑由原子檔案交接觸發，實際 flush 後才標記完成；畫面條件仍要等 App 收到 frame。尺寸另外在 agent 的 PTY 內跑 `stty size`，核對實際 rows／columns。

## 已檢查

| 情境 | 實際斷言 |
|---|---|
| 鍵盤與 modes | q／Esc／Ctrl-C／UTF-8／application cursor 到 raw consumer；q 不退出 App；modes 來自真 PTY ANSI |
| paste／拒絕 | bracketed 繁中多行及 0x1D 為資料；超限 paste 整次拒絕，下一筆 sentinel 比對確認無半段前綴 |
| mouse／focus | SGR 左上 press、右下 release／modifiers；status／區外／未啟用 motion／Shift click 不送；focus 依 mode |
| resize | 真 PTY 由 11×40 改成 8×31；尺寸 ack 前不送 paste，frame 與 stty size 相符 |
| 多視窗 | 15×60 的 B 取走 A 控制；A 的 paste／resize 不改 B；A 明確 i 後以 4×20 重新取得控制；Ctrl-] 不送 agent |
| 歷史 | >1,000 行淘汰；無 mouse tracking 的滾輪與有 tracking 的 Shift 滾輪本機回捲；固定列不隨新輸出移動，淘汰後 clamp 並提示，回底跟隨 |
| alternate screen | 真 PTY 切換；不捏造歷史，回捲不變成 normal history |
| daemon 重啟 | 同 holder pid 保留；offline／重連 paste 不送、控制不自動恢復，明確 i 後才收 FRESH |
| App 資源 | 20 次完整開啟／取得控制／Drop，每次 thread 為 0、fd 回到同一基準；所有 fixture holders 最後停止 |

兩個原生情境與一個 agent 入口測試通過；入口在一般測試程序直接返回，兩個情境內才啟動真正的 raw 收件程序。完整 agend 209／TUI 81 passed、0 ignored；workspace clippy、fmt 與實際 thumb no-std check-deps 通過，沒有 allow-skip。`accept tui` 既有 crate 清單包含 agend，會執行此 suite。

## 原失敗與反例

第一次歷史測試保存快照時只等 App 的 selection 改變，未等 daemon 回傳固定 viewport 的 frame，結果舊 live top 1105 與實際固定 top 1102 不同。改為等真 frame 的 viewport_top 小於 live_top，再保存快照；原斷言保留，通過後才驗後續輸出不移動固定列。

暫時把 application-cursor 編碼改為一般 CSI，同一 raw-consumer 情境收到 `ESC [ A`，預期 `ESC O A`，cargo exit 101。production 檔在 finally 還原，原生 suite 再跑通過。

原 logs 在 `g11c-implementation-logs/native-app-*.log`；本批完成檢查時的 snapshot 為 `SHA256SUMS-native-app`。這些證據驗 App、正式 transport 與真 agent PTY；本批未驗真外層 PTY；後續 [外層回歸](gate-11c-outer-validation.md) 已驗 crossterm event capture／restore。兩批都不認證 Terminal／iTerm2／Linux 終端字型或非美式鍵盤。

## 重連 CI 失敗

`d804580` 的 Ubuntu push／PR 與 macOS PR jobs 通過（各 workspace 864 個主 suite passed／2 個既有 ignored，另有一個 filtered 子程序 probe 輸出；原總計為 865，實際 no-std 通過）。macOS push job `110994132303` 在既有 `reconnect_attempts_are_every_500_ms` 失敗：兩秒只觀察到 2 次，原 log 保留。

舊 fixture 要求 host 兩秒內執行足夠多次 tick。原測試加入一次 2.1 秒受控停頓，即在未改產品碼下失敗；這證明該斷言對排程停頓敏感，CI 未記 tick 時間，無法核實那次 runner 的具體延遲。新 fixture 等三次實際重連，每次以 tick 起訖包住呼叫時間，檢查跨度不能小於 500 ms；有十秒 deadline，正常／受控停頓都跑，r 仍立即再試。暫改產品間隔為 100 ms 時，新斷言 exit 101；還原後整個 20-case suite 通過。沒有降低 500 ms 門檻或改產品的重連行為。

## 重跑

```bash
cd "<你的 AgEnD worktree>"
export CARGO_TARGET_DIR="$PWD/AgEnD-g11c-target"
~/.cargo/bin/cargo test -p agend --test tui_native_app -- --nocapture
```

## 下一步

補 [驗收矩陣](gate-11c-validation-plan.md) 的其餘案例、Codex U17；完整 C 段 ready 後才派全新 verifier，merge 等使用者確認。
