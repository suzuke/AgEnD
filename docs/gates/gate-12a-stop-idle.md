# 第 12A：Stop 續行結束後恢復 idle

> **TL;DR**
> - 固定 `ab5a296` 的真模型 v10 完成五則 confirmed，但 queue 工作後一直 working，完整 smoke FAILED。
> - active Stop 不再取 queue，仍須建立五秒 idle 候選；未知 payload、歷史 hook 與 live screen gate 的限制保留。
> - 下一步：原生回歸、全新覆核後執行新的有限真測；Interrupt 尚未通過。

## 原始失敗

2026-10-07，Claude 2.1.284／兩個 Haiku 4.5 low，固定 v10 計畫
`7d8eaa282b30f05c747c5cd4202ac3a3ebb821b3309ab75bd7020e1b3549f445`
於四個固定 head CI checks 通過後執行一次。五則訊息分別為 INITIAL、PEER、RETURN、BUSY、QUEUED；前四走 channel，最後走 Stop，全部有明確 ACK，且 ACK 先於所屬 Bash 工作。

BUSY 的 foreground Bash 持續約 45 秒；其間建立的 QUEUED 在下一個 inactive Stop 才預約。queue 工作完成後的最後一個原生事件為 `Stop(stop_hook_active=true)`，A 持續 working、B idle，約 180 秒後因 both idle 逾時停止。BLOCKING／INTERRUPT 沒有發出。

gh audit 的四筆 startup token refusals 與唯一工作 merge refusal 均符合契約。模型 PEER Bash 額外附加 `echo "Message sent"`，列為嚴格指令偏離；未見新增訊息。完整 smoke 沒有通過，也沒有自動重跑。

全新無相關 context verifier 核對 raw transcripts、native events、固定程式與 CI 證據，得到有限 `CONFIRMED_LIMITED_FAILED_OUTCOME`；五個記憶體反例均拒絕。自有十四個精確路徑、五個 PID／兩個 agent PGID 當時皆無殘留。trust entries 保留；沒有完整刪除前 metadata 或外部系統呼叫稽核，不能宣稱歷史上所有 shared-account 寫入為零。

## 修正與驗證

原本 active Stop 分支清除 idle 並設 busy，將「不可再 block」誤當成「仍在工作」。依 D40 P6，未要求續行的 Stop 應結束工作；修正只建立候選，五秒穩定及 live screen gate 通過後才可由 channel 送下一則訊息。Stop 本身不 ACK 訊息。

`active_stop_finishes_queue_work_then_restores_debounced_channel_delivery` 使用真 daemon／holder／hook／channel，核 Stop batch 後 active Stop 回空決定、下一則至少五秒後走 channel，兩則都只有 Sent，沒有冒充 confirmed。相同案例在原 runtime 以 channel timeout 失敗，修正版通過。

`newer_work_or_unknown_stop_revokes_the_active_stop_idle_candidate` 核新的 UserPromptSubmit、缺少及非布林 `stop_hook_active` 會撤銷候選，不開始投遞。

```bash
cargo test -p agend --test claude_bridge active_stop
```

作者完整 `claude_bridge` 原生測試 39／39 通過；fmt／diff check 通過。獨立覆核、真模型與完整 CI 結果另記。

## 下一步

依使用者 2026-10-07 的第 12 施工關持續授權，固定新 binary／計畫後完成真模型 smoke；每次失敗先查原因。完整 CI 作為合併門檻，相關原生驗證及獨立覆核完成後即可進行受控真測。

## v11：命令邊界偏差，未驗到 Stop idle 修正

固定 `0a86271` 執行一次 v11；兩個 backend 完成啟動，INITIAL 有明確 ACK。模型將命令後的說明也放進 Bash，`agend send` 以 `unexpected argument 'Do' found`／exit 2 拒絕；另有三次 Read。PEER 未送出，queue／Interrupt 未執行，不能據此宣稱 Stop idle 修正已經真模型驗收。發現確定失敗後，對精確自有 runner 發 SIGTERM，走原清理流程；home 與自有 sessions 已移除，trust entries 保留。原始 evidence 與失敗紀錄不改寫。

後續提示把每段命令放入各自命名的 XML 區塊，說明全部在前，結尾不接句點或說明文字。INITIAL／PEER 的巢狀 shell quoting 已在 Bash 和 zsh 實際執行，核對逐 byte 的接收參數；不啟動模型、不傳送真訊息。原 audit、七則工作預算、零自動重跑與清理邊界不變。此修正只改善命令邊界，無法保證模型遵守；下一次真測仍須獨立固定計畫。
