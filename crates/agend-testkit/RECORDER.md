# 錄製器與一致性檢查

> **TL;DR**
> - `agend-record` 用假 agent 模擬的同一條傳輸路徑，驅動**真的** claude／codex／opencode 跑 5 個情境，錄成 `transcripts/<backend>/<scenario>.jsonl`；`tests/conformance.rs` 在 CI 裡用同一套情境驅動假 agent，按形狀比對錄製檔。
> - 記住：**錄製只在寫入沙箱裡跑**（`cargo xtask record … --sandbox <script>`），每個情境在全新的 `/private/tmp/agend-rec-<backend>-XXXX`；CI 的一致性檢查不碰真 CLI、不花 token。
> - 下一步：CLI 升版時重錄（「重錄」），一致性檢查不過就改假 agent。

## 錄製的內容

| backend | 真 CLI（錄製時） | 傳輸 | 設定（只作用於這次執行） |
|---|---|---|---|
| codex | `codex-cli 0.156.1` | `codex app-server --listen unix://…` 上的 WebSocket JSON-RPC | 模型 `gpt-6-luna`、reasoning `low`、approval `untrusted`、sandbox `read-only`；`-c notify=[]`、`--disable hooks` |
| opencode | `opencode 1.18.31` | `opencode serve --pure` 的 HTTP + `/event` SSE | 免費模型 `opencode/space-bunny-free`（也當 small model）；`permission` bash／edit／webfetch 都 `ask`；關自動更新 |
| claude | `claude 2.1.282` | 互動模式：`.claude/settings.json` hooks、`.mcp.json` channel（D16）、tmux 裡的 TTY 送按鍵 | `--model haiku --effort low --setting-sources project,local`（不載入使用者自己的設定與 hook）、專案規則 `ask: Bash(echo *)`、`DISABLE_AUTOUPDATER=1` |

| 情境 | 做什麼 | codex | opencode | claude |
|---|---|---|---|---|
| `one_turn` | 「Reply with exactly: OK」 | `turn/start` | `prompt_async` | channel 訊息 |
| `interrupt` | 長回覆，2 秒後用 CLI 自己的中斷 | `turn/interrupt` | `POST …/abort` | `Esc` |
| `approval` | 要求執行 `echo agend-record`，**一律拒絕** | `decline` | `reject` | 權限對話框按 `Esc` |
| `busy` | 忙碌時再送一則 | `turn/steer` + `thread/queue/add` | 第二個 `prompt_async` | 忙碌時 channel 訊息 + Stop hook 排隊（D16） |
| `resume` | 停掉 CLI、重啟、以 id 接續 | `thread/resume` | `GET /session/:id` 後再 prompt | `--resume <id>` 後打字送出 |

每個錄製檔第一行是 header（backend、CLI 版本、日期、情境），之後每行一則訊息 `{"from": "client"|"backend", "via": <通道>, "msg": …}`，兩個方向依時間順序。通道：`ws`（codex）、`http`／`sse`（opencode）、`hook`／`mcp`／`key`／`screen`（claude；`screen` 只記錄器等到的畫面標記，例如 `interrupted`）。claude 的啟動對話框（信任資料夾、專案 MCP server、development channels）由錄製器先回答，不在錄製檔裡。

## 遮蔽（`recorder::redact`）

寫檔前一律遮蔽，保留 JSON 型別：情境目錄 → `<rec>`、`$HOME` → `~`、使用者名稱與主機名稱、UUID 與 `ses_…`／`msg_…`／`toolu_…`／`call_…` 等 id → 穩定的 `<uuid-1>`、`<ses-2>`、email、token／JWT、帳號資訊（`planType`、`userAgent`、用量）、使用者自己的 MCP server 名稱。寫檔前再跑 `redact::scan`（家目錄路徑、UUID、email、token、CLI auth 檔裡的字串……），有任何發現就不寫。規則改了：`agend-record redact <檔案>…` 就地重跑。

## 一致性檢查的比對規則（`recorder::shape`，唯一出處）

1. 依「通道／方向」分流（`ws/backend`、`http/client`…），只比同一流內的順序；client 請求和 backend 事件之間的先後是時序，不比。
2. 訊息種類：JSON-RPC 的 `request`／`notify`／`result`／`error <method>`；HTTP 的 `方法 路徑`（id 段落 → `:id`）與回應 `狀態 請求`；SSE 的 `type`；hook 的事件名與回覆；按鍵；畫面標記。
3. 兩邊都先丟掉：`IGNORED`（機器、帳號、計時器雜訊，各附原因）與模型自己決定的 reasoning（事件與訊息內容裡的 reasoning part／item）。
4. `DELIBERATE`：假 agent 刻意和真 CLI 不同的地方，從真的那邊丟掉最後一則（目前只有 claude `busy`，見下表）。
5. `UNORDERED`：非同步的簿記事件（opencode 的 `session.*`、`message.updated`，codex 的 `thread/queue/changed`）只比「有沒有」與合併後的形狀，不比順序。其餘保持順序，連續同種類合併成一則（串流 delta）。
6. 形狀：欄位名稱、巢狀、值的型別；值本身不比，只有 `type`、`status`、`role`、`kind`、`source`、`hook_event_name`、`decision`、`stop_hook_active`、`method` 保留值。
7. 陣列比元素形狀的集合，空陣列和任何陣列相符；是 id 的物件 key 當成 `:id`。

## 發現的差異與處理（2026-09-25 錄製）

| backend | 真 CLI | 假 agent 原本 | 處理 |
|---|---|---|---|
| codex | `--listen` 路徑再短也綁在 `/private/tmp/codex-daemon-<uid>/<sha256>`，要求的路徑是 symlink | 只有長路徑才 symlink | 改假的：一律 symlink |
| codex | 通知帶 `emittedAtMs`；`initialize`、`thread/start`（完整設定 + thread 物件，另發 `thread/started`）、turn 物件（`items`、`itemsView`、時間）欄位很多 | 只有 `id`／`status` | 改假的：照錄製補齊 |
| codex | 每輪：`thread/status/changed`（active／idle）、`item/started` + `item/completed`（userMessage、agentMessage）、`item/agentMessage/delta`、`thread/tokenUsage/updated`；item 在模型開始回覆時才出現 | 只有兩個 `item/completed`，一開始就送 | 改假的：`--turn-ms` 到了才送 item |
| codex | `turn/steer` 在同一輪裡另成一則 user message 與回覆 | 把文字併進同一個回覆 | 改假的 |
| codex | `thread/queue/add` 回 `{queuedSubmission}`，另發 `thread/queue/changed`（加入、出列，串流中也會發）；出列那輪的 user message 帶 `clientId` | 回 `{}`，無通知 | 改假的；`thread/queue/changed` 列入 `UNORDERED` |
| codex | 授權：`thread/status/changed` `waitingOnApproval`、`commandExecution` item（`/bin/zsh -lc '…'`）、請求帶 `availableDecisions` 等、`serverRequest/resolved`、item `declined` | 只有請求 | 改假的 |
| codex | 中斷：先 idle 再 `turn/completed` `interrupted`（`items: []`） | 少 idle | 改假的 |
| codex | 重啟後 `thread/resume` 回完整設定與所有 turn，另發 `deprecationNotice`、`thread/tokenUsage/updated`、`thread/goal/cleared` | 重啟後 thread 不存在 | 改假的：`AGEND_FAKE_STATE_DIR` 持久化 |
| codex | 使用者設定的 MCP server、帳號、遠端控制通知 | — | `IGNORED`（依機器而定） |
| opencode | 事件帶 `id`；session 物件欄位多；另有 `session.created`／`updated`／`diff` | 只有 `id` | 改假的；簿記事件 `UNORDERED` |
| opencode | 一輪：user message 與其 text part、busy 兩次、assistant message、`step-start`、空 text part、`message.part.delta`、完整 text part、`step-finish`、assistant 完成（兩次）、busy、idle、`session.idle` | 少了 step、delta，只有一個 text part | 改假的；回覆的 parts 也改成 `[step-start, text, step-finish]` |
| opencode | 忙碌時的 prompt：user message 立刻送出，前一輪結束後直接接著跑（中間沒有 idle） | 默默排隊 | 改假的 |
| opencode | abort：`session.error`（`MessageAbortedError`）、idle，之後帶 error 的 assistant message、再 idle 一次 | 只有 message 帶 error | 改假的 |
| opencode | 權限：bash tool part（`pending`、`running`）、`permission.asked`、`GET /permission` 列出、回覆後 `permission.replied`、tool part `error`、`step-finish` `tool-calls`、不再回覆文字 | 沒有權限（`GET /permission` 永遠 `[]`） | 改假的：prompt 有 `run: ` 行時觸發 |
| opencode | 重啟後 session 還在 | 重啟後 404 | 改假的：`AGEND_FAKE_STATE_DIR` 持久化 |
| opencode | `plugin.added`、`catalog.updated` 等、`server.heartbeat` | — | `IGNORED` |
| claude | MCP `initialize`：protocol `2025-11-25`、`capabilities {elicitation, roots}`、`clientInfo` 五個欄位；之後送 `tools/list` | `2025-06-18`、空 capabilities、沒有 `tools/list` | 改假的 |
| claude | 每個 hook 都有 `scratchpad_dir`；`UserPromptSubmit`／`Stop` 有 `prompt_id`；`Stop` 有 `last_assistant_message`、`background_tasks`、`session_crons`；`SessionStart` 有 `model`、沒有 `permission_mode`；resume 時改帶 `context_tokens` 等四個欄位 | 欄位不同 | 改假的 |
| claude | `/exit` 觸發 `SessionEnd`（`reason: "prompt_input_exit"`） | 沒有 | 改假的 |
| claude | 授權：`PreToolUse`（Bash、`tool_input`、`tool_use_id`）、`PermissionRequest`、畫面 `Do you want to proceed?`；`Esc` 取消後沒有其他 hook、沒有 Stop | 沒有工具與權限 | 改假的：`run: ` 行觸發 |
| claude | **忙碌時的 channel 訊息會排隊**：這輪（含 Stop hook 續行）結束後才觸發 `UserPromptSubmit`，然後照做（多一個 Stop） | 立刻觸發 `UserPromptSubmit` 並丟掉 | `UserPromptSubmit` 的時間點改成和真的一樣；**仍然不處理**（D16 要 driver 用 Stop hook 排隊、gate-02 A6 的最壞情況），列入 `shape::DELIBERATE`，寫在假 claude 模組開頭 |
| claude | 啟動時多一個「專案 MCP server」對話框（spike-claude-f 說這條路徑沒有）；`CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` 會關掉 channel | — | 錄製器處理；假 agent 不模擬對話框（已寫在模組開頭） |

## 重錄（CLI 升版時）

```bash
~/.cargo/bin/cargo xtask record codex --sandbox ~/Documents/Hack/AgEnD-ops/record-sandbox.sh   # 或 opencode、claude；可只列情境
~/.cargo/bin/cargo xtask record claude one_turn --sandbox <script>
target/debug/agend-record startup-check claude   # 在沙箱裡：只啟動、回答對話框、/exit，不送 prompt
~/.cargo/bin/cargo test -p agend-testkit --test conformance
```

沙箱腳本只允許寫 `/private/tmp`、`TMPDIR` 與 CLI 自己的狀態目錄（`~/.claude`、`~/.claude.json`、`~/.codex`、opencode 的資料／設定／快取目錄），而且這些目錄裡使用者自己的設定檔仍是唯讀；repo 在錄製時不可寫。錄製檔先寫到 `mktemp -d /private/tmp/agend-rec-out-XXXX`，由 xtask 在沙箱外複製進 `transcripts/`（失敗的情境留在輸出目錄，名為 `<情境>.failed.jsonl`）。

## 新增一個 backend

1. 在 `src/recorder/` 加一個模組，實作 `recorder::Backend`：`name`、`program`（真 CLI）、`fake`（假 agent binary）、`scenarios`（支援的情境）、`run`（在 `<dir>/project` 啟動 agent、走傳輸、跑情境步驟、把每則訊息 `log.push` 進來、最後關掉）。同一段 `run` 要能跑真的與假的（看 `Agent::fake`）。
2. 把它加進 `recorder::BACKENDS`。
3. 需要時在 `recorder::shape` 的 `IGNORED`／`UNORDERED` 加規則（附原因）。
4. `cargo xtask record <name> --sandbox <script>` 錄製，檢查遮蔽結果後 commit；`tests/conformance.rs` 加一個 `#[test]`。

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-testkit --test conformance
cat crates/agend-testkit/README.md
```
