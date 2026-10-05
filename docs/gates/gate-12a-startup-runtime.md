# 第 12A：正式啟動提示與初始 idle

> **TL;DR**
> - 已知完整信任／development channels 畫面經 holder 單鍵 Down／Enter；未知畫面留給人。
> - SQLite 先存按鍵 intent；結果不明停送，接回 holder 不重設。初始 idle 要 live SessionStart＋已知主畫面，穩定五秒。
> - 下一步：本批 native 驗證與全新 verifier；不宣稱新增真 CLI 執行或完整 12A 驗收。

## 範圍

接在 draft #151 的 `53f4ea6`；本批實作 [D40 P5／P6](../decisions/d40.md)。
core 新增完整 frame 規則資料與啟動嘗試型別，沒有新增 pipeline 事件或忙閒事件。
原 `SCREEN_RULES` 繼續擋 hard gate；完整規則只授權已知啟動鍵及辨識初始主畫面。

| 畫面 | 動作／門檻 |
|---|---|
| 信任選 No | 只有自己的 canonical workspace；穩定完整畫面一秒後 Down |
| 信任選 Yes | 同一啟動嘗試；自己的 canonical workspace，Enter |
| development channels | 完整唯一 `server:agend` 選單，Enter |
| 已知主畫面 | Claude 2.1.284、自己的 canonical workspace；不送鍵 |
| 未知／額外選項／錯路徑／其他尺寸／alternate screen／斷線 | 不按鍵；不建立初始 idle |

支持錄製的 100／140 欄、24 列。新 Claude push holder 在 Spawn 前設 100×24；
一般 runtime、Claude inbox 及接回既有 holder 不強制 resize。
private startup capture 在啟動 daemon 前登記 `manual`：停用 P5 自動鍵及初始 resize，
所有輸入由已授權的蒐證工具處理，維持被動蒐證的零按鍵邊界。raw PTY 測試也明確登記此模式。
自動鍵不 Acquire 人工 owner；holder 在實際 PTY write 比對 generation 與 revision。
unknown UI 仍可由操作者開啟終端處理，不能把一句「ready」當成啟動完成。

## 持久化與恢復

migration `0009`／schema v9 的 `claude_startup` 每 instance 一筆：session、launch、
物理 holder generation、halted、diagnostic `manual` 與三種 key 的 intent／written。
真正新建 holder 前建立新 launch；daemon 接回既有 holder 不建立新 launch。
缺啟動紀錄的舊 holder 不自動按鍵。

送鍵前先 commit intent。同鍵已完成不重送；任何尚缺完成的 intent 都停止後續自動鍵。
只有 holder 明確的寫入前拒絕可釋放自己的 intent，重新等一秒穩定畫面後再試；
逾時、EOF、錯 generation 回條與 daemon 重啟均不自動釋放。
恢復不依賴 14 天 driver 歷史；紀錄只在新啟動替換或 instance 刪除時 cascade。

有效 UserPromptSubmit／Stop／SessionEnd 停止啟動按鍵；Ready 也停止本次自動啟動。
SessionStart 可以先於 Ready，也可以晚於 Ready；只配對 fresh live hook，
historical／duplicate hook 不建立初始 idle。初始 idle 候選失去已知畫面或 generation 變動即撤銷。
busy hook 不等待背景畫面查詢；初始投遞前再次核 live frame、session 與路由 revision。
Stop 的既有忙閒／續行／ACK 邊界維持 D40，不把按鍵成功當成訊息收件。
首次投遞 poll 完成前持續核畫面，不能在五秒到期後停止；首次 poll 成立才退出啟動採樣。
全新 verifier 在 `1beb03a` 重現「Ready 五秒→未知一秒→Ready 200ms」誤投遞；
原 REFUTED 保留，已移除提前停採樣條件，新增恢復後重等五秒及最終可投遞的 native 回歸。

第二位全新 verifier 在 `d81a7f3` 重現 P5 背景取樣與人工 TUI 共存時漏掉最後 modes：
holder 的 50ms 共用 sample 尚未更新，terminal hub 卻清掉 dirty。REFUTED 與只保留 dirty
便成功的因果對照均保留。修正後若抓 frame 的起點距最新 notice 不足 50ms，保留 dirty
到下一輪取樣；不延後首 frame，也不持續空轉。新增自動取樣下 history／focus／20 次 App 清理與尾段輸出 300ms 預算回歸。

## Fixture 與驗證邊界

新增五份 main UI fixture 直接來自先前已獲授權的兩寬真蒐證，不改原 JSONL：
100 欄的 line 18／19／20、revision 22／24／25；140 欄的 line 17／18、revision 25／26。
主畫面路徑仍遮蔽為 `<rec>/h1/workspace/g12-startup-capture`。
[來源與 SHA](../../crates/agend-core/tests/fixtures/screens/README.md)；
匯出核對保存於 `/Users/suzuke/Documents/Hack/AgEnD-ops/g12a-startup-gate-20261005/main-ui-fixture-export.json`。

native producer 在自己的臨時 cwd 重播文字，經真 daemon、holder、PTY parser 與 SQLite 驗證；
只替換本 lab 的 workspace。測試不啟動真 Claude、不查版本、不送模型 prompt 或團隊訊息。
回覆遺失故障注入丟棄真 holder 的完成回條，保留原 request／response producer。
歷史真蒐證證明文字來源；本批 native 測試證明正式程式行為，兩者不能互相代替。

## 可重跑

```bash
cargo test -p agend-core
cargo test -p agend-daemon --test store --test claude_startup_store
cargo test -p agend --test claude_bridge
cargo xtask accept core
cargo xtask check-deps
```

開任何 task／verifier worktree 前先核對上一批已完成的自有程序、暫存、target、worktree／branch。
必要證據及尚未合併、仍供驗收的目錄記錄保留原因；不強制刪除外來／dirty worktree。
測試 Lab 會清理自己的程序與 HOME；最終 verifier target／worktree 收尾後移除。

## 下一步

完成本批獨立驗證、CI 與人工重驗後等待使用者確認 merge；P8 真 CLI shim 路徑、
完整第 12A 驗收等尚未完成範圍見 [Driver 進度](gate-12a-driver.md)。
