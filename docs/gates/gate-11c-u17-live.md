# 第 11 施工關 C 段：U17 live 工具

> **TL;DR**
> - codex_u17_live 已編譯；沒有 opt-in 時在起 backend 前 exit 2，目前沒有 live 通過證據。
> - 它跑本機 Codex 的四個模型回合；成功後仍須使用者確認，才可開放已驗版本。
> - 下一步：先確認四回合 opt-in，再在既有寫入沙箱執行；CI 不呼叫真 LLM。

## 工具範圍

[入口](../../crates/agend/examples/codex_u17_live.rs) 與 [情境](../../crates/agend/tests/common/codex_u17_live.rs) 走真 App／ClientSource／client／daemon 子程序／holder／production wrapper／remote Codex。App 事件由工具送入；實體終端外觀另做人工驗收。

四回合使用 gpt-6-luna／low：

1. 人工多行 prompt 記住每次不同的 code word，再輸出 1–300 的數字，提供可觀察 busy。
2. busy 時正式 Send 送 Queue，核自己的 clientId／turn／receipt。
3. 重啟自己的 daemon，核 holder／generation／thread 不變、App 重連唯讀；再按 i 後人工問 code word，核上下文。
4. driver 觀察 idle 後正式 Send 送短回覆，核 turn/start 與 durable receipt。重送既有 UUID 不增造 row。

每則 prompt 都要求不使用工具。執行於 record-sandbox.sh；backend 仍會保存 session／cache，auth、config、rules 與 repo 寫入由該沙箱拒絕。暫存 home 是自己的 /tmp/g11live-*，停止及清理只限自己的 daemon／holders。等待都有 deadline。

工具記錄實際 codex-cli 版本；本機只查版本為 **0.159.3**，未改算舊 0.158.0 的 smoke。CLI／模型或 clientId 保存行為不符會失敗，不能放行。

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

目前只完成編譯與 guard。編譯失敗及修正輸出保留在 /private/tmp/g11c-implementation-logs/u17-live-tool-build*.log；不把編譯當 live 驗證。

## 下一步

取得四回合 opt-in，執行並保留實際輸出，再根據證據決定版本能否開放。
