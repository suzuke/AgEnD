# 第 11 施工關 C 段：Codex U17 驗證

> **TL;DR**
> - fake Codex 已走真 App／client／daemon／holder／wrapper／PTY／driver／SQLite，驗人工 turn 和 daemon 訊息分開對帳。
> - 六個測試通過（含一個 re-exec 入口）；已修正人工同文誤認 receipt 與 Queue receipt 缺 turn id。
> - 真 Codex 0.159.3 已有[首次 U17 證據](gate-11c-u17-live-validation.md)；下一步是全新 verifier 與版本確認，一般 daemon 仍 not_supported。

## 實際路徑與範圍

crates/agend/tests/codex_u17.rs 啟動真 holder 和 production Codex shell wrapper。明確設定 -c agend_fake_manual_tui=true 的 fake frontend 才進 raw PTY 模式；它把 bracketed paste／Enter 送到同一 remote thread 的 turn/start，不帶 daemon clientUserMessageId。busy／idle、user items 和 queue 都由實際 fake app-server 產生，不由測試造事件。fake Codex 預設 frontend 保持原行為。

前兩個 foundation 情境在測試程序內組裝 driver／runtime／SQLite；第三個驗 crash-window 的嚴格 receipt。另有完整 App／ClientSource／protocol server／daemon 子程序情境，以及一般 daemon 拒絕診斷環境變數的情境。沒有啟動真 Codex 或 LLM；App 事件由測試送入，實體終端 capture 另見 [外層 PTY](gate-11c-outer-validation.md)。

| 情境 | 核對結果 |
|---|---|
| 人工 turn 與 queue | 繁中多行進同 thread；driver 觀察 busy／idle，人工輸入不新增 daemon row。Queue 以自己的 clientId／turn 確認，一個 row／receipt |
| 相同文字與 component restart | 未嘗試 Queued row 不能取人工同文 receipt；重接後 holder／generation／thread 不變，舊 attach 拒絕 |
| attempted crash-window | 已寫 attempted_at、RPC 未送出，人工同文仍不取 receipt；idle 後自己的 identified retry 才確認一次 |
| 完整 App／daemon | 正式 Send、daemon 重啟、草稿、唯讀重連、重新 i、scope／caller 拒絕、durable turn id 與清理 |
| 預設 daemon | 同名 AGEND_U17_PROBE=1 環境變數不放行正常 CLI |
| re-exec 入口 | 正常 test invocation 是 no-op；只有指定子程序執行診斷 daemon，不重複計數 |

## 完整 App／daemon 情境

另一程序執行同一 daemon boot／server，並提供真 AgEnD binary 給 holder／shim。只有明確 AGEND_U17_PROBE=1 的 run_u17_probe 與指定 instance 能取得 Codex 輸入；一般 CLI 的 daemon 永遠使用預設拒絕政策。

- i／尺寸確認後，繁中多行 paste＋Enter 進同 thread。真 driver 的 turn 診斷核 busy／idle；另一 socket 的 history idle 不代表 driver 已收到通知。
- 忙碌時正式 agent Send 走 Queue，自己的 clientId／turn 對應一個 inbox row；人工 turn 不增造 row。
- 貼入草稿後停止 daemon，holder 持續。App 斷線停送、重連唯讀；再按 i 後提交原草稿。holder pid、generation、thread 不變。
- driver 觀察人工 turn idle 後，第二個 Send 走 turn/start；重送第一個 UUID 不增加 row／user item。兩個 durable Confirmed row 分別保存自己的 turn id。
- 別的 Codex instance 與 agent caller 不能取得診斷控制。App reader threads 回到 0，自己的 holder 完整清理。

## 原反例與修正

原 history 允許任何 no-turn Queued row 用相同文字對帳；第二個 foundation 在原程式 exit 101，未嘗試 row 被誤標 Confirmed。原 log u17-foundation-original.log 與原 source 保留。第一批修正拒絕明確外來 clientId，並要求 no-turn Queued 文字 fallback 已有 attempted_at。

後續原生 crash-window 再重現：attempted_at 已寫、RPC 尚未送出，人工同文 turn 仍被誤標 Confirmed（u17-attempted-text-original.log，exit 101）。允許人工輸入的 instance 現在必須以自己的 clientId 對帳；live notification、reconcile、Driver events 三處都套用，沒有文字 fallback。相同回歸證明人工 turn 沒有 receipt，idle 後自己的 identified retry 才確認一次。

一般 daemon 尚未允許人工輸入，保留第 7 施工關的舊 no-clientId lost-reply 相容路徑；它不是人工輸入安全證據。未來版本 smoke 必須證明 daemon clientId 能保存，否則不得開放該版本。

完整路徑另抓到 Queue row 已 Confirmed、turn_id 卻為空（u17-full-app-exclusive-fixed.log，exit 101）。confirm 現在在同一 durable advance 保存實際 UserItem turn id；回歸核 SQLite 與自己的 clientId／turn 一致。

fixture 原先在另一 history socket idle 後立即假設 driver 已 idle；實際曾走合法 Queue/start race。改等待該 turn 的真 driver idle 診斷，再核 idle turn/start；不放寬方法斷言。原 u17-full-app-strict-fixed.log 保留。

## 本機證據

| 檢查 | 結果 |
|---|---|
| 原生 U17 suite | 6 passed／0 ignored；含 re-exec 入口 |
| history 匹配 | 5 passed；嚴格 clientId 與舊相容行為分開核對 |
| 既有 Codex driver 契約 | 15 passed；忙碌三級、lost reply、冪等與四次重啟 |
| 真 Codex 工具 | 0.159.3 首次四回合 U17 通過；第四回合只核 receipt；[原始範圍](gate-11c-u17-live-validation.md) |
| 完整本機 workspace | 888 passed／2 個既有 ignored；在抽取共用 fixture 和新增互動 demo 前執行 |
| accept core | 158 passed／2 個既有 ignored；demo、fmt／clippy、實際 thumb no-std 通過 |
| accept tui | 594 passed／0 ignored；fake／真 daemon／完整 U17 三個 demos 通過；抽取共用 fixture 後執行 |

原 logs 在 /private/tmp/g11c-implementation-logs/u17-*.log。完整 U17 正向為 u17-full-app-driver-idle.log；完整 workspace 與 accept core 的時點早於最後 demo；accept tui 重跑最新相關 crates，新互動 demo 另跑 clippy／build 及外層 PTY。02ab00c 的 Ubuntu／macOS push／PR 四個 CI jobs 已核全部 success，新改動另跑 CI。

## 重跑

~~~bash
cd /Users/suzuke/AlphaCR-worktrees/AgEnD-v2-g11c-terminal
export CARGO_TARGET_DIR=/private/tmp/AgEnD-g11c-target
~/.cargo/bin/cargo build -p agend --example fake_codex
~/.cargo/bin/cargo test -p agend --test codex_u17 -- --nocapture
~/.cargo/bin/cargo test -p agend-daemon --test codex_driver
~~~

## Acceptance demo

accept tui 已加入 codex_u17_probe，直接共用完整 App 情境與 fake producer；獨立 demo 已通過。它驗預設拒絕及完整 fake 路徑，不呼叫真 LLM。

~~~bash
~/.cargo/bin/cargo run --quiet -p agend --example codex_u17_probe
~~~

## 下一步

真 Codex 0.159.3 首次 U17 已通過；[原始範圍](gate-11c-u17-live-validation.md)、[工具](gate-11c-u17-live.md)。接著做全新 verifier、版本確認、最新 checks 與人工驗收。C 段仍在 [draft PR #145](https://github.com/suzuke/AgEnD/pull/145)，未驗收或 merge。
