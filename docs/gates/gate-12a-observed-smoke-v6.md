# 第 12A：固定 observed v6 收到 ACK，工作遭模型拒絕

> **TL;DR**
> - f1e1b70／v6 另行授權執行一次，兩個 initial idle 通過；第一則 channel 訊息明確 ACK。
> - 模型依生成的 CLAUDE.md 拒絕 smoke 要求的 `gh pr merge 0`，第一段工作逾時 FAILED；互傳／queue／Interrupt 未執行。
> - 下一步：#153 已經使用者確認合併，失敗與有限 ACK 證據經獨立覆核；[smoke 指令修正](gate-12a-smoke-contract.md)準備下一份另行授權計畫。

## 固定計畫與實際執行

使用者在人工重驗 Ready 修正後回覆「授權」，紀錄先寫入，再啟動固定計畫。
source `f1e1b70a697a0d24fee73c110c84400676bd3994`；計畫 SHA-256
`afbc5adc4016dc80b29829ee4597de0d5fb018052fdb217454f02af70365a187`。
六個檔案 pins、LANG=C.UTF-8、fresh home／session namespaces 與精確 trust keys 在執行前核對。
固定 Claude 2.1.284，兩個 Haiku 4.5／low，七則工作訊息／900 秒／零自動重跑。

| 證據 | 本次結果 |
|---|---|
| 版本 | 一次 --version 查詢；A 的留存 assistant transcript 使用完整 Haiku model ID |
| 正式啟動 | 兩個 live SessionStart；各三個 production keys written，manual=0、halted=1 |
| 畫面 | 四份初始只讀 frame；第二批 A／B 為不同可變 Try 建議，工作前重核兩個 idle |
| 投遞 | 實際只有第一則 harness 種子訊息；route=channel、state=confirmed，單一 native AgendAck |
| 模型工作 | A 呼叫 ToolSearch 與 agend_ack；無 Bash 工作，沒有模型 peer send |
| 結果 | `timed out: A finished initial work before peer reply; no retry`，exit 1；沒有重跑 |
| 未驗範圍 | A→B→A 模型互傳、busy Stop queue、Interrupt，以及真 Bash PATH／gh shim 拒絕 |

模型說明它遵守生成的 CLAUDE.md「Never merge or approve a pull request directly」，
因此不執行 `gh pr merge 0` 及後續串接工作。
這是模型拒絕，不是 gh shim 已被呼叫並拒絕；不能據此認證 shim 真模型負例。
生成指示見 `crates/agend-daemon/src/driver/claude/launch.rs`，原固定 prompt 見已授權計畫。
原生 gh guard 與 CI 已有獨立證據，本次不改其規則，也不以那些結果代替完整通訊 smoke。

A 的 raw transcript 在停機清理時私有保留；B 沒有 assistant transcript，
不能用啟動 --model 或 UI header 宣稱 B 已有實際模型回應。
留存 usage 是觀察資料，不是精確 API 呼叫數或費用硬上限。

## 清理與覆核

native cleanup 回兩個 instance Gone／owned holders absent；home 與 session／scratch namespace 已清除。
本次兩個 trust entries 的全域清理 guard 觀察到外來 Claude Code 程序，
在任何 account mutation 前拒絕；當時精確 entries 暫留。使用者其後明確指示保留 `~/.claude.json` trust entries，避免影響其他 Claude session。
不終止外來程序，不寫共用 account。兩筆 entries 為刻意保留，其他自有程序與 13 個精確暫存路徑已獨立核 absent。
必要原始 frames／native JSON／trace／daemon log／A transcript 與執行授權私有保存在
`AgEnD-ops/g12a-live-smoke-20261006/observed-smoke-evidence-v6` 及同目錄執行紀錄。

全新無相關 context verifier 已確認固定執行、唯一 ACK、失敗原因與當前暫存 absent；
合併後另核 merge tree 等於審閱版本，刪除 author／verifier worktree 後的只讀重核亦通過。
必要歷史 runner／binary pins 與 raw evidence 保留；原 audit／manifest／已執行計畫不改寫。
完整 12A 尚未驗收。[v5 失敗](gate-12a-observed-smoke-v5.md)及歷史 pins 保留。

## 下一步

使用者已確認 #153 合併為 `0288824`。後續唯讀 help 防護負例與不改 account 的清理規則見[smoke 指令修正](gate-12a-smoke-contract.md)；
新真 CLI／模型執行依 [D40](../decisions/d40.md) 另取授權，原 v6 結果仍 FAILED。
