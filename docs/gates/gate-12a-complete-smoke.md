# 第 12A：完整模型 smoke

> **TL;DR**
> - 固定 `0f4b8e0` 的 v12 單次執行回報 PASS：七則 confirmed、六 channel／一 Stop。
> - 真 Claude 2.1.284、Haiku 4.5 low；互傳、忙碌排隊、續行後 idle、中斷及明確 ACK 都完成。
> - 下一步：全新 outcome 覆核與固定 head CI 完成後合併 #154，再進入 12B；第 12 關整體仍未完成。

## 實際範圍

| 項目 | v12 結果 |
|---|---|
| 啟動 | 兩個真 Claude，正式 trust／development channel 流程，100 × 24 |
| 工作 | 五個 harness seeds，加兩次模型執行 `agend send` 的 peer round trip；共七則 |
| 收件 | 七個原生 `AgendAck`，訊息／delivery／session 相符；Bash 工作在 ACK 後 |
| 排隊 | 45 秒前景忙碌期間排隊，經 Stop 取出；active Stop 後恢復 idle，接續後一工作 |
| 中斷 | 120 秒睡眠被中斷，完成 INTERRUPT，未產生 interrupt-end，最後雙方 idle |
| gh 防護 | 五個 shim 路徑正確；原始 audit prefix 不變，四筆 startup token refusals，工作只有一筆 merge help refusal |
| 原生事件 | 2 SessionStart、7 ACK、16 PreToolUse、15 PostToolUse；被中斷工具不以成功完成假填 |
| 模型工具 | 2 ToolSearch、7 ACK、7 Bash；沒有額外 Read 或額外 send |
| 清理 | 自有 home／holder／session 殘留清理成功；依使用者指示保留 trust entries |

計畫 SHA256：`55edce8b89666974db43a415f56b5ca4b1d2bb1f6565563983496262b711e84b`。原生 binary 與 Stop idle 修正相同；其後只改命令區塊與文件，Rust／Cargo／fixtures 未變。六個 artifacts 固定雜湊；verifier 未独立重建三個 native binary。

預算是七則工作、900 秒、零自動重試，不是七次 API call 或金額硬上限。歷史 v6–v11 失敗與限制保留，不以本次 PASS 改寫。完整原生故障矩陣由各批 PR 測試另證；這次真測不代替 crash／offline／錯誤身分測試。清理宣告不是 OS 級共享帳戶寫入稽核。

## 重驗與證據

本次原始 native、session、commands、audit 與 cleanup 保留在私有 `AgEnD-ops/g12a-smoke-contract-20261006/observed-smoke-evidence-v12`，不將模型 transcript 提交 Git。全新 outcome 報告及只讀重驗程式放同層 `v12-success-fresh-verifier`。重核既有證據不會再呼叫模型；不得重用已執行計畫。

```bash
python3 -B /Users/suzuke/Documents/Hack/AgEnD-ops/g12a-smoke-contract-20261006/v12-success-fresh-verifier/reverify.py
```

## 下一步

核對全新 outcome 結論與最新 CI；依第 12 關持續授權交付，清理已合併工作樹後推進 OpenCode。
