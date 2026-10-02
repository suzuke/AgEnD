# 第 11 施工關 C 段：原生 renderer 驗證

> **TL;DR**
> - 原生輸出已保留單／雙／捲曲／點狀／虛線底線、底線色與標準游標；C 段仍在 draft PR #145 實作。
> - 真 holder parser 產生 frame，經真 crossterm backend，再由另一個真 parser 讀回；尚未認證實機字型與外觀。
> - 下一步：完成真 PTY App／資源矩陣與 Codex U17，再做完整獨立及人工驗收。

## 行為

ratatui 的 `UNDERLINED` 無法區分五種底線。`terminal::native_render` 對已繪製的 buffer 補 typed commands，依 protocol cell 與真 buffer diff 判斷重畫；同字不同底線也會更新。它沿用 buffer 的寬字裁切，不印 spacer，不回放 agent ANSI。閒置畫面不重印底線文字；輸出後恢復 SGR 與已裁切的游標座標。

互動 TUI 暫時保留色彩，即使啟動環境設 `NO_COLOR`；guard 退出後恢復 crossterm 原設定。正常退出、setup error 與 unwind 都嘗試恢復 mouse／paste／focus／cursor、顯示游標及重設 foreground／background／underline color；某一項 I/O 失敗不跳過其他 reset。

底線序列依 [kitty styled underlines](https://sw.kovidgoyal.net/kitty/underlines/)；標準游標依 [XTerm DECSCUSR](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html)。Block／Underline／Beam 各有 blinking／steady；visible／裁切決定是否顯示。DECSCUSR 沒有 HollowBlock 編碼，目前使用 block fallback；實際終端對各樣式的呈現能力另待人工核對。

## 本批證據

| 案例 | 實際檢查 |
|---|---|
| 五種底線切換 | 相同 glyph 從 double → curl → dotted → dashed → single → none → curl；讀回可見 cell 的 style／RGB／indexed／underline color |
| CJK／combining／極窄視窗 | 真 parser 的寬字＋combining、spacer 不重印；單欄裁切為帶原樣式的空白，唯讀不 resize producer PTY |
| 游標 | 六種標準 shape／blink、最下方 content／最右欄、隱藏、唯讀裁切後位置；補畫不移動 hardware cursor |
| 畫面切換／閒置 | finder 隱藏終端游標，返回後重新補畫；閒置 bytes 不包含重印的 CJK glyph |
| guard | 原 `NO_COLOR` 設定在正常／error／unwind 後還原；實際 reset bytes 全數核對 |
| holder 單欄 | 同 parser 的 normal live／history、inactive normal＋active alt、mode／generation／revision、新 CJK／combining、放大後新寬字；有五秒 watchdog |

TUI 81 passed（`full_app` 17 cases），holder 57 passed；workspace clippy 與實際 thumb no-std check-deps 通過，沒有 allow-skip。完整 workspace 865 passed／2 個既有 ignored；完整 accept tui 572 passed／0 ignored，fake／真 daemon demos 均成功。fmt 與 72 個本批文件 links／anchors 通過。

## 原失敗與修正

- 原生色彩讀回第一次失敗：crossterm 在 `NO_COLOR=1` 下省略 color。保留原 logs，加入互動期 color guard；spacer 的獨立 SGR 期待也改為只核其佔位，可見 glyph 仍逐項核色彩／style。
- 把原生底線命令暫改為 single，相同回歸 exit 101（double 的 style 247 被讀成 127）；還原後通過。
- 真 parser 含寬字的畫面縮到一欄時，上游 alacritty 0.26 reflow 無法消耗寬字，反覆配置 row；stack sample 已保存，停止自己的測試程序。移除修正後，新 watchdog 回歸 SIGABRT／cargo exit 101；還原後通過。

holder 保持協定允許的實際單欄尺寸。放不下的寬字在 reflow 前改為保留樣式的空白；normal history 同步處理，alt 透過公開 grid swap 保存其內容。單欄的新寬字輸入也顯示空白，避免寫不存在的 spacer。放大後的新寬字正常顯示；已裁掉的 glyph 不宣稱能還原。只增加已在依賴樹的 `unicode-width` 直接依賴，沒有新增 package。

原 logs／stack／負面證據在 `/private/tmp/g11c-implementation-logs`。這些是 backend bytes 與 parser 證據，不等同於使用者實際 Terminal／iTerm2／Linux 終端驗收。

## 下一步

完成 [驗收矩陣](gate-11c-validation-plan.md) 剩餘項目與 U17；完整 C 段 ready 後才派全新 verifier，merge 等使用者確認。
