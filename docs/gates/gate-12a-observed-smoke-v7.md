# 第 12A：observed v7 的 shell 探測失敗

> **TL;DR**
> - 固定 `1af2a31`／v7 經使用者授權，在四個 CI checks 通過後執行一次；完整 smoke **FAILED**。
> - 兩個初始 idle、六個啟動鍵、第一則 channel ACK 與 A→B peer ACK 有原生證據；往返、Stop queue、Interrupt 未完成。
> - 下一步：修正 Bash 專用路徑探測並做 Bash／zsh 零模型覆核；新固定計畫另取授權，不自動重跑。

## 執行與失敗

Claude 2.1.284、兩個 Haiku 4.5 low、100×24；計畫 SHA `cb3132812d7780eec05267404a909e68e20c87a9aae544907571598177ea83f7`。七則工作訊息／900 秒／零重跑是上限，實際只送一則 harness INITIAL 與一則模型 A→B peer，兩則均經 channel confirmed；不是兩次 API 呼叫的宣稱。

模型確實執行唯讀 `gh pr merge --help`，沒有 merge 動作。INITIAL 的 `type -P` 是 Bash 專用選項；原生 PostToolUse 回報五次 `(eval):type:1: bad option: -P`，模型 Read 回報空路徑檔、拒絕文字與 exit 1。runner 在核路徑時停止，錯誤標籤為 `Bash PATH bypasses a shim`；**這個標籤不能證明實際 PATH 繞過**。零模型 zsh 執行舊生成命令亦被同一斷言拒絕。

原 gh audit／workspace 檔案沒有獨立封存，不能核完整 gh 身分與次數。模型將 INITIAL 拆成探測／gh 與 peer send 兩個 Bash calls，也不滿足完整 audit 對同一個 INITIAL command 的要求；完整驗收不因有限通訊成功改判通過。

## CI 與清理

原 push／PR macOS 在既有 TUI 測試超過 300 ms（305.969667／302.591792 ms），失敗保留。同一提交只重跑失敗 jobs 一次後，四個 checks 全通過；本機原案例 24 次亦通過，門檻未改，沒有宣稱修復既有時序問題。

runner 已停止自有 daemon，native cleanup 回報兩個 holders Gone／owned holders absent，刪除 home 與精確 session／scratch 暫存。使用者要求的兩個 trust entries 刻意保留，cleanup 未寫共用 account。plan／authorization／commands／四份 frames／native events／session 原始證據私有保留，全新無相關 context verifier 已確認有限失敗範圍與目前精確 absence，五個證據反例均拒絕。記錄的五個 PIDs 與兩個 agent PGIDs 已不存在；歷史 daemon PGID 未保存，不能補稱當時已證明其 absence。

## 修正範圍

- 外部 PATH 用 `/usr/bin/which`，避免 shell builtin 選項與 `kill` builtin；五個 shim 路徑、exit 1 與原生 gh 身分／次數門檻仍相同。
- INITIAL 明說整段 command 在一次 foreground Bash tool call 執行；其餘六則訊息、傳送順序、ACK／route／session 條件不變。
- 路徑、gh output、exit 與原生 audit 的原始觀察先私有保存，再做斷言；失敗清理不再丟失這些檔案內容。
- 零模型契約在 Bash／zsh 都執行；Ubuntu CI 安裝 zsh。沒有修改 Rust runtime／防護政策，也沒有新增真模型執行。

## 下一步

完成獨立覆核與新 head CI，產生新的固定計畫，依 [D40](../decisions/d40.md) 另取真模型執行授權。完整第 12A 仍未驗收。
