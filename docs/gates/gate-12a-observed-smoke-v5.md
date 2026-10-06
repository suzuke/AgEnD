# 第 12A：固定 observed v5 仍停於初始 idle

> **TL;DR**
> - c7b756b／v5 經另行授權執行一次，180秒初始 idle 逾時 FAILED；零工作訊息、沒有重跑。
> - 兩個 terminal 皆可讀，24份原始frame；兩個新的 Try 建議文字使完整 Ready literal 比對失敗。
> - 下一步：使用者已確認 [D41](../decisions/d41.md) 的 Ready 建議文字可變；修正驗證中，新真 CLI 計畫仍需另取授權，#153 未 merge。

## 固定計畫與結果

計畫 SHA `9457b3d26f0225ad49f971511df576b378e39327dc545dd8af871ab589c80776`。
六份 bytes、c7b756b source、LANG=C.UTF-8 與全新 nonce namespaces 在執行前核對，
使用者「授權」紀錄先保存，然後固定 runner 只執行一次。
預算兩個 Haiku、七則工作訊息、900秒，零自動重跑；實際訊息階段未開始。

一次版本查詢、兩次 instance add、24次只讀 frame 全成功，沒有 no_terminal 拒絕；
707次 status 無任何 idle。每個 instance 一個 live SessionStart、
TrustNo／TrustYes／Development 各 written，manual=0，startup halted=0。
180秒初始 idle 逾時後 exit1，native messages／all_messages 均零，無 delivery 或 ACK。
首次各一份空白frame，後續十一份每個 instance 各自維持同一個建議文字：

| instance | 原始建議 | 正式 classifier |
|---|---|---|
| A | `Try "fix typecheck errors"` | None |
| B | `Try "fix lint errors"` | None |

只將建議文字換為既有 how-does／create-util／edit 任一已錄製值，
其餘 whitespace-normalized tokens 與既有 Ready fixture 完全相同。
這是離線比對，不代表當次 producer 已認出 Ready，也不能將 main UI 的 model header
視為實際 assistant model／API次數或費用證明；沒有 assistant transcript／usage。

## 執行當時的辨識策略與後續決策

目前 TrustNo、TrustYes、Development 及 Ready 都使用完整版本化 literal。
連續執行已觀察到 create-util、how-does、fix-typecheck、fix-lint 等建議變體；
逐一補完整 literal 仍可能在下次遇到其他建議文字而停止。

提出的範圍只針對 Ready：允許唯一、完整、單列的 `❯ Try "…"` 建議內容可變，
版本、尺寸、canonical workspace、其他完整 header／channel／footer仍須與已錄製畫面相符。
Ready不按鍵；既有live SessionStart、generation及穩定interval仍不變。
信任與development選單不放寬，未知提示、額外UI、錯路徑、錯版本、
重複／不完整／含控制字元或換行的建議仍拒絕。

離線proposal probe以本次兩個真frame示範接受邊界，28個錯版本／路徑／channel／footer／
額外UI／建議形狀／尺寸反例均拒絕；它不是 production code或native gate驗收。
此行為差異須先由使用者確認，再實作、跑core/native矩陣及全新verifier。
目前正式classifier與已執行runner bytes均未變，沒有新增真CLI執行或額外按鍵。

## 清理、CI與證據

native cleanup回兩個instance Gone／owned holders absent；自有home、scratch及personal artifacts
共13精確路徑均absent。live ledger記8PID／5PGID，停機後核均無程序；兩個新trust keys精確刪除。
account settings的lsof／inode／bytes guard與其他parsed values保留只在記憶體檢查，
保存的是root執行者紀錄，沒有複製全域account JSON；不能事後獨立重建歷史guard。
目前精確absence可以重核，point-in-time檢查不是全域settings鎖或未來race保證。
必要私有原始frame／native／trace／log保存於AgEnD-ops，未合併author worktree及歷史pins保留。

c7b756b 的push／PR兩平台四個CI jobs皆通過；較早同代碼macOS 306.841042ms>300ms等
失敗log仍保存，這次Python修正不宣稱修復終端效能問題。

完成全新獨立覆核後提供只讀重驗指令；完整模型通訊尚未通過。
新辨識策略先確認，新真CLI計畫依 [D40](../decisions/d40.md) 另取授權；merge等使用者確認。

2026-10-06 使用者在說明建議文字變體後確認「視為可變」；後續修正依 [D41](../decisions/d41.md)，不改本次 v5 執行結論與歷史 pins。
