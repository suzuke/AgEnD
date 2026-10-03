# 第 11 施工關 C 段：實機紀錄與自動驗收收尾

> **TL;DR**
> - 使用者已在 macOS 實機操作完整終端、resize、多視窗、鍵鼠／貼上、歷史與 alternate screen；截圖及自行比對結果分開記錄。
> - 使用者於 2026-10-03 要求後續驗證改由自動化完成，並清理實作／驗證殘留；剩餘行為沿用真 producer、native PTY 及外層 PTY 的驗收矩陣。
> - 下一步：#145 已於 2026-10-03 經使用者確認合併（`b2152db`）；第 12 施工關 A 段提案另等逐項決定。

## 最終驗收與合併

使用者於 2026-10-03 明確回覆「確認合併145」。[PR #145](https://github.com/suzuke/AgEnD/pull/145) 已合併進 `v2`，merge commit `b2152db952c4713a5720e4f71cd0f669edd0a90e`；兩個 parent 分別是原 `v2` 的 `139fea5590f5286ef8fc6f20d1b7c7aa31876425` 及已驗證 head `cfee0276f40ded709b96002c5dcdcc7d6d49c745`，合併 tree `b8c3ce63bec2cc19fb8bba601d1a2b175da08fc8` 與該 head 相同。

- 全新、無相關 context 的 r5 在 c834 程式版本跑 accept tui 605 passed、其餘 workspace 295 passed，以及三個 demo；提示重複、EOF 不釋放與意外 refusal 的反例被拒絕，延後真拒絕的正向通過。
- c834 後僅 Markdown 變更。最終全新 r9 獨立核 417 個非 Markdown Git entries 相同、目前政策／文件／歷史證據一致；445 個 links／anchors、fmt、實際 no-std 通過，CONFIRMED。
- 最終 head 的 [PR CI](https://github.com/suzuke/AgEnD/actions/runs/37106002634) 與 [push CI](https://github.com/suzuke/AgEnD/actions/runs/37105998997) 各兩個 Ubuntu／macOS jobs 成功；原始 logs 各 900 passed／0 failed／2 既有 ignored，實際 no-std 通過。這是合併前 checks，不冒充合併後新 CI 結果。
- 使用者實機與後續 native／外層 PTY 行為驗收完成，範圍及環境限制仍依下表；零額外真模型回合，第四個原 U17 回合仍只核 user item／receipt。
- 已移除 13 個完成 worktrees、13 個專用外部編譯目錄、WT target 與散落 fixtures；原始反證、實機／U17 證據及最終報告已集中封存，本機位置由交付的驗證報告列示。合併後已確認實作 worktree 乾淨，另以非 force 移除並刪除本機 feature branch；清理報告保留。

## 實機已驗範圍

實機紀錄當時使用 `tui_full` 測試執行檔；原編譯目錄已清理。這個 demo 使用真 holder parser、FakeDaemon 及正式 ClientSource，不啟動真 backend／LLM；PARSER SIZE 是 parser 尺寸，kernel 尺寸另由 native suites 的 stty 證明。終端 app 名稱與鍵盤配置未提供，不把這份紀錄當所有 macOS／Linux 終端的認證。

| 行為 | 實際結果 | 證據形式 |
|---|---|---|
| 畫面／模式 | 繁中、寬字、combining é、綠色 heading；i 後只留一列狀態，Ctrl-] 回唯讀 | 截圖 |
| 唯讀按鍵 | y 畫面不變 | 使用者自行比對 |
| 輸入／resize | x 實收 [120]；rows／columns 跟視窗改變 | 截圖 |
| 歷史 | F4 到 history-1199／HISTORY END；滾回 1008–1033 後三次 z 不拉回底，回底有三筆 [122]；v [118] 直接跟隨 | 截圖＋自行比對 |
| mouse | 文字區 press／release 實收 SGR；status 點擊不送；普通滾輪 code64 到 producer | 截圖＋自行比對 |
| Shift 分流 | mouse on 時回看 1170–1195；回底仍只有原來兩筆普通滾輪回報 | 截圖 |
| alt／normal | ALTERNATE SCREEN 不帶 normal 歷史；Shift 回捲不出現 normal 歷史；F5 還原原內容 | 截圖＋自行比對 |
| 多行 paste | 第一行繁中、第二行 é、第三行，單筆 bracketed packet 包含 0x1D；維持輸入中 | 截圖 |
| 超限 paste | 1,048,577 bytes 顯示「整段未送出」，無新 GOT／bytes；之後 k [107] 可送 | 截圖＋自行比對 |
| 第二視窗 | 同 socket 接回原內容；第二 i 後第一唯讀，a 不送、第二 b [98] 可送 | 截圖＋自行比對 |
| 多視窗 resize／重取 | 第一唯讀 resize 不改第二 27×102；第一明確 i 後取得控制，最新 26×102，第二唯讀 | 截圖 |

原始圖片與逐步 JSON 已逐檔核 hash，封存於 `historical-evidence.tar.gz` 的 `g11c-manual-r1/`；每份紀錄保留當時的驗證範圍。 早期文件中的同名證據可在封存包內查到；公開頁只列證據檔名，本機位置由交付報告列示。首次簡單 paste 複製到 Markdown 符號，未算繁中通過；後續實際 UTF-8 packet 才通過。曾報告的不明重啟不算 daemon recovery 證據。

## 人工發現的修正

多視窗交接的底部提示出現兩次，原因是同一控制狀態同時作為 label 與暫存 message。renderer 只附加與 label 不同的 message，保留超限／其他錯誤提示。既有真 producer 多視窗回歸在交接後、按鍵清除 message 前核提示只有一次；原版失敗 count=2，修正版通過。

第一個嘗試把斷言放在按鍵之後，因按鍵會清 message 而未重現；原 pass log 保留，不能當反例成功。沒有改協定、控制權或 Codex 許可。

## 後續自動化對照

| 要證明的行為 | 正式 suite |
|---|---|
| 真 input／kernel resize、多視窗、拒絕、paste／alt | `agend --test tui_native_app` |
| crossterm capture、外層 resize、正常／unwind 還原、Shift／20 次 fd 清理、最後 dirty | `agend --test tui_outer_pty` |
| agent caller forbidden、舊 token／EOF、背景 FIFO、surviving holder 重連 | `agend --test terminal_hub`、`full_terminal_contract` |
| view／owner／generation 失效、bounded replies、reader／writer 回收 | `agend-tui --test full_source`、`agend-client --test full_terminal` |
| 未驗 Codex 拒絕、已驗版本正常 daemon、永久歸屬與 resume 反例 | `agend --test codex_u17` |

固定 09a205d 的全新 verifier r3、完整 workspace 覆蓋及四個 Ubuntu／macOS CI jobs 各核 900 passed／2 既有 ignored，實際 no-std 通過。本批提示修正後的原始 checks 與最終固定 head verifier／CI 見 [PR #145](https://github.com/suzuke/AgEnD/pull/145)。

真 U17 的四個已核准模型回合與 [live 證據](gate-11c-u17-live-validation.md) 保留；本批不增加真模型回合。fake suites 不代替這份 live 證據。

## 清理與確認

驗證完成後移除本批的 fixture、測試程序、編譯目錄與完成的 verifier worktrees；最終報告與必要原證據保留；#145 合併後的實作 worktree 及本機 branch 已移除。清理前核所有權、worktree dirty 狀態與實際程序，不用名稱相似就刪除其他任務。

最終 head 的自動檢查、獨立反證、CI 與清理已由 PR 報告核實；使用者已明確確認 #145 合併。合併後的文件收尾 PR 仍須另經驗證與使用者確認，不沿用實作 PR 的 merge 授權。

## 下一步

C 段已完成並合併；回 [ROADMAP](../ROADMAP.md) 檢視第 12 施工關待決定的提案。
