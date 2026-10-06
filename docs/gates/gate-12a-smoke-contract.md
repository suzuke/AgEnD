# 第 12A：smoke 指令與既有防護一致

> **TL;DR**
> - v6 的 `gh pr merge 0` 與生成的 CLAUDE.md 禁止直接 merge 衝突；改用唯讀 `gh pr merge --help`。
> - 既有 shim 仍拒絕整個 `pr merge` 家族；新增原生 audit 身分核對，不修改 driver、啟動處理或防護政策。
> - 下一步：v7 已授權執行一次，shell 探測失敗；修正版雙 shell 覆核與新固定計畫另核，完整 12A 仍未通過。

## 改動

INITIAL prompt 明說 `--help` 只要求說明，沒有 PR 編號或 merge 動作；模型經 PATH 執行，記錄既有 shim 拒絕後繼續 peer send。不得用絕對路徑繞過、改命令或重送。

通過條件同時要求：五個 PATH shim、exit 1、`agend-shim: refused` stderr、原生 shim audit 的 `gh_merge`、A 的 instance／cwd／argv，以及真模型 Bash hook 包含指定 `gh pr merge --help`。沒有原生 audit、其他拒絕原因、錯誤身分、duplicate／bypass 都失敗。gh audit 的 argv 依既有政策只留 `pr merge`，不新增 payload logging。

互傳、Stop queue、Interrupt、ACK-before-work、production startup、相同 session 與七則 exact body 的驗收不變。Bash sleep 與訊息預算仍是原五次 harness send＋兩次 model peer send，900 秒、零自動重跑。唯讀 help 被 shim 拒絕可以驗到相同 guard 分支；是否模型實際願意呼叫仍需新真 smoke，不以零模型測試代替。

使用者 2026-10-06 指示保留 `~/.claude.json` trust entries。新計畫與 cleanup 明列 `trust_entries_retained`／`shared_account_writes=0`；不清共用 account、不要求其他 Claude session 退出。仍移除本次自有 daemon、holders、home、session／scratch 暫存；必要私有證據保留。

## 零模型驗證

```bash
python3 -B scripts/verify_smoke_contract.py --agend <固定 agend binary>
```

這個 verifier 在自有暫存目錄，用生成的 INITIAL shell 呼叫真 agend `gh` shim，peer send 由本地 sentinel 接收，不啟動 daemon／Claude、不投遞訊息。核真 gh 不執行、完整 peer prompt 經 shell quoting 後不變、七段工作命令可被 Bash 解析；錯誤 code／event／instance／cwd／argv、重複／缺少 native audit、exit 與 PATH 反例均拒絕。結束刪自有 fixture。

[v7](gate-12a-observed-smoke-v7.md) 於固定 `1af2a31` 執行一次後 FAILED：實際工具不接受 Bash 專用 `type -P`，不能據此宣稱 PATH 繞過。修正版改用 `/usr/bin/which`，Bash／zsh 各跑原生契約及十個反例，七段 command 各檢語法；INITIAL 明說整段一次 foreground call。原 guard 觀察先保存再斷言，既有五個路徑及原生 audit 身分／次數門檻不變。Ubuntu CI 補 zsh，Rust runtime 不改；新真模型計畫仍須另取授權。

本批只改 scripts／文件；使用的歷史 native binaries 釘 SHA，與本批 Rust source tree 比對，不能宣稱重新編譯，也不能認證真模型行為。全新覆核與 CI 完成後填入紀錄。

## 進度紀錄

- 2026-10-06：開工前確認 #153 author／fresh verifier worktree、branch、target 及已結束自有 runtime 暫存已清理，必要 pins／raw evidence 與使用者要求的 trust entries 保留。新 branch `test/g12a-smoke-contract` 從 merge `0288824` 開始；未啟動真 CLI／模型。

## 下一步

準備完整 argv／prompt／SHA／七則訊息與 900 秒上限的新計畫，獨立覆核後依 [D40](../decisions/d40.md) 另取執行授權。#153 的 v6 原始失敗與有限初始 idle／ACK 證據不改；本批不宣稱完整 12A 完成。
