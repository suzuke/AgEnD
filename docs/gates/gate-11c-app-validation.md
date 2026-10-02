# 第 11 施工關 C 段：App 局部驗證

> **TL;DR**
> - App 已接結構化 Source：完整模式、尺寸確認、多視窗失效、鍵鼠／貼上與固定歷史；C 段仍在 [draft PR #145](https://github.com/suzuke/AgEnD/pull/145) 實作。
> - 此頁只記錄本批證據，不能代替完整 C 段、Codex U17、fresh-context verifier 或人工驗收。
> - 下一步：完成其餘 renderer／壓力矩陣與 U17，再做完整驗收。

## 本批行為與證據

| 行為 | 實際驗證 |
|---|---|
| `i` 明確取得控制 | 真 parser＋socket；取得控制被 gate 擋住時不送 q／x，途中改大小後只在最新尺寸完整 ack 才送 UTF-8 |
| 退出與交接 | 三種 Ctrl-] 回報不進 PTY；另一視窗取得控制後舊視窗唯讀，再按 i 才恢復 |
| renderer | 真 parser 的 RGB／indexed／bold／italic／underline、CJK＋combining、寬字裁切、alternate screen、cursor shape |
| 唯讀跟隨／固定 | 完整 live grid 不 resize PTY；裁切至最後輸出／cursor；可固定 grid 內的列、捲上後新輸出不跳回底 |
| 歷史淘汰 | 超過 1,000 列後 clamp 並提示；保留實際絕對 row id；回到底部恢復跟隨 |
| keys | 真 parser modes：application cursor／keypad、CRLF；q／Esc／Ctrl-C／L／UTF-8／Alt／modified cursor 送到 consumer，release 不送 |
| paste | 一個 Paste event；bracketed 包裹／plain、繁中多行及 0x1D 當資料；raw／base64 與 exact JSON envelope 超限均整次拒絕，控制保留 |
| mouse | 真 modes：Click／Drag／Motion、SGR press／release／wheel、legacy binary 與 UTF-8 coordinates；status／outside／不支援 motion 不送 |
| 本機滾動 | 無 tracking、唯讀或 Shift wheel 看 holder viewport；alt 不捏造歷史；固定 viewport 上不把歷史座標送 agent |
| 停止／重連 | 已控制 instance failed、daemon EOF／重啟後唯讀；舊輸入不補送，明確 i 才重新控制；focus 依 mode |
| 外層恢復 | output writer 注入 setup error／unwind，檢查 mouse／paste／focus／cursor reset；每個 reset 獨立嘗試 |
| 舊／拒絕能力 | 1.3 App 提示升級且不送輸入；Acquire not_supported 保持 live 唯讀，不自動重試控制 |

## 本批檢查

- TUI 79 passed（其中 `full_app` 15 cases）；holder 56 passed，含新增的 live-grid 小 viewport 定位與上界 clamp。
- 真 daemon TUI 主流程／Codex 拒絕共 2 cases 通過；fake `tui_accept`／native `tui_real` demo 通過，原 echo fixture 期待不符的失敗保留。
- 完整 workspace 861 passed／2 個既有 ignored（退出重訂失敗修正前）；修正後 TUI 79 passed，最新 workspace clippy、fmt 與實際 thumb no-std check-deps 通過，198 個文件 links／anchors 有效。這批尚未跑完整 accept tui 或 U17。

## 原失敗

- 真 parser＋socket 重現 Ctrl-] 的新訂閱協商失敗時，舊 expanded mode 仍殘留（`failed exit resubscribe kept the old full mode`，exit 101）。修正為立即作廢 owner／expanded mode，保留 ended 狀態並限速重試；重連不自動控制，明確 i 才可送新輸入。

- `f999bbf` 的 Ubuntu push／PR CI 通過，兩個 macOS jobs 的 Source overflow fixture 只送出 53／51 次，未到 64-reply 邊界就開始 drain，沒有觀察到 overflow。改為逐次等真 consumer 收件並等 worker 關閉，原 assertion 保留，本機同一 case 通過；新 CI 另核。
- App cases 從 11 增至 13 時在 macOS 256 fd 下耗盡 descriptor；限制同時存活 fixture 為 3，保留全部 cases，再跑通過。Source 的 20 次 fd baseline 檢查沒有放寬。
- 編譯／clippy、重連 fixture TempDir 提前清掉 socket parent，以及唯讀小視窗捲動的初次失敗均保留；修正後重跑。

原 logs 在 `/private/tmp/g11c-implementation-logs`。TestBackend 及 writer 證據尚未認證外層終端字型／外觀或完整 native TUI 資源清理。

## 尚待完成

- 本批後已補原生五種底線與標準游標讀回；詳見 [原生驗證](gate-11c-native-validation.md)。實機外觀待驗，HollowBlock 使用 block fallback。
- 真 PTY input consumer 的完整 App 已補[原生回歸](gate-11c-native-app-validation.md)；外層 event capture／restore 已補 [原生證據](gate-11c-outer-validation.md)；其餘拒絕／壓力／時效矩陣與實機外觀待完成。
- fake Codex＋真 AgEnD U17、明確 opt-in live smoke、CLI 版本及使用者確認後才開放已驗版本；目前 Codex 仍拒絕輸入。
- 完整 accept tui、雙平台最新 CI、全新無 context verifier、逐步人工驗收與 merge 確認。

## 下一步

照 [驗收矩陣](gate-11c-validation-plan.md) 完成剩餘證據；完整 C 段 ready 後才派全新 verifier。
