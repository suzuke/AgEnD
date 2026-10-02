# 第 11 施工關 C 段提案：完整 agent 終端

> **TL;DR**
> - 建議讓 `i` 進入完整 agent 畫面，只留一行 AgEnD 狀態列；色彩、游標、resize、滑鼠與貼上都經既有 client／daemon／holder 路徑。
> - 使用者於 2026-10-02 逐項確認 P1–P6，記為 [D39](../decisions/d39.md)；[PR #144](https://github.com/suzuke/AgEnD/pull/144) 只含提案，尚未實作。
> - 下一步：提案通過最新 verifier／CI 後依使用者授權合併，再開實作 worktree；實作另經驗證與人工驗收後才 merge。

## 現況與依據

基準 `v2`：`4bab1e7572e4838d28d1e6267f39f9326096233f`（Gate 10 #143）。[B 段 P5／P6](gate-11-tui.md#p5t-的終端怎麼即時更新) 只傳純文字畫面、固定 PTY 大小、200 ms 重拿、按鍵輸入；Codex 輸入回 `not_supported`。C 段目標已於 2026-09-29 另列，以下 P1–P6 已由使用者確認；現況表描述提案基準，不代表 C 段已實作。

| 可核對來源 | 現況 |
|---|---|
| `crates/agend-holder/src/screen.rs` | alacritty 維護畫面與 1,000 列記憶體歷史；`text()` 丟掉色彩／游標，歷史不傳出 |
| `crates/agend-core/src/protocol/holder.rs` | holder 1.0，已有 `Resize` 與 `OperatorTerminalInput` |
| `crates/agend-core/src/protocol/client.rs` | client 1.3；沒有 resize 或完整終端 frame 請求 |
| `crates/agend-tui/src/lib.rs`、`terminal.rs` | 互動迴圈只處理 Key；文字列從左邊畫、跟著最後非空白列 |
| `crates/agend-daemon/src/handlers.rs` | agent caller 打字被拒；Codex 的操作者輸入仍被拒 |
| [Codex S2](../backends/codex.md#結論表)、[Gate 7 P1](gate-07-codex.md#p1daemon-怎麼跟-codex-講話) | spike 已驗 resume 後可觀察手動 turn；完整 AgEnD PTY 輸入／messages／忙碌策略的 U17 尚未認證 |

## P1：完整模式的入口與退出

- 問題：直接把首頁慣用鍵送進 agent 會誤觸；完整畫面又不能保留 B 段的大框線與說明列。
- 建議：`t` 仍先開唯讀終端；`i` 明確進入完整模式，內容區佔視窗寬度與高度減一列，最後一列只顯示 instance、連線／輸入狀態與 `Ctrl-] back`。新尺寸與完整 frame 確認後才轉為可輸入。
- `Ctrl-]`／既有 `Ctrl-5`／0x1D 只在本機退出；其餘可編碼的鍵（含 Esc、q、Ctrl-C）送給 agent。退出回唯讀終端；停機、斷線或失去控制權立即停送輸入，重連後須再按 `i`。
- 理由：沿用已確認的明確輸入入口，完整模式裡 agent 能收到它自己的快捷鍵。
- 替代方案：`t` 直接開可輸入畫面，少一次按鍵但改掉 B 段的唯讀預設。
- 例子：首頁按 `q` 離開 AgEnD；進完整模式後按 `q` 是 agent 輸入，按 Ctrl-] 才回 AgEnD。
- [x] 使用者確認 P1（2026-10-02，照建議）

## P2：畫面由 holder 提供，協定採加法

- 問題：純文字不能還原色彩與游標；重連後從任意 PTY byte 開始解析，也缺少之前的終端模式。
- 建議：沿用 holder 的 alacritty parser，新增結構化終端 frame：可視格子的文字／寬格佔位／組合字、前景／背景／字型屬性、游標位置與形狀／可見性、normal／alternate screen、輸入模式及 viewport／歷史範圍。TUI 經 `Source`／`agend-client` 畫 ratatui cells；不直接連 holder。
- 每份 frame 帶 holder generation 與單調 revision，畫面、mode、歷史 metadata 在同一次 holder lock 下取出。holder process generation 活過 daemon 重連，與 runtime link generation 分開；holder 重起後才換 frame generation；丟棄舊 generation／舊 revision。viewport 回覆另帶 request id，舊查詢不蓋新選取範圍；歷史讀取屬每個 client，不改 holder 的 live grid／螢幕分類視窗。先提供完整 viewport frames，不先做 cell delta。
- 提案版本：client 1.4、holder 1.1；新增訂閱 frame／查歷史 viewport 與操作者控制請求。只有兩段協商都足夠才開完整模式；否則保留 B 段純文字唯讀並明示需要升級。一般 client 連線的 NEEDED 維持 1.3，新 API 各自檢查協商 1.4；既有 1.3／1.0 格式與讀取路徑不變。
- 輸出變更只設 dirty；每個 instance 至多每 50 ms 取一份最新 frame，最後一段 dirty 必須送出。新 frame 序列化行上限 8 MiB，viewport 一次只傳所需列；無效尺寸或過大 frame 明確拒絕，不能截斷後當成功。保留既有 holder 1 MiB 請求行上限。
- 理由：holder 是唯一終端狀態來源，重連不靠 byte replay；合併畫面更新可丟中間 frame，但不能丟最後的畫面。
- 替代方案：TUI 自建第二個 parser（要設計完整 parser state checkpoint）；直接輸出原始 ANSI 到操作者終端（畫面與外層狀態列、控制序列互相干擾）。
- [x] 使用者確認 P2（2026-10-02，照建議）

## P3：resize 與多視窗控制

- 問題：兩個視窗反覆 resize 同一 PTY，agent 會跳動，舊視窗也可能依過期 mode 編碼輸入。
- 建議：完整模式建立連線範圍的 attach id；最後進入完整模式的視窗取得控制權，以其內容區尺寸 resize。**較先開的視窗轉唯讀**，保留畫面且明示另一視窗控制中；再次按 `i` 可取得控制權。這比原 C 段「最後開啟的尺寸為準」多了輸入控制，使用者已於 2026-10-02 明確確認本項。
- daemon 先驗 caller，再驗活終端、attach id／generation／控制權及尺寸。resize 送既有 holder 長連線；新畫面回來後才確認可輸入。一般終端尺寸變更只由目前控制者送，status row 與零尺寸不能進 PTY。
- Ctrl-]／關視窗／終端連線 EOF 釋放控制，保留最後 PTY 大小；不自動恢復舊視窗控制或預設 50×200。daemon／holder 重連後 attach id 作廢，須重新取得。
- 控制／resize 的 I/O 在背景處理，主畫面不等最長 5 秒的 holder write；同一連線有 request id 對應回覆，權限拒絕或失效請求不影響 PTY。每個 instance 的控制與 PTY 寫入按序處理，寫前核對 token；新控制者的成功回覆須等舊在途操作結束或作廢，才允許新輸入。失去控制後的舊輸入一律拒絕、不重送。完整模式有控制者時，沒有 attach id 的舊版 terminal_input 亦拒絕，避免繞過控制權；沒有控制者時保留 B 段原行為。
- 理由：尺寸與輸入有同一個控制者，避免兩個視窗送互相矛盾的操作。
- 替代方案：多視窗都能輸入、最後開啟者只管尺寸；保留原描述，但兩份不同大小的畫面可同時操作。
- [x] 使用者確認 P3（2026-10-02，照建議）

## P4：按鍵、貼上與滑鼠滾動

- 問題：「原樣轉送」會受終端回報方式限制；滑鼠滾輪可能是 agent 的事件，也可能是看歷史。
- 建議：轉送 crossterm 能觀察到的按鍵語義，依 holder mode 編碼 application cursor／keypad、UTF-8、Ctrl／Alt；無法區分的鍵不聲稱 byte-identical。滑鼠座標換算成內容區的座標；底部狀態列與區外事件不送，不把內容列的 y 減一。
- 外層開 mouse capture／bracketed paste，退出、錯誤與 unwind 時恢復。agent 開 mouse tracking 時，按其 mode 編碼支援的 xterm mouse 事件；未開時滾輪看本機 viewport 的 holder 歷史。Shift+滾輪固定看歷史，不送 agent；若終端不回報 Shift，以唯讀模式滾動作替代。
- normal screen 歷史沿用 holder 1,000 列；以絕對列 id 定位，捲上去後新輸出不跳回底，超出已淘汰範圍明示並夾到最舊列。回到底部才跟隨；alternate screen 無獨立捏造歷史。
- 貼上維持一個 Paste 事件；agent 開 bracketed paste 時加 `ESC[200~`／`ESC[201~`，否則送文字。沿用既有輸入行大小檢查；過大整次拒絕、不拆成可能半次成功的請求。貼上內容中的退出控制碼當資料，不作本機退出鍵。
- 理由：依 agent 已啟用的模式送事件，捲動不會誤變成按鍵或誤把貼上當逐鍵導航。
- 替代方案：所有滾輪都轉成 Up／Down（容易打斷輸入），或所有滾輪都本機捲動（agent 滑鼠功能失效）。
- 協定依據：[XTerm Control Sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Mouse-Tracking) 的 mouse modes 與 [bracketed paste](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Bracketed-Paste-Mode)；本項的 UI 分流為 AgEnD 提案。
- [x] 使用者確認 P4（2026-10-02，照建議）

## P5：Codex 輸入的開放條件

- 問題：B 段已確認暫回 `not_supported`；spike S2 不能代替完整 AgEnD 路徑的 U17。
- 建議：先補 fake Codex／真 daemon 的 PTY＋app-server 整合回歸，再以明確 opt-in 的真 Codex smoke 驗同 thread、人打字產生的 turn 被 driver 看見、messages 不增造 daemon receipt、忙碌時派工仍走既有策略、daemon 重啟後上下文與 holder 不變。記錄實際 CLI 版本，不把舊 0.156.1／0.158.0 證據改算新版本通過。
- smoke 通過、使用者確認後才解除已驗版本範圍的 Codex 輸入拒絕；未通過或未驗版本仍回 `not_supported` 並說明原因。CI 只跑 fake／真 AgEnD 程序，不呼叫真 LLM、不花 token。
- 理由：只放開已驗的手動輸入路徑；訊息送達仍走結構化 driver，沒有把 agent 送訊息改成 PTY 打字。
- 替代方案：C 段一開始對所有 Codex 版本解除限制，會把未驗的 U17 當已成立。
- [x] 使用者確認 P5（2026-10-02，照建議）

## P6：完成範圍與實作順序

1. core additive 型別／協商 → holder frame／歷史／resize → runtime／daemon／client／fake 契約；core 不加 std 或依賴。
2. TUI 完整模式／控制／renderer → mouse／paste／resize／恢復 → fake Codex 整合與 opt-in live smoke。
3. 更新各 crate README／TESTING、名詞表、Gate 11 與 ROADMAP；跑適用測試、fmt、workspace clippy、實際 no-std、完整 `accept tui`、雙平台 CI；全新無 context verifier 後逐步人工驗收。

文字終端的色彩、游標、寬字、alternate screen、一般 xterm 按鍵／mouse／paste 是完成範圍。圖形協定、外層 clipboard OSC、任意 terminal extension 與分割視窗另列支援邊界，不把「完整模式」宣稱為所有終端功能。

詳見 [C 段驗收計畫](gate-11c-validation-plan.md)。本 PR 只有提案，沒有新 runtime demo，亦未執行上述未新增的測試。

- [x] 使用者確認 P6（2026-10-02，照建議）

## 使用者確認紀錄

2026-10-02 依使用者要求逐項說明、每項等回覆才繼續；六項均採建議。P1 另提供靜態示意，未把預覽算成 runtime 驗收。

| 項目 | 使用者回覆 |
|---|---|
| P1 入口／退出 | `sounds good` |
| P2 holder 畫面／相容 | `好` |
| P3 多視窗控制 | `按你的建議` |
| P4 mouse／paste／歷史 | `ok` |
| P5 Codex 驗證門檻 | `ok` |
| P6 範圍／順序 | `OK` |

使用者另明確指示「#144合併」。該授權只用於此提案 PR；C 段 runtime／Codex U17 尚未驗收，實作 PR 的 merge 仍須另行確認。

## 下一步

最新文件經全新 verifier 與 CI 通過後合併 #144；再開 `feat/gate-11c-terminal` 與新 worktree 實作。實作 merge 仍等使用者明確確認。
