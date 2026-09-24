# agend v1 訊息注入機制調查（供 v2 設計參考）

> 歸檔註：設計階段的原始紀錄，內容原樣保存（2026-09-24）。只把絕對路徑換成佔位符：`<scratchpad>` = 當時的暫存目錄、`<v1-repo>` = agend-terminal repo、`<tmp>` = 系統暫存目錄、`~` = 使用者家目錄。文中提到的腳本、log、schema dump 沒有一起歸檔。

調查方法：讀原始碼（不信文件/註解）、`git log` 全歷史 commit 分類、
`~/.agend-terminal`（唯讀）daemon log 實跑統計。時間框：daemon log 涵蓋
2026-09-22～2026-09-23（163,383 行）；git log 涵蓋 2026-04-09（最早一批 inject 相關
fix）到 2026-09-19（最新一批）。

---

## Q1. v1 對每個 backend 實際怎麼注入

### 分派入口

`src/transport/registry.rs:30-38` `mode_for_backend`：

```rust
Backend::Codex | Backend::OpenCode => TransportMode::NativeShared,
Backend::ClaudeCode => TransportMode::ChannelBridge,
Backend::Grok | Backend::KiroCli | Backend::Agy | Backend::Shell | Backend::Raw(_) => TransportMode::LegacyPty,
```

`mode_for_instance`（同檔 40-55）疊加兩個例外：Claude 若 `legacy_pty_opt_in`
則退回 LegacyPty；任何 backend 若持久化資料證明曾是 NativeShared
（`persisted_native_shared_hint`）就不會靜默降級回 PTY——`registry.rs:14-16`
的註解明講「a persisted structured session artifact is an explicit mode
anchor and must never silently downgrade to PTY」。

`src/transport/mod.rs:70-90` 定義四種 `TransportMode`（Legacy 名稱其實有 5
個，`LegacyPty` 是唯一無結構化保證的那個）。所有結構化 adapter 共用
`AgentDeliveryTransport` trait（`mod.rs:140-150`）：`deliver` 回傳
`DeliveryReceipt`，`next_event` 拉 backend 自己的事件流。

送達狀態機（三個結構化 transport 共用）：`src/transport/receipt.rs:86-97`

```rust
enum DeliveryState { Queued, ProtocolAccepted, ObservedInSession, TurnStarted,
                      Completed, Failed, Ambiguous, AckOverdue }
```

`is_terminal()`（receipt.rs:~115）只認 `Completed|Failed|Ambiguous` 為終態。

### Codex — NativeShared，走 app-server WebSocket JSON-RPC

`src/transport/codex_app_server.rs:1-7`：Codex 的 Unix app-server 是「WebSocket
carrying JSON-RPC messages」，協定版本鎖 `CODEX_PROTOCOL = "v2"`（line 25）。

- 何時送：`turn_request()`（codex_app_server.rs:338-380）依 envelope kind 決定
  method。若 `self.in_flight.is_some()` 且 kind 不是 `Interrupt`，**自動改走
  `turn/steer`**（把新訊息塞進正在跑的 turn），只有完全 idle 才用
  `turn/start`。這代表 Codex 是三個 backend 裡唯一「busy 時真正把內容送進
  live turn」的，不是排隊。
- 忙碌時的邊界 race：`deliver_blocking`（codex_app_server.rs:166-176）——若
  `in_flight` 已標記但 `active_turn_id` 還沒從 `turn/started` 事件回填（剛送出
  turn/start、對方還沒回 turn id 的窗口期），任何新投遞都直接 `Err`
  記成 `Queued`／`"Codex active turn is not ready for turn/steer"`，**不會自動
  重試**——這是本次唯一在原始碼裡找到、Codex 自己承認的 race window。
- 確認送達：`send_request` 成功即記 `ProtocolAccepted`
  （codex_app_server.rs:~410-419，detail=`"Codex app-server accepted
  turn/steer"` 或 `"...turn/start"`），之後靠 `next_event` 收
  `turn/started`（line 773）、`turn/completed`（line 797）推進到
  `TurnStarted`/`Completed`。JSON-RPC 層級拒絕→`Failed`；傳輸層例外（連線斷）
  →`Ambiguous`，並在 detail 寫「reconcile before retry」（明確不自動重送）。

### OpenCode — NativeShared，走 loopback HTTP + SSE

`src/transport/opencode_server.rs:1-7`：「small authenticated HTTP API and a
server-wide SSE event stream」，用 `prompt_async` 送訊息，**deliberately has no
PTY fallback**。

- 何時送 / busy 時怎麼辦：`deliver_blocking`（opencode_server.rs:950-1024）。
  OpenCode 的 HTTP API **不支援 steer/interrupt**——同檔 978-982 明寫
  `DeliveryKind::Steer | DeliveryKind::Interrupt` 直接 `Failed`，detail=
  `"OpenCode has no implicit steer/interrupt operation"`。所以忙碌時只能
  排隊：`self.in_flight.is_some()` 時把 envelope 包成 `ParkedDelivery`
  push 進 `self.parked`（FIFO，struct 定義 line 54-60），
  `MAX_PARKED_REDRIVE_ATTEMPTS = 3`（line 39）、`PARKED_REDRIVE_AGING =
  120s`（line 46，防止卡死的老化強制重試）。`redrive_parked`
  （line 1906）在 turn 完成事件觸發時 FIFO 重放。**這條路徑即使成功排入
  parked queue，仍然對外回傳 `Err("OpenCode session already has an ordinary
  turn in flight")`**——這造成呼叫端（`delivery_worker`）把「已安全排隊、稍後
  會重送」記成「送達失敗」的 WARN log（見 Q2 daemon log 統計）。
- 冪等：`store.latest(envelope.delivery_id)` 先查 payload_digest 是否重放
  （opencode_server.rs:951-957），對同一 delivery_id 但內容不同直接
  `bail!("OpenCode delivery id was reused with a different payload")`。
- 確認送達：`/session/status` 輪詢（line 1547, 2064, 2158），回傳
  `busy`/`retry`→仍在跑（`TurnStarted`），其餘→按完成/失敗判定。

### Claude Code — ChannelBridge，走 MCP stdio + loopback webhook

`src/transport/claude_channel.rs:1-6`：「Claude Channels are an MCP
server-side notification capability. The bridge owns a loopback HTTP listener,
forwards authenticated webhook envelopes to Claude over MCP stdio ... It never
falls back to PTY.」

- 何時送 / busy 時怎麼辦：**這條路徑完全不判斷 Claude 是否在忙**——它閘的是
  「MCP consumer 有沒有連上並完成 initialize」（`is_ready()` /
  `mark_initialized()` / `mark_unready()`，claude_channel.rs:348-363），不是
  turn 狀態。因為訊息是透過 Claude 自己會去 poll 的 MCP 工具通道送達，本質上
  是非同步佇列，不需要「busy 就擋住」。這個 readiness gate 是
  commit `2616790f`（2026-08-19，#3305）補上的——修前有「consumer 還沒
  initialize 完就被判定 ready」的競態（PR 描述：`test(claude): expose
  premature channel readiness`）。
- 確認送達：Claude 端呼叫 `reply` MCP tool 才算真正回應
  （claude_channel.rs:763 的工具說明字串）；self-kick 場景另有
  `SELF_KICK_ACK_WINDOW = 30s`（line 52）的「未逾時＝可能還沒 ack，但非終態」
  時間窗，對應 `DeliveryState::AckOverdue`。

### 其餘 backend（Grok/KiroCli/Agy/Shell/Raw，以及 Claude opt-out）— LegacyPty

`src/transport/legacy_pty.rs:10-12`：「Explicit compatibility adapter for
backends without a verified shared protocol ... no screen/readback result is
promoted to a backend receipt.」`deliver_blocking`（line 40-67）就是呼叫一個
`injector` closure做原始 PTY write，**成功也只記 `DeliveryState::Ambiguous`**
（line 47-53），detail 明寫「legacy PTY write completed; backend acceptance is
unproven」——原始碼自己承認這條路完全沒有協定層確認。

真正提供「有沒有送到」訊號的是螢幕截圖式 heuristic，在
`src/agent/typed_inject.rs`（全檔 122 行）：
- `readback_confirm_typed`（line 41-88）：送出文字後，poll vterm（最多
  `READBACK_TIMEOUT=2s`，每 `READBACK_POLL=15ms`）比對輸入區「尾行」是否出現
  剛打的文字的尾端 24 字元 sentinel（`inject_sentinel`, line 13-32）。**逾時
  只回傳 `false`，不重試**（避免對已渲染但比對失敗的行再敲一次）。
- `observe_post_submit`（line 95-121）：送出 `\r` 後看輸入框內容在
  `POSTSUBMIT_WINDOW=500ms` 內有沒有變化；沒變化只是 `tracing::warn!`，**明講
  「NEVER retries the submit: a second `\r` would risk double-submit」**
  （line 94）。
這兩者都是 2026-06-09（`140cd6d1`, #1912）引入，取代之前「固定 sleep 猜測」
的舊法。

忙碌時的通用閘門（不分結構化/PTY，PTY 專用最後一道）：
`src/agent/inject_gate.rs`（全檔 76 行）`prepare_inject`/
`defer_direct_inject_if_needed` 呼叫
`src/inbox/notify.rs:544-546`：

```rust
pub(crate) fn should_defer_direct_inject(home: &Path, agent_name: &str) -> bool {
    crate::snapshot::agent_is_busy(home, agent_name) || operator_typing_recent(home, agent_name)
}
```

busy 或 operator 正在打字 → 非「actionable」的通知會被丟進
`notification_queue`（合併／延後送），只有 `notification_is_actionable_wake`
（notify.rs:559-571，比對 `kind=ci-ready-for-action `/`kind=task `/
`kind=query `）判定的訊息會硬闖。**這個判定式本身踩過坑**：#1483 的修法
（notify.rs 553-558 註解自述）——舊版比對訊息「內文」裡的方括號標記（如
`[delegate_task]`），但實際 PTY 喚醒指標 `[AGEND-MSG-PENDING]` 從不含這種內文
標記，導致這個 matcher 在生產環境從未真正觸發過（專案自己的
`CLAUDE.md`「Test fidelity」一節也記了這個 #1483/#1487 假陽性測試教訓）。

---

## Q2. 不穩定的實際樣貌

### git log 分類（8 類，對應題目給的分類）

方法：`git log --all --format='%h %ad %s'`（全歷史 7315 commits）先用寬鬆
關鍵字（inject/deliver/paste/submit/keystroke）篩出 262 個候選 subject，再用
每類的關鍵字組合窄篩，人工核對代表 commit。**這是 commit subject 關鍵字比對，
下界估計，不是完整 diff 審查**——實際修法數量可能更多（很多修復訊息用
`#\d+` 編號而非描述性詞彙）。

| 類別 | subject 命中數 | 代表 commit | HEAD 是否仍有對應 workaround |
|---|---|---|---|
| 文字貼上但沒送出（submit_key/auto-submit 缺漏） | 26 | `506f2d6b` handle_send must auto-submit (2026-04-23)；`d900e964` #658 PTY inject 2-tier format (2026-05-12)；`de3a0d1d` #607 Gemini submit_key 修正 | 是——per-backend submit_key preset + `inject_gate.rs` marker prepend 仍是現行機制 |
| 送到錯的時機被吃掉（busy/draft 誤判導致被吞） | 5（窄）／實際與下方 busy 類高度重疊 | `5107d6d5` #1675 defer actionable inject while operator has live draft (2026-06-03)；`2b5cba4d` #1762 only text-composing input marks draft | 是——`notify.rs:544-571` 現行版本，`notification_is_actionable_wake` 本身在 #1483 踩過「matcher 從未生產環境觸發」的坑（見上） |
| 多行/typed injection 被拆開（per-byte vs atomic、ANSI/CRLF 衝突） | 14 | `ba3503bb`/`31cdb2c1` per-backend typed_inject atomic-vs-per-byte (2026-04-10)；`68b4df03` strip ANSI 防 ESC 衝突 (2026-05-06)；`f51f1ccd` shorten task-inject note for CRLF window (2026-08-30) | 是——`typed_inject.rs` 的 readback/postsubmit 仍是「事後偵測」而非根治，daemon log 2 天內仍有 2 次 `#1912-readback-timeout` |
| busy 時丟失 | 9（窄）| **`1c20f32e`（2026-09-18，距今僅 6 天）**：修前「An ordinary delivery colliding with an in-flight opencode turn was recorded terminal Failed with no retry, so messages sent during the receiver's busy window sat silently in the inbox with no PTY nudge」——即最近才修掉的真實靜默丟失；另 `2c83f1fb`/`6e7a6f5f` #1513 busy-gate direct/notification inject (2026-05-31)、`d1b0b348` #1713 gate ServerRateLimit continue-inject | 部分——資料遺失本身已修（改成 park+redrive），但**症狀仍在**：daemon log 2 天內仍有 61 次「OpenCode session already has an ordinary turn in flight」WARN（因為 park 成功仍對外回傳 Err，見上）|
| 重複送達 | 11 | `19d00479` #836 notification dedup race (2026-05-16)；`58e8a9cd` #911 compose_aware_inject dedup gate hybrid (2026-05-19)；`517ce0fc` #1135 merge dual delivery into inbox-only path | 是——daemon log 當天有 1 次「dedup-state GC: daemon-init sweep complete」例行掃描，dedup 機制仍在運作（但分散成至少 3 套：#836 msg_id、#911 compose_aware、OpenCode payload_digest reuse check） |
| 截斷 | 3 | `7ac2cec6` UTF-8 truncation panic (2026-04-09)；`94ed912d` #1352 Telegram inbound length-based delivery split (2026-05-28)；`01872577`/`72111c99` #3535 codex 截斷 thread id 的 ambiguous-delivery 拒絕 (2026-09-07，距今 17 天) | 是——最新一筆只有 17 天前 |
| 選單或 modal 擋住 | 4 | `d438ad65` #996 claude trust dismiss → single Enter (2026-05-20)；`72161f7c` kiro-cli trust dialog dismiss with split keystroke delay (2026-04-14)；`f7bb9d2e` #468 dismiss patterns regex-anchored, no longer auto-inject from scrollback | 是——`src/agent/dismiss.rs`（43,889 bytes）與 `dev_modal.rs`（59,368 bytes）都在 2026-09-14 仍有修改，是持續維護中的大子系統，非一次性修復 |
| 狀態誤判導致時機錯誤 | 11 | `a0c1cffa` event-bus subscribers 註冊 cutover regression → silent delivery drop (2026-06-04)；`22b1b404` #1720 event-bus delivery failures visible + 關閉 cron silent-drop；`140cd6d1` #1912 readback-confirm inject 取代 fixed-sleep guess (2026-06-09)；`9d1c0d01`/`3eaf2eca` 明確標號「**Sprint 54 silent-drop class 6th instance**」| 是——專案自己的 `CLAUDE.md` 記載 `#919` state-detection red-anchor 是「Phase A 已上、Phase B 待 telemetry 確認才收緊」的**尚未完全解決**的緩解措施 |

**重要背景數字**：這整個問題域從 2026-04-09（最早一批 submit_key/inject 修復）
持續修到 2026-09-19（最新一批 OpenCode busy-park 修復），橫跨約 5.5 個月、
至少 8 個不同失敗模式，且團隊自己用「silent-drop class 6th instance」這種
編號方式追蹤同一類 bug 的第 N 次出現——代表這不是幾個孤立 bug，而是這個
架構（結構化 API + PTY 混合、狀態靠側面推測）本質上難以窮盡邊界情況。

### daemon log 實跑統計（~/.agend-terminal，唯讀，2026-09-22～09-23，163,383 行）

`delivery_worker: structured transport delivery failed` 總計 **124 筆**，依
error 訊息分群：

| 訊息模板 | 次數 | 意義 |
|---|---|---|
| `error=OpenCode session already has an ordinary turn in flight` | 61 | busy collision——多數已被內部 park+redrive 接住（見上），但仍以 WARN 形式記成「失敗」，觀測面尚未跟上修復 |
| `error=agent '<name>' not found` | 60（`suzuke` 52 次、`operator-2` 8 次（第三方帳號，已改為代稱），取自 `src/error.rs:31` `AgentNotFound`）| 目標似乎是操作者/人類帳號名而非 agent instance 名——像是有訊息被誤路由進 agent 專用的結構化投遞路徑；未深入追查呼叫端，**標「未查證」根因** |
| `error=Claude ChannelBridge locator is not owned by a live bridge` | 3 | bridge 尚未起來或已被回收時的競態 |

其他訊號：`busy-gated dispatch parked — auto-redrive`
（`src/mcp/handlers/comms_gates/busy_park/mod.rs:283`，MCP 層級的 agent-to-agent
`delegate_task` 忙碌停放，與上面 OpenCode 傳輸層的 park 是**兩套獨立實作**）
13 次；`#1912-readback-timeout`/`#1912-postsubmit-nochange` 2 次；
`ambiguous-delivery` 相關 0 次（採樣窗口內未觸發）。

---

## Q3. 三個 backend 官方的「送訊息給進行中 session」機制

（由背景 general-purpose/sonnet agent 用 WebSearch/WebFetch 查證，完整原始
記錄另存 `injection-research-q3.md`——**注意**：部分頁面是透過自動摘要工具
擷取，非逐字讀取原文，agent 已自行標註此限制，設計時建議親自複核官方頁面。）

**結論**：三個 backend 裡只有 Codex 有「真正的忙碌中注入」原語；Claude Code
的兩個候選機制（Channels、Remote Control）都是「排到下一個 turn 邊界」而非
中斷；OpenCode 官方文件完全沒講忙碌時的合約，且有公開 bug 顯示忙碌時可能
靜默卡住而非乾淨排隊。

### Codex CLI app-server
- `turn/steer`——https://developers.openai.com/codex/app-server ；原始碼
  `codex-rs/app-server-protocol/src/protocol/v2/turn.rs:293`（參數）、`:320`
  （回應）；`turn/start` 在 `:167`/`:284`；`turn/interrupt`（取消）在 `:327`。
  https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/src/protocol/v2/turn.rs
- 忙碌時：`turn/steer` **要求**必須有進行中的 turn，沒有就失敗——這是三者中
  唯一「設計上就是給忙碌中注入用」的呼叫，與 v1 `codex_app_server.rs` 的用法
  一致。
- 送達確認：只有 JSON-RPC 同步回應 `{"result":{"turnId":"turn_456"}}`；文件
  明講不會另外觸發 `turn/started` 通知；是否有其他事件標示「注入的內容已被
  採用」——**未查證**。
- 穩定度：核心 `turn/start`/`turn/steer` 基本欄位原始碼未標 experimental，
  但進階子欄位（如 `turn/steer.additionalContext`）需
  `capabilities.experimentalApi`；官方文件另一處又寫「The app-server command
  and WebSocket transport are experimental and aren't supported for
  production workloads」——**這句話的適用範圍（整個指令 vs. 只有
  WebSocket transport）語意含糊，需親自讀原文件確認**。

### OpenCode server API
- `POST /session/:id/message`（同步等回應）與 `POST /session/:id/prompt_async`
  ——opencode.ai/docs/server（僅摘要擷取，非原文）。
- 忙碌時：**官方文件完全沒定義**（WebFetch opencode.ai/docs/sdk 確認查無）。
  實際證據來自公開 issue：
  https://github.com/anomalyco/opencode/issues/46842 ——`prompt_async`
  在 runner 狀態為 "Running" 時送出會寫進 DB 但**該 turn 永遠不會被排程**
  （`packages/opencode/src/effect/runner.ts` 的 `ensureRunning` 回傳既有 run、
  從不啟動新的）——本質是靜默卡住，不是乾淨排隊或拒絕。另見
  https://github.com/anomalyco/opencode/issues/13304（某版本確有排隊/取消
  概念）、https://github.com/openchamber/openchamber/issues/3194（subagent
  執行中訊息被靜默丟棄）。
- 送達確認：`GET /event` SSE（`server.connected`→`message.part.updated`），
  沒有文件化的「忙碌中注入已接受」ack。
- 穩定度：`/session` HTTP 介面本身未標 experimental（只有另一節 Tools 標
  experimental）；但忙碌時行為應視為不可靠（上述公開 bug）。

### Claude Code
- `Stop` hook：只在「Claude 自然結束回應」時觸發，`{"decision":"block"}`
  是阻止結束，**不是**把新內容注入正在跑的 turn。
  https://code.claude.com/docs/en/hooks
- `UserPromptSubmit` + `additionalContext`：只在使用者自己送出 prompt 時
  同步觸發（「before Claude processes it」），外部/非同步程式無法用它在
  turn 中途插入；30s timeout，逾時靜默丟棄。同上連結。
- `--channels`：**research preview**（隱藏旗標，合約可能變動）。忙碌時行為
  文件明講：「Events queue into the session and are processed in order. If
  several notifications arrive while Claude is busy, they're delivered
  together on the next turn.」且「Claude Code doesn't acknowledge
  notifications ... drops the events silently and returns no error」（除非
  自建 reply-tool 回合）。
  https://code.claude.com/docs/en/channels ,
  https://code.claude.com/docs/en/channels-reference
- Remote Control：**GA（"available on all plans"）**，非 preview。官方比較表
  將它標為「Steering in-progress work from another device」的機制；但忙碌
  時行為：「when you send a prompt from a connected device before the
  current turn ends, Claude Code queues it ... after that turn finishes」——
  跟 Channels 一樣是排到下個 turn 邊界，**不是**中途插入。
  https://code.claude.com/docs/en/remote-control
- `--input-format stream-json`：文件裡一行 CLI 旗標表格之外幾乎沒有說明；
  一則 stdin JSON 行是「在下一個 turn 邊界送達」而非即時中斷；Anthropic 把
  對應的文件請求關閉為「not planned」：
  https://github.com/anthropics/claude-code/issues/24594。有無送達 ack——
  **未查證**。

---

## Q4. v2 統一注入模型建議（草案）

1. **送達確認的定義**：三段式信賴分級，不要只有二元「成功/失敗」：
   - **Protocol-Confirmed**：backend 自己協定產生的事件（Codex `turn/started`、
     OpenCode `/session/status` 轉態、Claude `reply` tool 被呼叫）。
   - **Heuristic-Observed**：screen-scrape 式確認（`typed_inject.rs` 的
     sentinel/tail-diff）——永遠標成 `Ambiguous`，不得晉升為 `Completed`，
     這點 v1 的 `legacy_pty.rs:47-60` 已經做對，v2 應該保留並在 UI/log 上更
     顯眼地標出「這條線的送達是猜的」。
   - **Unconfirmed**：write 呼叫本身失敗或逾時。
   v1 現有的 `DeliveryState`（`receipt.rs:86-97`）骨架是好的，v2 應該把它
   變成**所有 transport 必須遵守的唯一契約**，而不是像現在三套結構化
   transport 各自土法煉鋼判斷什麼時候該記哪個狀態。

2. **忙碌策略——按 backend 協定能力分兩檔，不要每個 transport 各發明一套**：
   - 協定原生支援「塞進 live turn」（Codex `turn/steer`）→ 用它，這是 v1
     Codex 這條已經做對的地方，v2 應該保留當範本。
   - 協定不支援 steer（OpenCode）→ 用**唯一一套**排隊/停放/老化重試/
     fail-closed 原語，而不是像現在 OpenCode transport 自己發明
     `ParkedDelivery`、MCP dispatch 又自己發明 `busy_park` 模組——同一問題
     兩份實作正是 judgment rubric §4「特例越堆越多」的訊號。且停放成功時
     **不該**再對呼叫端回傳 `Err`（v1 現在的行為導致 daemon log 充滿假警訊，
     見 Q2 的 61 次 WARN）。
   - Claude 這種「訊息走非同步通道、agent 自己 poll」的模式最乾淨，直接不需要
     busy 判斷；v2 若能替 Codex/OpenCode 找到類似的原生非同步通道
     （見 Q3，若存在）應優先於「猜測 busy 再排隊」。

3. **重試與冪等**：`delivery_id` (UUID) + `payload_digest` 的重放檢查
   （OpenCode `opencode_server.rs:951-957` 已有）應該是**唯一**的冪等層，
   取代現在至少 3 套並存的 dedup 機制（#836 msg_id 抑制、#911
   compose_aware_inject dedup gate、OpenCode 自己的 payload_digest 檢查）。
   PTY 提交嚴禁自動重試第二次 `\r`（`typed_inject.rs:94` 的教訓：雙重送出
   風險 > 沒送到的風險）。

4. **何時允許退回 PTY 鍵入**：只在 backend 完全沒有結構化協定時
   （目前是 Grok/Kiro/Agy/Shell/Raw），且必須：(a) 保留 v1 的
   readback-confirm + post-submit-diff 兩層螢幕觀察；(b) 逾時只警告不重試；
   (c) 送達狀態永遠標 `Ambiguous`，絕不冒充 `Completed`。**不要**把 PTY 當
   Claude/Codex/OpenCode 的降級路徑——v1 的 `legacy_pty.rs` 和
   `opencode_server.rs` doc comment 都已經明文「never falls back to PTY」，
   這個原則要延續到 v2。

5. **每個 backend 對應機制（已用 Q3 官方查證更新）**：
   - **Codex**：v1 現行的 `turn/steer` 就是官方唯一「忙碌中注入」原語
     （`codex-rs/app-server-protocol/.../turn.rs:293`）——v2 應該原樣保留，
     只需補上 Q1 發現的 race window（`active_turn_id` 還沒回填時的窗口期）
     退避重試，而非直接放棄。
   - **OpenCode**：官方 API **沒有**忙碌合約，且有公開 bug
     （anomalyco/opencode#46842）證實忙碌時 `prompt_async` 可能靜默卡住
     （runner 從不排程新 turn）。v1 自建的 park+redrive
     （`MAX_PARKED_REDRIVE_ATTEMPTS=3` + `PARKED_REDRIVE_AGING=120s`）不是
     繞遠路，而是官方協定本身沒給保證下的**必要**補償；v2 應保留這套機制，
     但要修掉「排隊成功仍回報 Err」這個觀測面缺陷（Q2 daemon log 61 次假警訊
     的根因）。
   - **Claude Code**：官方兩個「送訊息進行中 session」候選（`--channels`
     research preview、Remote Control GA）都是「排到下一個 turn 邊界」，
     且 Channels 官方自己說「不 ack，靜默丟棄」——語意上其實比 v1 現有的
     MCP webhook-bridge（`claude_channel.rs`，用 consumer-readiness 而非
     turn-busy 判斷，且靠 `reply` tool 呼叫當作真確認）更弱。v2 對 Claude
     **不需要**改用官方 Channels/Remote Control，現有 ChannelBridge 設計方向
     是對的；但可以考慮把 `--channels`（GA 版一旦成熟）當作與
     ChannelBridge 並存的官方旁路，降低對自建 webhook 的長期維護負擔。
   - **Grok/KiroCli/Agy/Shell/Raw**：無結構化協定，維持 LegacyPty + 兩層
     screen-scrape 確認（見上第 4 點），這是誠實的最後手段，不是預設路徑。
