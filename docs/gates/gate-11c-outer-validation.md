# 第 11 施工關 C 段：真外層 PTY 驗證

> **TL;DR**
> - 真 `agend app` 已在原生外層 PTY 驗 event capture、resize、多視窗、歷史及正常／unwind 還原；C 段仍在 draft PR #145。
> - 四個完整情境加兩個子程序入口，共 6 tests 通過；前批完整 agend 214 passed、0 ignored，本批 clippy／fmt／實際 no-std 通過。
> - 下一步：補其餘拒絕／壓力矩陣與 Codex U17，再做完整獨立及人工驗收。

## 證據路徑

`crates/agend/tests/tui_outer_pty.rs` 用 `portable-pty` 啟動真正 `agend app`。輸入 bytes 由外層 PTY 寫入，經 crossterm event capture、App、ClientSource、真 daemon／holder 到 raw agent；測試不直接呼叫 App events。唯一例外是 panic 子程序：執行同一個 `run_with`，只在攔截 hook 注入 panic，讓 unwind 確實走 native guard。

兩批 App suites 共用 `tests/common/native_app.rs`：只有 holder 的 instance 身分才進入 raw agent；ANSI 由 agent 寫入真 PTY，尺寸由 agent 的 `stty size` 讀 kernel。外層輸出再交給第二個真正 holder parser 讀回，不造理想化 frame。沒有啟動真 backend 或 LLM。

## 四個完整情境

| 情境 | 實際檢查 |
|---|---|
| capture／render／resize／多視窗 | 真 q／Esc／Ctrl-C／UTF-8、application cursor、SGR press／release、focus、bracketed 繁中多行及 0x1D；status／區外／Shift click 不送；RGB／indexed／底線色／CJK／combining／beam 游標經外層 parser 讀回，即使 NO_COLOR=1 |
| 尺寸與交接 | kernel resize 通知由 App capture；agent PTY 11×40 → 8×31 → 15×60 → 4×20；fresh producer marker 加 input 狀態證明相符 frame／ack 已到；舊視窗 paste／resize 不影響新控制者，明確 i 可重取 |
| 正常退出 | Ctrl-] 本機退出輸入，Esc 回導航，q 正常結束；raw agent 沒收到這些本機退出鍵，kernel termios 等於原值 |
| panic unwind | 真 raw／alt／capture 已開啟，physical x 觸發 panic；子程序故意 exit 101，父情境驗原 termios、alt／mouse／paste／focus／cursor／SGR 還原並通過 |
| 滾輪與歷史 | >1,000 行真 producer；無 tracking 滾輪回捲，有 tracking 時 Shift 分流；固定 viewport 不隨新輸出跳，回底再跟；focus 及 FIFO sentinel 證明新 modes 已到，不靠 sleep 當成功 |
| 資源 | 20 個真正 App 程序逐次 open／acquire／quit／wait／join，每次父程序 fd 回同一基準；holder pid 保留，raw agent 無額外輸入，fixture 最後清理 |

正常與 unwind 都以實際輸出核 capture modes 已關、游標可見，並核 1000／1002／1003／1006／2004／1004、cursor shape、SGR／三種 color 的 reset bytes。child exit 後仍持有 slave 讀 termios，之後關自己的 slave 讓 reader EOF，再 join。Drop 只 kill／wait 自己的 child。

## 最後 dirty 的端到端時效

新增情境每次送 100 段 intermediate ANSI，再送唯一 FINAL-DIRTY marker；真 producer 停止輸出後，等外層 parser 的實際可見 cell。12 次逐筆從通知 producer 前開始計時，包含檔案 handoff、flush、daemon／holder、App draw 與外層 parsing；每筆都要求 ≤300 ms，不加 holder round-trip 額度，比計畫的 300 ms 加一趟更嚴。最終 suite 的範圍 95.756–215.302 ms，原數值保留於 `latency-outer-final-suite.log`。

暫改 daemon SAMPLE_EVERY 為 800 ms，同一 native 情境在 806.224 ms 的時效斷言失敗、exit 101；finally 還原產品碼。正向外層 suite 6 passed／0 ignored，workspace clippy／fmt／實際 no-std 通過。本批沒有重算尚未重跑的完整 agend 或 workspace；前批 214 是 `d8b65cc` 的結果。這是正常本機時效證據，不宣稱重載／停頓環境仍可守同一 wall-clock 上限。

## 原失敗與反例

最初以 40×12 開首頁，觸發 dashboard 最小 70×20 規則；改以 80×24 開啟後再縮小完整畫面。原生輸入初跑也指出「標題已出現」不等於 ready，「stty 已改尺寸」不等於 App 已收到相符 ack；改等實際 read-only／input 狀態與 fresh producer marker，保留原失敗。沒有放寬輸入或尺寸斷言。

暫把 guard 的 DisableBracketedPaste 改為 Enable，同一 native unwind 情境在「paste／focus／SGR 已關」斷言失敗，cargo exit 101。production 檔在 finally 還原，兩批 App suites 共 8 tests 再跑通過。macOS termios flags 是 u64、Linux 是 u32；改用 Into<u64> 保留原值，沒有新增 unsafe 或 suppress clippy。

## 檢查與計數

- 本批完整 agend：214 passed／0 ignored；兩批 App suites 8 passed；workspace clippy、fmt 與實際 thumb no-std check-deps 通過，沒有 allow-skip。
- 前 head `50851e2` 的 push／PR Ubuntu、macOS 四個 CI jobs 全數成功；各主 suite 共 867 passed／2 個既有 ignored。本批新 head CI 另核，前 head 結果不能代替新 head。
- CI stdout 有時另含 re-exec 的 filtered `daemon::stop_flag::tests::child_probe` 成功摘要；它已由父測試覆蓋。計數只加 filtered out = 0 的主 suite，不再把子程序輸出重算。前 renderer 的原報告 865 = 864 主 suite + 1 probe 輸出；原 logs 保留，accept tui 572 的計數不受影響。

原 logs／CI metadata：`/private/tmp/g11c-implementation-logs/outer-app-*.log`、`outer-app-baseline-50851e2-ci.json`；本批收尾 snapshot 為 `SHA256SUMS-outer-app`，原失敗與 mutant 也保存；後續時效證據為 `latency-outer-*.log`、`SHA256SUMS-latency`。

## 重跑

```bash
cd /Users/suzuke/AlphaCR-worktrees/AgEnD-v2-g11c-terminal
export CARGO_TARGET_DIR=/private/tmp/AgEnD-g11c-target
~/.cargo/bin/cargo test -p agend --test tui_outer_pty -- --nocapture
```

`portable-pty` 只新增為 agend 的 dev dependency，與 holder 共用已存在的 0.9，Cargo.lock 沒有新增 package。一般子程序入口 tests 直接返回，三個父情境內才執行真正的 agent／panic 子程序。

## 下一步

這是 macOS headless PTY／kernel 證據；不認證實際 Terminal／iTerm2／Linux 字型、外觀或非美式鍵盤，也不把 writer error injection 當所有硬體輸出故障的原生證據。完成 [剩餘矩陣](gate-11c-validation-plan.md)／U17，再核最新雙平台 CI、派全新無 context verifier、帶逐步人工驗收；merge 等使用者確認。
