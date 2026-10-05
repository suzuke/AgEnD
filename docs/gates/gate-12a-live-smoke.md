# 第 12A：完整真模型通訊 smoke

> **TL;DR**
> - 使用者 2026-10-06 要求完整模型 smoke 必做；#151／#152 已合併；首次真執行停於初始 idle 逾時，訊息階段未開始。
> - 固定 Claude 2.1.284，兩個 Haiku 4.5 instance、七則工作訊息；首個失敗停止，不重跑。
> - 下一步：保留失敗證據，修 audit／清理及原始畫面蒐證；新真 CLI 計畫依 D40 另取授權。

## 範圍與證據

| 情境 | 通過條件 |
|---|---|
| 正式啟動 | 兩個新 session 使用 production P5／P6；無人工鍵或 manual mode；已知提示外出現 attention 就停止 |
| 閒置互傳 | 測試驅動者以 B 身分 → A；A 模型自己 send B；B 模型自己 send A；三則均 channel、完整內容與原身分 |
| 明確 ACK | 每則原生 AgendAck tuple 與訊息／delivery／session 一致；ACK 在相應 Bash 工作前 |
| 忙碌 queue | A 前景 sleep 45 秒中加入訊息；訊息保留到後續 Stop，route=stop，再由模型 ACK 與工作 |
| Interrupt | A 前景 sleep 120 秒中發 Interrupt；由正式單鍵路徑中斷並 channel 送達，90 秒內完成新工作，舊命令未完成 |
| Bash PATH | 真模型的 Bash 確認 git／kill／pkill／killall／gh 五個外部命令在自有 bin；gh pr merge 0 被 shim 拒絕 |
| 版本 | 固定 executable SHA、另查 --version，真 CLI 自有 transcript header 同為 2.1.284 |
| 清理 | 停自己的 daemon／holders、以既有精確 session／pgid sweep 清掉孤兒；保留必要私有證據，刪自有暫存與 session 檔，精確清掉本次 trust 條目 |

這是完整真模型**通訊 smoke**；崩潰、四次開機、部分 ACK、人工控制、檔案 ownership 與 pipeline 的故障矩陣沿用已驗的 native 證據，不逐項消耗真模型重跑，也不因此宣稱第 12 施工關 B／C／D 完成。

## 預算與命令

`scripts/claude_live_smoke.py --plan` 只讀 bytes／產生計畫，不執行 Claude。
`send` 是 agent 命令，操作者不能直接呼叫；五次種子訊息由 harness 明確使用 B 身分，兩次 peer send 必須另有真模型 Bash 的原生 hook 證據，不能把 harness 呼叫算成模型互傳。
JSON 計畫包含 version、完整 backend／operator／cleanup argv、每則 prompt、路徑與 SHA-256。
執行須額外 `AGEND_REAL_CLAUDE_LIVE=1` 及核准計畫的 SHA；runner／三個 binaries 改變就拒絕。

預算：五次 harness send（以 B 的 agent caller 身分）＋兩次模型 peer send＝七則工作訊息，工作期限 900 秒，另留停止與清理時間。
這是工作訊息數，**不是 API 呼叫次數或費用硬上限**；ACK／Bash 工具可能產生額外模型續行。
模型以完整 ID `claude-haiku-4-5-20251001`／low 啟動，事後再核 transcript 的實際 model 並保存 usage，拒絕 fallback；完整 ID 固定方式見[官方模型設定](https://code.claude.com/docs/en/model-config)。runner 不自動 retry，監測到 daemon 內建 backend restart 即停止並拒絕通過；沒有主動 daemon restart、追加 prompt、手動 terminal input 或版本替換。
模型額外送訊息、未 ACK、未知提示、路徑／版本差異或逾時都記失敗，不修補成成功。

```bash
export CARGO_TARGET_DIR=/private/tmp/agend-g12a-live-target
~/.cargo/bin/cargo build -p agend --bins
~/.cargo/bin/cargo build -p agend-daemon --example claude_live_cleanup
python3 -B scripts/claude_live_smoke.py \
  --plan <全新 plan.json> --out <全新私有證據目錄> \
  --agend "$CARGO_TARGET_DIR/debug/agend" \
  --cleanup "$CARGO_TARGET_DIR/debug/examples/claude_live_cleanup"
```

取得該固定計畫授權後才執行：

```bash
AGEND_REAL_CLAUDE_LIVE=1 python3 -B scripts/claude_live_smoke.py \
  --execute <核准 plan.json> --approved-plan-sha256 <核准 SHA-256>
```

SQLite 在 daemon 停機後才讀取；禁止用 immutable bypass 或複製變動中的 WAL。
腳本只刪唯一 session／workspace 暫存；全域個人 trust JSON 由執行者在核無同時寫入程序後精確清理，保留其他 parsed values，不以整份舊檔覆蓋。
`cleanup.json` 的 `trust_cleanup_pending` 要在最後清理證據中核為不存在，不能由 smoke PASS 宣稱清理完成。
失敗紀錄同樣要保留，未知身分／仍有程序／symlink 的路徑保留並報告；不得強制刪除。

## 失敗後的只讀診斷

`scripts/claude_startup_diagnostic.py --plan` 固定一個 A instance、90 秒、零工作訊息、四份只讀 native frame；它使用 production 啟動處理，仍可能寫三個已知 startup keys。每份 frame 最多等 5 秒，不取得控制權、不 resize、不送人工鍵；未知提示或失敗即停止。首次 frame 在 status 檢查之前留存，避免 attention 造成證據遺失。成功只記 `CAPTURED`，不宣稱模型通訊 smoke 或 12A 通過；真 CLI 版本查詢及啟動仍須取得這份新計畫授權。

```bash
~/.cargo/bin/cargo build -p agend-client --example startup_frame
python3 -B scripts/claude_startup_diagnostic.py \
  --plan <全新 diagnosis-plan.json> --out <全新私有證據目錄> \
  --agend "$CARGO_TARGET_DIR/debug/agend" \
  --cleanup "$CARGO_TARGET_DIR/debug/examples/claude_live_cleanup" \
  --snapshot "$CARGO_TARGET_DIR/debug/examples/startup_frame"
```

runner 補核 Ready 的正常 `halted=1` 及三鍵 written；清理按 native SessionStart 的 session／cwd／scratch 路徑，檢查 uid／型別／連結後保留私有證據並刪整個自有 nonce namespace，含 bootstrap UUID。foreign／symlink／hardlink 不刪。global trust keys 仍由執行者精確清理，不能由 runner 宣稱已完成。

## 目前紀錄

固定 `3ac1b5d` 的計畫 `plan-v5.json` 經全新 verifier 核原生清理與授權防護，四個 CI jobs 通過；先前 foreign workspace、漏 holder、HOME、symlink／hardlink 的反例與修正重驗保留。

取得使用者授權後執行一次，結果 **FAILED：`timed out: both idle; no retry`**。一次版本查詢為 2.1.284，兩個 instance 各一個 live SessionStart、三個已知 production startup key states=written；695 次 status 未見兩者同時 idle，七則工作訊息全部未開始，零測試 send／訊息／ACK。沒有自動重跑。

另一位全新 verifier 覆核失敗證據，推翻原 cleanup 完整性：兩個自有 scratchpad namespace 曾殘留，補清後獨立核 49 個精確路徑、兩個 trust keys、記錄 PID／PGID absent。亦發現正常 Ready 設 `halted=1`，原 audit 卻要求為 0；本次尚未走到該 audit 判斷。

原始 terminal frames／transcripts／usage 沒有留存，不能認定失敗畫面或精確模型／API 次數。授權 metadata 是執行開始後 22,520ms 寫檔，使用者授權先於執行是 root 對話順序聲明，不把寫檔時間當授權事件時間。原證據與限制保留。


## 下一步

補只讀原始 frame 蒐證及 audit／scratch 清理的 native 回歸，再由全新 verifier 核固定診斷計畫；新真 CLI 需另取授權，禁止自動重跑成成功。新改動的 merge 仍等使用者確認。
