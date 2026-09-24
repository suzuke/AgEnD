# agend-testkit

> **TL;DR**
> - 共用測試基礎設施（只能當 dev-dependency）：7 個 trait 的假實作、契約測試、假 daemon、3 個假 agent 程式。
> - 記住：**假實作要跑和真實作同一套契約測試**，才不會漂移（v1 #1483）；真實作在各自的施工關接上同一個 suite。
> - 下一步：`~/.cargo/bin/cargo xtask accept testkit`。

## 負責

| 項目 | 模組 | 內容 |
|---|---|---|
| 假實作 | `fakes` | `FakeDriver`、`FakeForge`、`FakeStore`、`FakeRuntime`、`FakeNotifier`、`FakeClock`、`FakeRunner` |
| 契約測試 | `contract` | 每個 trait 一個 suite：`contract::<trait>::run(實作名, 建 fixture 的函式)` 回傳 `Report` |
| 假 daemon | `fake_daemon` | 行程內的 client protocol v1 server（unix socket + JSON Lines）與 `ProbeClient` |
| 假 agent | `fake_agent` + `src/bin/` | `fake-codex-app-server`、`fake-opencode-serve`、`fake-claude` |
| 執行 future | `executor` | `block_on`：不用 async runtime 就能跑 trait 的 future |
| 暫存目錄 | `tempdir` | `TempDir`：唯一目錄，drop 時刪除 |

## 不負責

- production 邏輯；呼叫真 backend 或模型
- git 與 binding 快照 fixture（`git_fixture` 仍只有說明；不在第 2 施工關範圍，第 3 施工關需要時加）

## 假實作：共同規則

| 規則 | 怎麼用 |
|---|---|
| 可預測 | id、cursor、SHA、pid 都來自計數器；不讀時鐘、不 sleep |
| 可檢查 | `calls()` 依序列出每一次 trait 呼叫（失敗的也算） |
| 可編排 | `fail_next("<trait 方法名>", "訊息")` 讓下一次呼叫回 `FakeError`；打錯方法名會 panic |
| 契約之外的行為 | 寫在各假實作的 doc comment（例如停止不存在的 holder 會失敗） |

各假實作的編排方法：

| 假實作 | 編排 |
|---|---|
| `FakeDriver` | `with_instance`／`add_instance`；預設每次送達自動產生 busy → confirmed → turn completed → idle 事件；`set_auto_turn(false)` 後用 `push_event`；`next_receipt` |
| `FakeForge` | `push(branch)` 加一個 commit；`merges()`；`base_head()`（base branch 的 head：最後一個 merge commit，還沒 merge 時是 `BASE_ROOT`） |
| `FakeStore` | `insert_workflow`、`events(task)` |
| `FakeRuntime` | `crash(id)`（holder 自己死掉）、`adopt(handle)`（daemon 重啟後還活著的 holder）、`running()` |
| `FakeNotifier` | `delivered()` |
| `FakeClock` | `advance(ms)`、`set(ms)`（往回設會 panic）、`reads()` |
| `FakeRunner` | `on(command, ScriptedCommand)`；`takes_ms` 大於 timeout 就回 timed out；沒編排的指令 exit 127 |

## 契約測試：怎麼接真實作

1. 為真實作寫 fixture，實作 `contract::<trait>::<Trait>Fixture`（例如 `ForgeFixture::commit_to` 在暫存 repo 真的 commit，`ForgeFixture::base_head` 從 trait 外面讀 base branch 的 head）。
2. 在該 crate 的測試裡：`agend_testkit::contract::forge::run("local", MyFixture::new).assert_passed();`
3. 需要 tokio 的真實作：在 fixture 裡進入 runtime（future 仍由 `block_on` 驅動）。

| suite | 條數 | 釘住的規則 |
|---|---|---|
| Driver | 7 | 閒置送達是 Sent／Confirmed；最後會 `TurnCompleted`；cursor 唯一；從 cursor 之後只拿到新事件；重播只會變長；未知 instance 是錯誤 |
| Forge | 8 | head 是最新 commit；submit 回報送出的 head；head 對才 merge，且 base 移到回報的 merge commit；head 不對回 `HeadChanged{actual_head}` 且什麼都不變（base 沒動、work branch 沒動）；未知 branch 是錯誤 |
| Store | 8 | task 完整往返；連續多次 CAS，版本每次都嚴格變大（擋 1→2→1）；過期版本 `Conflict{Some(目前)}` 且不變；不存在 `Conflict{None}`；重複建立失敗；workflow 依版本讀；事件依序 |
| Runtime | 4 | handle 對應啟動的 instance；recover 列出正在跑的；停掉的不再出現、可再啟動 |
| Notifier | 2 | 內容完整（不截斷）；順序不變 |
| Clock | 2 | 是 unix 毫秒（2020–2100）；不倒退 |
| Runner | 5 | exit code、stdout／stderr 分開；逾時 `timed_out` 且沒有 exit code，2 秒內回報，指令被停掉（沒寫出標記檔 `timed-out-command-finished`）；在指定目錄跑 |

每個 suite 都有「故意弄壞的包裝」測試（`tests/contract_teeth.rs`），證明它抓得到漂移。

## 假 daemon

- `FakeDaemon::start()`：`<tmp>/agend-test-fd-*/daemon.sock`；drop 時停止接受連線、關閉所有已開的連線（client 讀到 EOF）、刪除 socket。
- 第一行必須是 `hello`；其他請求或無效 JSON 都回 `hello_required` 並關閉；hello 之後的無效 JSON 回 `invalid_request`，連線不關；major 不合回 `version_mismatch` 並關閉。
- 事件身分：`assign(task, ResultIdentity)` 設定目前要的結果；`done`／`result`／`review_*` 沒帶或不符 → `stale_result`、什麼都不變；接受後這個 attempt 就用掉了。
- 其他：`status`、`inbox`、`task_create`、`ask`、`answer_ask`、`subscribe_events`（先補 backlog 再推即時事件）、`subscribe_terminal`（一張快照）。
- `stale_result` 以外的錯誤碼是假 daemon 自己定的，第 8 施工關定案時要對齊。

## 假 agent 程式

所有假 agent：回覆固定為 `fake reply: <prompt>`；一個 turn 花 `--turn-ms`（預設 100）毫秒；**stdin 結束就以 0 結束**。

| 程式 | 真的指令 | 涵蓋 | 沒涵蓋 |
|---|---|---|---|
| `fake-codex-app-server` | `codex app-server --listen unix://<path>` | WebSocket（unix socket）上的 JSON-RPC：`initialize`、`thread/start`、`thread/resume`、`turn/start` → `turn/completed`、`turn/steer`、`turn/interrupt`、`thread/queue/add`、`item/commandExecution/requestApproval`（prompt 以 `run: ` 開頭時）；長路徑改用短路徑 `<tmp>/fake-codex-<hash>.sock` + symlink（陷阱 1） | `thread/turns/list`、delta、token usage、其他 approval、sandbox |
| `fake-opencode-serve` | `opencode serve --hostname 127.0.0.1 --port <p>` | `POST /session`、`message`、`prompt_async`（忙碌時排隊）、`abort`（`MessageAbortedError`）、`GET /session/:id/message`、`/session/status`、`/event` SSE（無 replay） | permission、delta、config、model |
| `fake-claude` | `claude`（互動模式） | `.claude/settings.json` hooks：`SessionStart`、`UserPromptSubmit`、`Stop`（`decision: block` 多跑一輪，`stop_hook_active`）；stdin 的 `Esc` 中斷且不觸發 Stop；`.mcp.json` channel server 的 `notifications/claude/channel` 包成 `<channel source=...>`；忙碌時的 channel 訊息不處理；transcript 寫在專案內 `.claude/fake-transcripts/<session>.jsonl`（真的 Claude Code 寫在 `~/.claude/projects/`） | TUI 畫面、啟動提示、工具與 Pre/PostToolUse、權限、hook exit 2、CLAUDE.md 來源說明的效果 |

各模組開頭的表格是準確的清單。欄位只有上面列的最少集合；第 7、12 施工關接真 backend 前要用真的 schema 對一次。

## 依賴規則

| 依賴 | 為什麼 |
|---|---|
| `agend-core` | 被測的型別與 trait |
| `serde_json` | client protocol v1、codex JSON-RPC、opencode JSON、claude hook payload 都是 JSON |
| `tungstenite`（`default-features = false`，只開 `handshake`） | codex app-server 的傳輸是 WebSocket；用現成、同步（不帶 async runtime）的實作，避免自己寫的 frame 解析和真的 client 不相容 |

- 任何 crate 都不能把它當一般依賴（`cargo xtask check-deps`）。
- HTTP 與 SSE 用 std 自己寫（`fake_agent::http`，約 150 行），不加 HTTP crate。
- 只支援 unix（`fake_daemon`、`fake_agent` 以 `cfg(unix)` 編譯）；CI 只有 ubuntu 與 macOS。

## 入口

- `agend_testkit::fakes::*`、`agend_testkit::contract::{<trait>::run, run_all_fakes}`
- `agend_testkit::fake_daemon::{FakeDaemon, ProbeClient}`
- `agend_testkit::fake_agent::{locate, codex::Probe, http::{call, EventStream}}`
- 其他 crate 的測試要用假 agent 程式：先 `cargo build -p agend-testkit --bins`，再用 `fake_agent::locate("fake-codex-app-server")`

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-testkit
~/.cargo/bin/cargo xtask accept testkit
```
