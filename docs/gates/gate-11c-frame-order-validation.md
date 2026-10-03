# 第 11 施工關 C 段：延遲 frame 與 wire golden

> **TL;DR**
> - 三個 App 順序回歸由真 holder parser／FakeDaemon／ClientSource 產生資料，只延遲或重排既有 events。
> - 三個正向通過；移除對應防護的三個 mutants 各 exit 101。真 PTY frame golden 與既有 frame suite 共 8 passed。
> - 下一步：真 Codex 0.159.3 已有首次 U17 證據，接著核新 head CI、全新 verifier 和人工驗收；尚未完成 C 段。

本頁保存該批次的歷史結果與當時下一步，不能作為目前待辦清單。最新版本許可見 [輸入政策](gate-11c-codex-input.md)；使用者已要求剩餘行為自動驗證，固定 head 結果、實機限制與清理見 [驗收收尾](gate-11c-closeout.md)。

## App consumer 的證據

[測試與排程 seam](../../crates/agend-tui/tests/support/delayed_frames.rs) 由 full_app 載入，沿用同時最多三個 labs 的限制。Source 的連線、控制、輸入與關閉都委派正式 ClientSource；排程 seam 只保留真正收到的 frame 或重新送出其 clone，沒有手寫 cells／generation／revision／request id。

| 回歸 | 刻意隔離的條件 | 核對 |
|---|---|---|
| 舊 revision | 相同 instance／view／generation／request id，只有 revision 與實際畫面較舊 | 延遲 frame 不蓋回目前 frame，輸入控制保留；renderer 仍顯示 NEW-REVISION |
| 舊 viewport query | 同 view／generation／revision，兩次真正查詢的 request id 與 viewport 不同 | 先送第二次，再送第一次；選取與逐 cell 資料保持第二次 |
| 舊 generation／view | 重開後 App request 名稱相同；舊 generation revision 反而比較大 | 舊畫面不取代新資料、不撤銷新 owner；z 仍進真 consumer |

第三項驗實際 holder generation／view 生命週期，不把不同 generation 的 revision 當可比較序號。只跑這三個 cases 的結果為 3 passed／18 filtered，不冒充完整 App suite。query fixture 依真正回覆的 request id 同步，排除仍在途的 live frame；修正同步後完整 full_app 為 21 passed／0 ignored（frame-order-full-app-final.log）。

## 嘗試推翻

在自己的 worktree 逐項暫時移除 App 防護，跑對應回歸；每次以原 bytes 還原，最後 production source 與原檔完全相同。

| mutant | 實際失敗 |
|---|---|
| 不檢查較舊 revision | late real frame rolled the display back；exit 101 |
| 不檢查目前 query id | late query changed the selected history；exit 101 |
| 不檢查目前 view id | old view revoked the current owner；exit 101 |

原 logs／mutant source 保留在 g11c-implementation-logs/frame-order-*-mutant.*；正向原輸出為 frame-order-first.log。這是 App 邊界受控重排，不宣稱 wire 上的 TCP／Unix stream 會自己倒序。

## Frame golden

[真 PTY 測試](../../crates/agend-holder/tests/terminal_frames.rs) 用 bash 印終端序列，再由真正 Screen 取出 frame，透過正式 holder／client response 型別序列化並核完整 round-trip。

[固定 golden](../../crates/agend-holder/tests/golden/terminal-frame-1.4.json) 包含兩套 envelope、格子／樣式／寬字／combining、游標／modes、尺寸與歷史 metadata。只把每次不同的 generation 換成固定 token，其他欄位維持 producer 輸出。新增 fixture 前的空 golden 比對 exit 101 是建立 fixture 的過程，沒有把它當產品 bug 或有效 golden 通過。

frame-golden-suite.log：8 passed／0 ignored，包含新 golden 與原有真 PTY renderer／歷史／舊 holder serializer 案例；沒有新增依賴。bootstrap 原始 producer JSON 留在 frame-golden-bootstrap.log。

## 完整 checks

本批 accept tui exit 0：597 主 suite passed／0 ignored，fake／真 daemon／完整 fake U17 三個 demos 通過。執行在新增 golden 前已完成 holder 階段；golden 後另跑完整 terminal_frames 8 passed，沒有把它加進 597。fmt、workspace clippy／最後 TUI clippy、實際 thumb no-std 及 linkcheck 通過。56dbb71 四個 CI jobs 均成功，新提交 CI 另核。

## 重跑

~~~bash
cd "<你的 AgEnD worktree>"
export CARGO_TARGET_DIR="$PWD/AgEnD-g11c-target"
~/.cargo/bin/cargo test -p agend-tui --test full_app
~/.cargo/bin/cargo test -p agend-holder --test terminal_frames
~/.cargo/bin/cargo xtask accept tui
~~~

## 下一步

完整矩陣對照見 [執行狀態](gate-11c-matrix-status.md)。局部測試與 golden 不代替真 Codex、雙平台新 head、全新 verifier 或人工驗收。
