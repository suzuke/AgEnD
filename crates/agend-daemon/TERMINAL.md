# 完整終端入口（第 11 施工關 C 段）

> **TL;DR**
> - 真 daemon 的 client 1.4 路徑已接通；`TerminalHub` 綁定 socket 的 view／attach，最後 Acquire 的視窗控制 PTY。
> - 控制與畫面 I/O 在背景，不阻塞 socket reader；EOF／失效不恢復舊 owner，重連後要重新訂閱與 Acquire。
> - 下一步：`cargo test -p agend --test terminal_hub --test terminal_runtime`；TUI／fake 契約與 Codex U17 仍待完成。

## 控制與更新

- 每 instance 一個 actor、操作 queue 上限 64；每 client 的完成回覆 queue 上限 8，畫面只保留最新一份。慢 client 斷線並釋放控制，沒有無界 task／回覆佇列。
- 先驗 caller，再驗活終端與本連線的 view／generation／owner／尺寸。Acquire 由 daemon 產生新 attach，其他操作須帶目前 owner 的 attach；foreign view／舊 token 不影響 PTY。
- 新 grant 等舊在途操作結束；實際 resize 加完整 frame 才可輸入，實際 write／flush 完成才 input ack。沒有 attach 的舊版輸入同樣依序排隊，有 C owner 時明示拒絕；沒有 owner 保留 B 行為。
- EOF 同步標記 scope 失效；actor 等在途 write 結束再 Release，保留最後尺寸，不恢復舊控制者。若 actor 被停止／銷毀時仍持有 completed owner，作廢該 holder epoch，讓 holder 釋放控制；agent 與 PTY 保留。
- runtime 的 dirty watch 合併 PTY chunk；holder 的同一 parser grid／palette／mode 共用 50 ms 取樣，各 view 只讀所需列。resize ack 是當下的新完整畫面；歷史 viewport 和 request id 分屬各 view。
- client 新 frame 行包含 envelope／換行最多 8 MiB，超限整份拒絕再關該 client；新請求最多 1 MiB。舊版 9 MiB 純文字 client 回歸保持。

## Runtime 長連線

`HolderRuntime::terminal_connection` 提供 holder 1.1 能力與連線 epoch；背景 writer 依 request id 配對回覆，queue／pending 各最多 64。holder 請求最多 1 MiB；回覆最多 8 MiB（含換行）。舊 holder 明示需升級。

斷線、agent 結束、取消在途控制或 15 秒未回覆作廢憑證；取消控制或逾時關閉該 holder 連線，避免晚到 grant 留下控制權。唯讀查詢取消保留連線；操作不跟隨新連線或重送。重新取得憑證保留同一 holder 的畫面 generation。

client 提供 1.4／1.3，真 daemon 選 1.4；fake 尚未接完整 C 路徑，暫留 1.3。1.3 連線的新請求明示 not_supported 並保留 request id；控制請求先拒 agent caller。Codex Acquire／輸入在 U17 認證前保持 not_supported。

## 驗證範圍

`agend/tests/terminal_hub.rs` 用真 daemon／holder／PTY 驗八項；`terminal_runtime` 驗 pinned 連線、取消與不重送。actor 回收另有 rejected subscription 的 20-instance unit case。這些證據尚未涵蓋 TUI 外觀／鍵鼠、fake 完整 C 契約或 Codex U17；完整矩陣在 [C 段驗收計畫](../../docs/gates/gate-11c-validation-plan.md)。
