# 第 11 施工關 C 段：U17 live 工具

> **TL;DR**
> - Codex CLI 0.159.3 的首次四回合 U17 已通過；[原始範圍與限制](gate-11c-u17-live-validation.md)。未 opt-in 仍在起 backend 前 exit 2。
> - 它跑本機 Codex 的四個模型回合；首次 run 已獨立核實且只開放 0.159.3 獲同意；額外執行仍須另獲核准。
> - 下一步：核 [版本政策](gate-11c-codex-input.md) 新 head；CI 不呼叫真 LLM，額外真回合須另獲核准。

## 工具範圍

[入口](../../crates/agend/examples/codex_u17_live.rs) 與 [情境](../../crates/agend/tests/common/codex_u17_live.rs) 走真 App／ClientSource／client／daemon 子程序／holder／production wrapper／remote Codex。App 事件由工具送入；實體終端外觀另做人工驗收。

四回合使用 gpt-6-luna／low：

1. 人工多行 prompt 記住每次不同的 code word，再輸出 1–300 的數字，提供可觀察 busy。
2. busy 時正式 Send 送 Queue，核自己的 clientId／turn／receipt。
3. 重啟自己的 daemon，核 holder／generation／thread 不變、App 重連唯讀；再按 i 後人工問 code word，核上下文。
4. driver 觀察 idle 後正式 Send 送短回覆，核 turn/start 與 durable receipt。重送既有 UUID 不增造 row。

每則 prompt 都要求不使用工具。執行於 record-sandbox.sh；backend 仍會保存 session／cache，auth、config、rules 與 repo 寫入由該沙箱拒絕。暫存 home 專屬本次 run，停止及清理只限自己的 daemon／holders。等待都有 deadline。

工具記錄實際 codex-cli 版本；本機只查版本為 **0.159.3**，未改算舊 0.158.0 的 smoke。CLI／模型或 clientId 保存行為不符會失敗，不能放行。

本機 0.159.3 的 experimental schema 由 CLI 自行產生，turn/start 支援 clientUserMessageId、queue/add 要求該欄位，userMessage 的 clientId 可為 null。thread/turns/list 預設 itemsView 是 summary；工具明確要求 full，核全部持久化 items 與上下文。這只是協定形狀，不證明 runtime 保存 id 或 U17 通過。原 schema 與摘要／SHA256 已封存於 `g11c-final-review/historical-evidence.tar.gz` 的 `g11c-codex-0.159.3-schema/` 與 `g11c-implementation-logs/codex-schema-0.159.3-inspection.json`。

## 準備與 guard

~~~bash
cd "<你的 AgEnD worktree>"
export CARGO_TARGET_DIR=AgEnD-g11c-target
~/.cargo/bin/cargo build -p agend --bin agend --example codex_u17_live
env -u AGEND_REAL_CODEX "$CARGO_TARGET_DIR/debug/examples/codex_u17_live"
~~~

最後一行應 exit 2 並說明 AGEND_REAL_CODEX=1；這是 guard 通過，沒有跑模型。

## 明確 opt-in 後執行

~~~bash
export AGEND_BIN="$CARGO_TARGET_DIR/debug/agend"
AGEND_REAL_CODEX=1 \
  "<你的 record-sandbox.sh 路徑>" \
  "$CARGO_TARGET_DIR/debug/examples/codex_u17_live"
~~~

原首次 run 工具以 CLI 版本、同 thread／holder、manual busy／idle、兩個自己的 clientId／turn receipt、上下文與 cleanup 判定，成功仍印 `U17 live: passed; version approval is still required`。這是工具保留的歷史輸出；現行一般 daemon 的許可以 [版本政策](gate-11c-codex-input.md) 為準，額外 run 不會擴大或撤銷 0.159.3 的許可，仍須另獲 opt-in 授權。

完整歷史讀取修正後，binary／live example 建置、agend all-targets clippy、fmt、實際 thumb no-std check-deps 與文件 linkcheck 通過；未 opt-in 的 guard 仍 exit 2。原輸出在 g11c-implementation-logs/u17-live-full-history-*.log。這些檢查沒有啟動真 backend 或模型，不是 live 通過證據。先前編譯失敗及修正輸出保留在 u17-live-tool-build*.log。

## 下一步

首次四回合已由獨立 verifier 核實，使用者已同意只開放 0.159.3；[驗收收尾](gate-11c-closeout.md) 已核最終 head 結果，使用者另明確確認 #145 合併。重跑命令只供另獲 opt-in 後使用，不沿用這次四回合授權。
