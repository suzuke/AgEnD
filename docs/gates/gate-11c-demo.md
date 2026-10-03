# 第 11 施工關 C 段：互動 demo

> **TL;DR**
> - tui_full 用真 holder parser、FakeDaemon 和正式 ClientSource；不啟動真 agent 或模型。
> - 可操作色彩／繁中／游標、resize、鍵鼠／貼上、長歷史、alt screen、兩視窗與重連。
> - 下一步：可用下方命令操作 demo；已取得實機紀錄與後續自動驗證見 [驗收收尾](gate-11c-closeout.md)，demo 不代替真 daemon／Codex 證據。

## 開啟

~~~bash
cd /Users/suzuke/AlphaCR-worktrees/AgEnD-v2-g11c-terminal
export CARGO_TARGET_DIR=/private/tmp/AgEnD-g11c-target
~/.cargo/bin/cargo run --quiet -p agend-tui --example tui_full
~~~

按 /，輸入 g11-contract，Enter 開唯讀畫面，再按 i。畫面只留一行狀態列；Ctrl-] 返回唯讀，q 離開。

| 操作 | 應看到 |
|---|---|
| 改大小 | PARSER SIZE 顯示內容區尺寸，高度扣一列 |
| 打字／貼上 | INPUT BYTES 顯示實收 bytes；繁中及 bracketed paste 可核 |
| F3 | MOUSE ON／OFF；開啟後點選／滾輪到 consumer，Shift 看歷史 |
| F4 | 1,200 個編號行；捲上固定，回底跟隨 |
| F5 | alternate／normal 切換；alt 不捏造歷史 |
| F2 | 假 daemon 斷線／重連；重連唯讀，須再 i |
| F1 | 顯示 demo socket，可開第二視窗 |

第二視窗連第一個 F1 顯示的完整 socket；路徑換行時先拉寬視窗再按 F1：

~~~bash
~/.cargo/bin/cargo run --quiet -p agend-tui --example tui_full -- \
  --socket "<第一個視窗顯示的絕對 socket>"
~~~

這個模式只連原 demo，不建另一份 fake daemon。兩邊找 g11-contract 再按 i；最後按 i 者控制，原視窗唯讀。F2–F5 的 producer 操作只在第一視窗攔截。

## 已驗範圍

新 demo 的單獨 clippy／build 通過。自己的外層 PTY capture 核開畫面、i／23×80 parser resize、實收 x 與 SGR mouse bytes、正常 exit 0 與原 termios 還原；不是實機字型／外觀驗收。初次 probe 把 fleet 預期成 instance、忽略 ANSI 排版空白及退出時沒 drain PTY，fixture 失敗保留，改正後通過。第二 client 已經兩個真 App 程序核對：第二視窗由 21×90 resize 到 15×60，原視窗唯讀，x 被拒絕、y 實收；第一視窗重新 i 後 z 實收，兩個程序 exit 0 且各自 termios 還原。原 fixture 用 60×16 啟動 dashboard，低於 70×20 最小尺寸而失敗；修正為進完整模式後再縮小，原 log 保留。

INPUT BYTES 是實收 bytes 再經真 Screen 輸出，沒有手寫格子。PARSER SIZE 是 parser 尺寸，不是 kernel stty 證據；真 PTY／多視窗另見 [原生 App](gate-11c-native-app-validation.md)。

[完整 U17 fake demo](../../crates/agend/examples/codex_u17_probe.rs) 已納入 accept tui；真模型另見 [U17 live](gate-11c-u17-live.md)。

## 下一步

依 [驗收計畫](gate-11c-validation-plan.md) 的更新方式自動驗證剩餘行為；核最新固定 head verifier／CI 與清理結果後報告，merge 等使用者確認。
