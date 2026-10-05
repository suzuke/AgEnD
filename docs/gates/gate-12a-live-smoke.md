# 第 12A：完整真模型通訊 smoke

> **TL;DR**
> - 使用者 2026-10-06 要求完整模型 smoke 必做；#151／#152 已合併，真模型尚未執行。
> - 固定 Claude 2.1.284，兩個 Haiku 4.5 instance、七則工作訊息；首個失敗停止，不重跑。
> - 下一步：全新 verifier 核固定腳本與計畫後，依 D40 確認版本／完整命令／預算才執行。

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

## 目前紀錄

真模型尚未執行。native／腳本與固定計畫驗證進行中，沒有新增模型或訊息費用。全新 verifier 在 `1256c99` 以 native producer 推翻清理順序及漏額外 holder，並重現 HOME 漂移與版本查詢失敗留下空 lab；原證據保留。修正為先全面驗身分／namespace、綁固定環境、無 DB 且無 holder 才可清空 lab，重驗待核。`2b6084d` 另被 native holder 反例推翻：同名 lock／socket 符號連結可誤停另一 home；補所有控制路徑型態預核。`fb4801f` 另被硬連結控制檔案的同類反例推翻，追加檔案／socket 必須單一 link 的 ownership 核對；原反例保留，修正版另驗。

## 下一步

核對全新 verifier 的結果及固定計畫，取得真模型預算授權後執行一次；完成再核證據與殘留，提供使用者可重驗命令。新改動的 merge 仍等使用者確認。
