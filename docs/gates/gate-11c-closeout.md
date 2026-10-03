# 第 11 施工關 C 段：實機紀錄與自動驗收收尾

> **TL;DR**
> - 使用者已在 macOS 實機操作完整終端、resize、多視窗、鍵鼠／貼上、歷史與 alternate screen；截圖及自行比對結果分開記錄。
> - 使用者於 2026-10-03 要求後續驗證改由自動化完成，並清理實作／驗證殘留；剩餘行為沿用真 producer、native PTY 及外層 PTY 的驗收矩陣。
> - 下一步：核本批固定 head、全新無 context verifier 與雙平台 CI；實作 PR #145 的 merge 仍等使用者確認。

## 實機已驗範圍

測試執行檔為 `/private/tmp/AgEnD-g11c-target/debug/examples/tui_full`。這個 demo 使用真 holder parser、FakeDaemon 及正式 ClientSource，不啟動真 backend／LLM；PARSER SIZE 是 parser 尺寸，kernel 尺寸另由 native suites 的 stty 證明。終端 app 名稱與鍵盤配置未提供，不把這份紀錄當所有 macOS／Linux 終端的認證。

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

原始圖片與逐步 JSON 在 `/private/tmp/g11c-manual-r1`；每份紀錄保留當時的驗證範圍。首次簡單 paste 複製到 Markdown 符號，未算繁中通過；後續實際 UTF-8 packet 才通過。曾報告的不明重啟不算 daemon recovery 證據。

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

驗證完成後移除本批的 fixture、測試程序、編譯目錄與完成的 verifier worktrees；最終報告／必要原證據及待 merge 的實作 worktree保留。清理前核所有權、worktree dirty 狀態與實際程序，不用名稱相似就刪除其他任務。

最新 head 的自動檢查、獨立反證、CI 與清理結果由 PR 報告核實；本頁不把待執行檢查預算成通過。第 11 施工關完成與 merge 仍需使用者確認。

## 下一步

閱讀 PR 的固定 head 驗證與清理結果，使用者確認後再合併。
