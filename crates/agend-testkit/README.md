# agend-testkit

> **TL;DR**
> - 共用測試基礎設施（只能當 dev-dependency）：7 個 trait 的假實作、契約測試、假 daemon、3 個假 agent 程式。
> - 記住：**假實作要跑和真實作同一套契約測試**，才不會漂移（v1 #1483）；真實作在各自的施工關接上同一個 suite。
> - 下一步：`~/.cargo/bin/cargo xtask accept testkit`；契約規則看 [CONTRACTS.md](CONTRACTS.md)。

## 負責

| 項目 | 模組 | 內容 |
|---|---|---|
| 假實作 | `fakes` | `FakeDriver`、`FakeForge`、`FakeStore`、`FakeRuntime`、`FakeNotifier`、`FakeClock`、`FakeRunner` |
| 契約測試 | `contract` | 每個 trait 一個 suite：`contract::<trait>::run(實作名, 建 fixture 的函式)` 回傳 `Report`；規則編號見 [CONTRACTS.md](CONTRACTS.md) |
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
| `FakeDriver` | `with_instance`／`add_instance`；預設每次送達自動產生 busy → confirmed → turn completed → idle 事件，同一個訊息 id 再送只回回條、不再產生 turn；`set_auto_turn(false)` 後用 `push_event`；`next_receipt`；`restarted()`（daemon 重啟：同一個 backend 上的新 driver，事件與 cursor 都在） |
| `FakeForge` | `push(branch)` 加一個 commit；`merges()`；`base_head()`（base branch 的 head：最後一個 merge commit，還沒 merge 時是 `BASE_ROOT`） |
| `FakeStore` | `insert_workflow`、`events(task)`；`reopen()`（daemon 重啟：同一份資料上的新 handle） |
| `FakeRuntime` | `crash(id)`（holder 自己死掉）、`adopt(handle)`（外來的 holder）、`running()`；`restarted()`（daemon 重啟：同一張 holder 表上的新 runtime） |
| `FakeNotifier` | `delivered()` |
| `FakeClock` | `advance(ms)`、`set(ms)`（往回設會 panic）、`peek()`（不算一次讀取）、`reads()` |
| `FakeRunner` | `on(command, ScriptedCommand)`；`takes_ms` 大於 timeout 就回 timed out；沒編排的指令 exit 127 |

## 契約測試：怎麼接真實作

1. 為真實作寫 fixture，實作 `contract::<trait>::<Trait>Fixture`（例如 `ForgeFixture::commit_to` 在暫存 repo 真的 commit，`ForgeFixture::base_head` 從 trait 外面讀 base branch 的 head）。
2. 在該 crate 的測試裡：`agend_testkit::contract::forge::run("local", MyFixture::new).assert_passed();`
3. 需要 tokio 的真實作：在 fixture 裡進入 runtime（future 仍由 `block_on` 驅動）。

規則的唯一清單是 [CONTRACTS.md](CONTRACTS.md)：每條一個編號，每個 case 標著它驗的編號（失敗訊息 `FAIL forge.<case> (FRG-6): …`）。

| suite | 規則 | 重點 |
|---|---|---|
| Driver | `DRV-1`–`9` | 閒置送達 Sent／Confirmed；最後 `TurnCompleted`；同一個訊息 id 只一個 turn；從任何 cursor 補回之後的**全部**事件、不含舊的、讀了不消耗，daemon 重啟後的新 driver 也一樣 |
| Forge | `FRG-1`–`9` | change id 可以是空的（local）；整串 head 相等才 merge（空字串、短 SHA 不算）；比的是 merge 當下的 head；不符回真正的 head 且什麼都不變（從 `base_head()` 看） |
| Store | `STO-1`–`12` | CAS 是相等（舊版本、未來版本都衝突）；衝突回報實際版本；版本嚴格遞增；事件依序、依 task 分開；重新開啟後全部還在 |
| Runtime | `RTM-1`–`9` | recover 回的 handle 與 start 的相同；停止要真的停（`is_running()` 從 trait 外看）；daemon 重啟後 holder 還在，新 runtime 找得回、停得掉（D3） |
| Notifier | `NTF-1`–`4` | 欄位原樣、3,000 字多位元組 body 不截斷、不修剪空白、順序不變 |
| Clock | `CLK-1`–`4` | unix 毫秒、UTC（對照 `utc_now_unix_ms()`）、不倒退、不凍結 |
| Runner | `RUN-1`–`9` | 輸出逐位元組、256 KiB 不卡；逾時 2 秒內回報，`sh` 與它啟動的子程序都停掉（標記檔判斷）；在指定目錄跑 |

每條規則至少有一個故意弄壞的實作（mutant），列在 CONTRACTS.md 那一列；`tests/contract_teeth/` 跑全部 mutant，並檢查規則表、case、mutant 三者互相對得上（見 [TESTING.md](TESTING.md)）。接真實作時 fixture 多實作的方法：`ForgeFixture::base_head`、`RuntimeFixture::is_running`、`ClockFixture::utc_now_unix_ms`；`DriverFixture::turn_timeout` 可選（預設 10 秒）。

daemon 重啟的三個 hook：`RuntimeFixture::restart`、`DriverFixture::restart`、`StoreFixture::reopen`，都是 `fn(&self) -> Self`：回傳一個新 fixture，裡面是在**同一份持久狀態**上新建的 trait 物件（真實作：同一個 run 目錄與 holder 程序、同一個 backend、同一個 SQLite 檔）。case 先 drop 舊的 fixture 再建新的，所以舊物件 drop 時做的事（例如殺掉 holder）都會被看到。

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
