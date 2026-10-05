# 第 12A：Claude Driver、啟動設定與控制

> **TL;DR**
> - 這批接續已合併的 client #147、store #148、bridge #149 與 gh 防護 #150；目前施工中，native 實作提交為檢查點，尚未合併。
> - 已接入訊息保存、事件讀取、設定檔 ownership 與單鍵中斷；以下結果只證明列出的回歸案例。
> - 下一步：完成啟動提示、真 CLI 驗收與最後清理，再交 fresh verifier。

## 狀態

依 [D40](../decisions/d40.md) 實作，完整第 12A 尚未完成。工作分支是 `feat/gate-12a-claude-driver`；worktree 位於 `/Users/suzuke/AlphaCR-worktrees/AgEnD-g12a-claude-driver`。本頁不把局部回歸當成完整 adapters 驗收。

| 範圍 | 已實作 | 尚待完成 |
|---|---|---|
| Driver | Claude push 的訊息 claim、重複 id 檢查、持久化事件游標；agent send 與 pipeline dispatch 經 backend router | 真 Claude 的收件與工作回報驗收 |
| 啟動設定 | D40 旗標、六種 native hooks、channel MCP、ACK 說明；三個設定檔以 SHA-256 記 ownership，拒絕覆寫外來或已變更內容 | 選定真 CLI 版本驗收 |
| 持久化 | 未發布的 0008 保存設定檔 ownership；schema fixture／snapshot／retention 同步；ACK 事件使用獨立 id，避免與 hook id 撞號 | 最後獨立驗證 |
| 忙碌中斷 | holder 1.2 的單鍵控制；Steer／Interrupt 發單一 Esc，再走 channel；人工 owner 拒絕時保留訊息 | 選定真 CLI 的 Esc／channel 行為 |
| 人工終結 | 結果不明顯示 `claude-delivery:<message-id>`；`abandon` 限人操作，保存理由並改 failed；晚到有效 ACK 與放棄在 DB thread 核對 | fresh verifier 與人工驗收 |
| 忙閒 | hooks 的 busy／idle 候選、五秒穩定與 live screen gate | SessionStart 必須與啟動完成共同判定；版本化提示 fixtures 與 Down／Enter |
| 清掃 | Claude 清掃接既有 supervisor 時機；pgid／精確 session 或 channel instance 身分檢查；本批 native fixtures 隨測試結束清理 | 最後殘留檢查 |

設定檔 ownership 是 0008 的施工內容，不能由本頁宣稱已發布 schema v8。`delivery = inbox` 維持原路徑，不建立 Claude push 設定。

## 已執行的回歸

以下 native 回歸於 2026-10-05 在本 worktree 執行，不使用真 Claude。另獲授權的真版本查詢與兩寬零輸入蒐證已完成，見[蒐證範圍](gate-12a-startup-capture.md)；沒有新增模型回合。

| 命令 | 結果與邊界 |
|---|---|
| `cargo test -p agend-daemon --lib driver::claude` | 修正後 17 cases 通過：8 個設定檔、6 個 Driver／SQLite、3 個清掃案例 |
| `cargo test -p agend --test claude_process` | 3 cases 通過：在線／下次開機清掃、存活 holder 保護、設定衝突的三次重試及 failed |
| `cargo test -p agend --test claude_bridge` | 全檔 27 個測試入口通過：26 個回歸及 1 個子程序入口；共用 DRV-1–9 另有十個案例，沒有將子程序入口算成獨立證據 |
| `cargo xtask demo adapters` | 通過；共用上述 17 個 Driver 單元、3 個 native process 與 27 個 bridge 測試入口；不執行真 CLI／模型 |
| `cargo test -p agend --test pipeline` | CI fixture 前提修正後 15 cases 通過；等待同一 fleet view 同時具備 approve 階段與 approval attention |
| `cargo test -p xtask` | 通過；含 additive protocol 與 workflow wire format 回歸 |
| `cargo test --workspace` | 檢查點 `afe5188` 全部測試結果通過：992 個測試函式、0 failed、2 個既有 deep explorer ignored；包含子程序入口，不等於 992 個獨立行為案例 |
| `cargo test -p agend-daemon` | options terminator 修正後 exit 0；整個改動 crate 重跑通過 |
| `cargo xtask accept core` | 通過：core、protocol 相容性與實際 no-std；原本兩項 deep explorer ignored 不算通過 |
| `cargo fmt --all -- --check` | 通過 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通過 |
| `cargo xtask check-deps` | 通過；實際編譯無 std target，沒有跳過 |

Driver 回歸先重現「instance failed 就自動放棄未送訊息」的失敗，再移除自動放棄。instance 失敗後，未送內容及結果不明的投遞仍保留；只有有效 ACK 或人的明確放棄能終結訊息。另驗證事件種類及 instance 的 SQL 篩選在 limit 前執行，避免其他 hook 前綴餓死稍後的完成事件。

閒置 Driver 會等待 helper 的持久化寫出回條，最多五秒；只有實際 Written／ACK transaction 改變狀態才回 sent／confirmed，逾時仍 queued。這個等待不授權寫出或重送；舊開機的 idle 紀錄只能造成有界等待。實際 Driver 經 agent send 與 native channel 寫出／ACK、四個 daemon 程序及新 HOME 對照已通過；共用 DRV-1–9 的十個案例已接入，透過 test-only RPC 呼叫獨立程序中的實際 ClaudeDriver；RPC 不實作送達或改寫結果。DRV-6／9 各有 seed 程序及四次獨立開機，共五個不同 pid，停機期間的 native Stop 經 spool 補回；每次更換 HOME 的反向在 boot 2 失敗。

結果不明的 attention 在 helper 十秒期限後顯示，這段等待只避免將仍在執行的寫出提早列為異常，不允許重送。查詢在 limit 前篩選並循環分頁；pipeline 的刷新與 task 終結不抹掉它。`abandon` 明確終結投遞，不確認收件，也不完成 task；ACK／Written 已先入庫時拒絕過時的放棄。真 Claude 尚未執行，安裝路徑指向 2.1.284，與歷史錄製 2.1.282 不同。

pipeline 的 `dispatch:<ticket>`／reviewer 派工 id 保留原值，ACK 接受非空的原訊息 id，delivery／session 仍限 UUID v4 並核對完整歸屬。通用 PipelineStore 回條只觀察 Claude Driver/helper 已保存的狀態，不代寫 sent／confirmed；task CAS 不代替 ACK。完整 Git 流水線經 native channel、Rust CLI done／review approve、人核准與四個 daemon；每個角色只一個派工、head 綁定正確、只 merge 一次。dev ACK 刻意延到 task 完成後，先保持 sent，晚到有效 ACK 仍可 confirmed。

遺失 Esc 回覆案例讓真 holder／PTY 收到一次 Esc，再丟掉其原生 completion；四次開機保持同一投遞識別碼與結果不明，沒有寫 content、沒有第二次 Esc。測試代理會還原 socket，holder／helper／home 都由 fixture 清理。

全新無相關 context 的 checkpoint verifier 對 `afe5188` 重跑 native demo／實際 no-std 通過，但找出 instance args 中的 `--` 能把 daemon-owned 旗標移到 options terminator 後方；原 head 判有缺陷。作者先重現回歸 exit 101，再拒絕 `--`，驗證 fresh／resume 均拒絕、拒絕時不建立三個設定檔；inbox 保持原路徑。修正後 17 個 Driver 單元、整個 daemon crate、workspace clippy 與實際 no-std 通過。修正版 `441d658` 的第二位全新 verifier 針對 terminator 修正判定 CONFIRMED，另以真 workspace 矩陣與 native supervisor 核拒絕不產生設定檔；作者單元的路徑斷言已改成真正 workspace。這份局部結果不認證完整 12A。必要反例與最新成功 log 保存於 `/Users/suzuke/Documents/Hack/AgEnD-ops/g12a-native-checkpoint-20261005/`；`cb0212c` 的全新 fixture verifier 已局部 CONFIRMED，固定 `50e0e83` 的 push／PR CI 均已在 Ubuntu、macOS 通過。

CI 在 `441d658` 暴露兩個測試前提空窗：macOS 的 DRV-4 在 actor 等待 5.3 秒後送達，daemon 尚未保存 idle，正確回 queued；Ubuntu 的 pipeline 看到 approve 階段後，approval attention 尚未發布。DRV fixture 現在等實際 Driver 的最新 BusyChanged idle 事件；刻意提早 readiness hint 的回歸通過，暫時忽略真 idle 紀錄的反向仍以 queued 失敗（exit 101）。pipeline fixture 在原 60 秒期限內等同一 fleet view 具備階段與 attention；15 cases 通過。產品逾時、Sent／Confirmed 斷言與核准行為未改。最新 native demo／clippy／fmt／實際 no-std 通過，固定 `cb0212c` 的 fresh verifier 已局部 CONFIRMED，固定 `50e0e83` 的 push／PR CI 均通過，原 CI 失敗 log 保留。

上述檢查尚未取代 [完整驗收計畫](gate-12a-validation.md)。Draft [#151](https://github.com/suzuke/AgEnD/pull/151) 仍在施工，完整 12A 完成後須再交全新 verifier。

## 開發重驗

```bash
cd /Users/suzuke/AlphaCR-worktrees/AgEnD-g12a-claude-driver
export CARGO_TARGET_DIR=/private/tmp/agend-g12a-driver-target
~/.cargo/bin/cargo test -p agend-daemon --lib driver::claude
~/.cargo/bin/cargo test -p agend --test claude_bridge
~/.cargo/bin/cargo xtask demo adapters
```

這是局部開發重驗指令；完整人工驗收指令會在實作及 fresh verifier 完成後提供。真 CLI 的版本、完整命令與回合預算仍須另外取得授權。

## 下一步

補完表中的未完成工作，執行完整 adapters demo、文件與依賴檢查，交 fresh verifier 驗證；push／draft PR／CI 依既有授權進行，merge 等使用者驗收及明確確認。驗證結束移除本批暫存與程序，待 merge 的實作 worktree 保留供驗收。

- 2026-10-05：`441d658` 的 options terminator 修正獲第二位全新 verifier 局部 CONFIRMED；CI 揭露 DRV idle 與 pipeline approval attention 的 fixture 前提空窗，補真實狀態 barrier 與提早 hint 反向。17 Driver／3 process／27 bridge 測試入口、15 pipeline cases、clippy／fmt／實際 no-std 通過；fixture 修正版獨立驗證及新版 CI 待核，完整 12A 未完成（#151）。

- 2026-10-05：`cb0212c` 全新無相關 context verifier 判局部 CONFIRMED：native demo／15 pipeline cases／clippy／fmt／實際 no-std 通過；忽略 idle 的反向 cargo 101。注入一次消費端 FleetView attention 空窗時，新 helper 等下一個真 view 後 single merge，原 helper 以 missing attention 失敗；這是消費端故障注入，不宣稱控制 daemon 原生發布順序。已還原 mutations、清理該 verifier 的 worktree／branch／target／程序／fixtures；固定 head 雙平台 CI 尚待完成，完整 12A 未認證（#151）。

- 2026-10-05：`50e0e83` push／PR CI 均在 Ubuntu、macOS 通過；新增[被動蒐證工具](gate-12a-startup-capture.md)供後續真啟動畫面核對，native 替身驗工具，不宣稱 P5／P6 或真 CLI 通過（#151）。

- 2026-10-05：蒐證工具 `4511e21` 的 fresh verifier 找到跨 soft-wrap 的 synthetic Bearer 前綴會寫入且回成功；修正採 native cell.wrap／leading_spacer 還原 logical line，再做遮蔽與 scan。原電郵反例是拒絕、未寫出；另補 Bearer 拒絕回歸，修正 head 獨立驗證待核。這些只驗工具，不是真 Claude fixture。

- 2026-10-05：`356fbef` 的全新無相關 context verifier 局部 CONFIRMED 被動蒐證工具修正；原同一 native Bearer wrap 反例改為寫入前拒絕，5 cases、四種尺寸／寬字 spacer、零 stdin、成功／失敗清理、clippy／fmt／實際 no-std 通過。`4511e21` 的原 REFUTED 證據保留；只認證工具，真 CLI／P5／P6 未完成（#151）。

- 2026-10-05：使用者另行授權後，固定真 Claude 2.1.284 查版本與 100／140 欄被動蒐證均成功；0 模型回合、0 按鍵、0 訊息。完整 trust frame 匯入版本化 fixture，既有 hard gate 分類回歸待核；未接受預設 No 選項，development channels／P5 按鍵／P6 初始 idle 與完整 12A 仍未完成（#151）。

- 2026-10-05：準備額外 opt-in 的受控 trust 蒐證工具，最多兩次 operator Input，native producer 正反例通過；只為取得後續真提示，不認證正式 P5 daemon-key／revision CAS 或 P6。真按鍵尚未授權；完整 crate／fresh verifier 待核（#151）。

- 2026-10-05：受控 trust 工具 `b0d8074` 的全新 verifier 重現路徑前綴／畫面別處提及自有路徑會誤送 Down、Enter，原 REFUTED 證據保留。改成唯一 workspace 標頭下完整路徑相等並補兩寬原生拒絕回歸；修正版獨立核對待完成，未執行真按鍵，完整 12A 未完成（#151）。
