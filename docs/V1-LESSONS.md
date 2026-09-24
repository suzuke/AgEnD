# v1 教訓 → v2 結構性答案

> **TL;DR**
> - v1 = agend-terminal（HEAD `6586472a`），約 18.3 萬行 production 程式碼；近 60 天 67.6% 的 commit 是 fix。
> - 記住：**v1 是研究過的參考資料，不是約束**；v2 用結構消除問題，不再用事後 invariant 測試補洞。
> - 下一步：改某個子系統前，先在下表找到它對應的 v1 問題。

來源：history.md、inventory.md、usage.md、injection.md、workflow-validation.md、規劃 r4。數字的時間窗見各來源（使用量約 10 天：2026-09-14～24；daemon log 約 2 天：2026-09-22～23）。

## 問題類別對照

| # | v1 問題 | 證據 | v2 答案 |
|---|---|---|---|
| 1 | 訊息注入不穩 | 5.5 個月、至少 8 種失敗模式；三套去重並存；busy 時靜默丟失到 2026-09-18 才修（`1c20f32e`）；2 天 61 次假「送達失敗」WARN | 結構化 API 唯一路徑；訊息 id 單一冪等；PTY 只送單一控制鍵；排隊成功不回報失敗 |
| 2 | 推送截斷 → agent 狂查 inbox | 超過 200 字截斷或只推標頭（`src/inbox/notify.rs:228-256`）；inbox 10 天 18,937 次 | 推送一律帶完整內容；inbox 只作補查 |
| 3 | 狀態誤判 | 約兩天 75 萬次 idle↔active；usage_limit 10 天 1,589 次轉換；每次 CLI 改版就要重新校準（`c02de096`、`2fb635be`） | 結構化事件判 busy／idle；螢幕只認 hard gate 且附 fixture；去抖動 |
| 4 | 刮螢幕回應啟動提示 | `dismiss.rs` 44 KB + `dev_modal.rs` 59 KB、27 個 commit，2026-09-14 仍在修 | 四層：避免 → 結構化 → 規則資料 → 未知提示請人處理 |
| 5 | worktree／branch 混亂 | 137 個 branch 只有 2 個被 git 認出已合併；62 個沒 upstream；49 個 worktree 目錄；約 9 個清理機制；「有 WIP 跳過 GC」兩天 301 次；worktree 子系統 fix 比例 59.6%、約 18,161 行 | 只有 daemon 建立；先記錄再建立；固定命名空間；事件觸發清理；WIP 存 patch；單一對帳 |
| 6 | 從 git 推論 done | 2 天 2,973 次 ancestry compare 失敗（squash merge 讓推論失效） | daemon 自己 merge，done 以 merge 記錄為準 |
| 7 | daemon 重啟 agent 就死 | `shutdown_sequence` 關機殺掉所有 agent（`src/daemon/mod.rs:1609-1660`）；hot-restart 等前任退出才 spawn | 每個 instance 一個 holder 持有 PTY 與附屬程序 |
| 8 | 附屬程序是 daemon 的子程序 | codex app-server（`src/transport/codex_app_server.rs:286-316`）、opencode serve（`src/transport/opencode_server.rs:452-500`） | 由 holder 啟動並持有 |
| 9 | daemon 停機時 hook 事件被丟棄 | `src/main.rs:1243-1245`；漏掉 Stop 就一直被當 busy | 磁碟佇列補送，再用螢幕確認一次 |
| 10 | daemon 內阻塞 I/O | 2 天 861 次「scanner-thread slip」 | DB 專屬執行緒；外部指令一律 `tokio::process` + timeout |
| 11 | 協定沒有版本協商 | `src/framing.rs` 只有一個版本位元組 | holder 與 client 協定都有版本、向後相容 |
| 12 | 從 cwd 推 task | agent 的 cwd 是 `workspace/<instance>`，不是 worktree | 呼叫者身分 → DB binding |
| 13 | shim 放行已綁定 agent 的 `update-ref` | `classify.rs:752-759` | protected-ref 檢查 |
| 14 | 設定錯了不報錯 | #2207、#2005、#3402、#3499、#1351／#2204 | `agend doctor`／`agend init` 在第一次使用前指出並附修正指令 |
| 15 | 治理機制膨脹、型別事件沒人用 | 約 18 個治理機制；8,347 個 task 中 `Verified`、`Linked`、`TaskCloseProposed`、`OperatorSettled` 使用 0 次；核准靠 `ResultSet` 自由文字 | 記帳由 daemon 自動完成；v1 治理機制不帶入 v2.0 |
| 16 | 架構保證靠 grep 測試補 | 48 個 invariant／guard／audit 測試，至少 15 個是同一根因換 issue 再犯 | crate 邊界 + `cargo xtask check-deps`；型別與所有權 |
| 17 | 假綠測試 | #1483：matcher 比對 production 從不送出的格式；#1493 規則 | 契約測試跑真假兩種實作；輸入用 producer 產生 |
| 18 | 磁碟堆積 | home 161G；`evidence/` 108G 是 agent 自己寫的；`workspace/` 23G | 定義 workspace 內容與清理規則；給 agent 會被清的暫存目錄；監看 home 大小 |
| 19 | 功能擺盪 | #879 系列三次被 revert、第四次換手法才成功；rate-limit 恢復提示三層 revert | 範圍以規劃 §3.1 為準，新增須附使用證據 |
| 20 | 沒人用的功能 | tray 無執行期使用證據；Discord 未設定；kiro 從未建立 instance | v2.0 不包含（規劃 §3.2） |

## 保留下來的 v1 好零件

- portable-pty + VTerm「重連時送乾淨畫面」的做法（runtime-spike）。
- codex 用 `turn/steer` 插入的做法（injection.md Q4）。
- `block_on` 巢狀 tokio runtime 已用共用 helper + 測試鎖死，近 60 天 0 次新 fix（history.md §2）。

## 細節

- 熱區排行（fix commit）：`src/daemon` 429、`src/mcp` 343；`agent`／`tasks`／`inbox` 的 fix 比例都超過 60%（history.md §1）。
- 近 60 天：408 個非 dependabot commit，fix 67.6%、feat 9.8%（history.md §4）。
- daemon log 2 天 WARN 17,133 筆，前 5 名約 84% 與 GitHub CI／ci_watch 有關（usage.md §4）。
- 真實 task 形態：46.6% 有 parent，最多一個 parent 77 個子 task；12.4% 有 `depends_on`；2.1% superseded（workflow-validation §2）。

## 下一步

```bash
cat docs/ROADMAP.md
```
