# 第 11 施工關 C 段：U17 live 工具

> **TL;DR**
> - Codex CLI 0.159.3 的首次四回合 U17 已通過；[原始範圍與限制](gate-11c-u17-live-validation.md)。未 opt-in 仍在起 backend 前 exit 2。
> - 它跑本機 Codex 的四個模型回合；成功後仍須使用者確認，才可開放已驗版本。
> - 下一步：獨立核對原證據，再由使用者確認已驗版本開放；CI 不呼叫真 LLM，額外真回合須另獲核准。

## 工具範圍

[入口](../../crates/agend/examples/codex_u17_live.rs) 與 [情境](../../crates/agend/tests/common/codex_u17_live.rs) 走真 App／ClientSource／client／daemon 子程序／holder／production wrapper／remote Codex。App 事件由工具送入；實體終端外觀另做人工驗收。

四回合使用 gpt-6-luna／low：

1. 人工多行 prompt 記住每次不同的 code word，再輸出 1–300 的數字，提供可觀察 busy。
2. busy 時正式 Send 送 Queue，核自己的 clientId／turn／receipt。
3. 重啟自己的 daemon，核 holder／generation／thread 不變、App 重連唯讀；再按 i 後人工問 code word，核上下文。
4. driver 觀察 idle 後正式 Send 送短回覆，核 turn/start 與 durable receipt。重送既有 UUID 不增造 row。

每則 prompt 都要求不使用工具。執行於 record-sandbox.sh；backend 仍會保存 session／cache，auth、config、rules 與 repo 寫入由該沙箱拒絕。暫存 home 是自己的 /tmp/g11live-*，停止及清理只限自己的 daemon／holders。等待都有 deadline。

工具記錄實際 codex-cli 版本；本機只查版本為 **0.159.3**，未改算舊 0.158.0 的 smoke。CLI／模型或 clientId 保存行為不符會失敗，不能放行。

本機 0.159.3 的 experimental schema 由 CLI 自行產生，turn/start 支援 clientUserMessageId、queue/add 要求該欄位，userMessage 的 clientId 可為 null。thread/turns/list 預設 itemsView 是 summary；工具明確要求 full，核全部持久化 items 與上下文。這只是協定形狀，不證明 runtime 保存 id 或 U17 通過。原 schema 在 /private/tmp/g11c-codex-0.159.3-schema，摘要／SHA256 在 /private/tmp/g11c-implementation-logs/codex-schema-0.159.3-inspection.json。

## 準備與 guard

~~~bash
cd /Users/suzuke/AlphaCR-worktrees/AgEnD-v2-g11c-terminal
export CARGO_TARGET_DIR=/private/tmp/AgEnD-g11c-target
~/.cargo/bin/cargo build -p agend --bin agend --example codex_u17_live
env -u AGEND_REAL_CODEX "$CARGO_TARGET_DIR/debug/examples/codex_u17_live"
~~~

最後一行應 exit 2 並說明 AGEND_REAL_CODEX=1；這是 guard 通過，沒有跑模型。

## 明確 opt-in 後執行

~~~bash
export AGEND_BIN="$CARGO_TARGET_DIR/debug/agend"
AGEND_REAL_CODEX=1 \
  /Users/suzuke/Documents/Hack/AgEnD-ops/record-sandbox.sh \
  "$CARGO_TARGET_DIR/debug/examples/codex_u17_live"
~~~

成功必須有 CLI 版本、同 thread／holder、manual busy／idle、兩個自己的 clientId／turn receipt、上下文與 cleanup；最後印 U17 live: passed; version approval is still required。failed 或未執行保持一般 Codex 輸入 not_supported。

完整歷史讀取修正後，binary／live example 建置、agend all-targets clippy、fmt、實際 thumb no-std check-deps 與文件 linkcheck 通過；未 opt-in 的 guard 仍 exit 2。原輸出在 /private/tmp/g11c-implementation-logs/u17-live-full-history-*.log。這些檢查沒有啟動真 backend 或模型，不是 live 通過證據。先前編譯失敗及修正輸出保留在 u17-live-tool-build*.log。

## 下一步

首次四回合已有明確核准及通過紀錄；派全新 verifier 核對，再依 P5 等使用者確認已驗版本開放。重跑命令只供另獲 opt-in 後使用，不沿用這次四回合授權。
