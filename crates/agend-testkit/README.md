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
| 暫存 git repo | `git_fixture` | `GitFixture`：canonical repo、bare team origin、linked worktree 與 branch，全部在一個新的暫存目錄裡；見下方「git fixture」 |

## 不負責

- production 邏輯；呼叫真 backend 或模型
- binding 快照 fixture（第 3 施工關需要時加）

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
| `FakeDriver` | `with_instance`／`add_instance`；預設每次送達自動產生 busy → confirmed → turn completed → idle 事件，同一個訊息 id 再送只回回條、不再產生 turn；`set_auto_turn(false)` 後用 `push_event`；`next_receipt`；`backend()` 取出 `FakeBackend`（假的 agent 那一側，不是 driver；`push_event` 讓 agent 自己產生事件），`FakeDriver::connect(&backend)`（daemon 重啟：同一個 backend 上的新 driver，事件與 cursor 都在） |
| `FakeForge` | `push(branch)` 加一個 commit（新 branch 從目前的 base 開出）；`merges()`；`base_head()`（base branch 的 head：最後一個 merge commit，還沒 merge 時是 `BASE_ROOT`）；`base_contains(commit)`（commit 是 base head 或它的祖先）；`set_base(commit)`（像 force push 把 base 移到某個 commit）；change id 是 `Some("change-<n>")` |
| `FakeStore` | `insert_workflow`、`events(task)`；`file()` 取出 `FakeStoreFile`（假的 DB 檔，不是 store），`FakeStore::open(&file)`（daemon 重啟：同一份資料上的新 handle） |
| `FakeRuntime` | `crash(id)`（holder 自己死掉）、`adopt(handle)`（外來的 holder）、`running()`；`holders()` 取出 `FakeHolders`（假的 holder 程序，不是 runtime），`FakeRuntime::on(&holders)`（daemon 重啟：同一張 holder 表上的新 runtime） |
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
| Driver | `DRV-1`–`9` | 閒置送達 Sent／Confirmed；最後 `TurnCompleted`；同一個訊息 id 只一個 turn，重啟後再送也一樣；從任何 cursor 補回之後的**全部**事件、不含舊的、讀了不消耗，每次 daemon 重啟後的新 driver 也一樣（也從較舊的 cursor），含 daemon 不在時的事件 |
| Forge | `FRG-1`–`10` | 沒有 change id 是 `None`（local）；整串 head 相等才 merge（空字串、短 SHA 不算）；比的是 merge 當下的 head；不符回真正的 head 且什麼都不變（從 `base_head()` 看）；merge 落在目前的 base 上、不弄丟之前的 merge（從 `base_contains()` 看） |
| Store | `STO-1`–`12` | CAS 是相等（舊版本、未來版本都衝突）；衝突回報實際版本；版本嚴格遞增，跨重新開啟也是；事件依序、依 task 分開；每次重新開啟後全部還在 |
| Runtime | `RTM-1`–`9` | recover 回的 handle 與 start 的相同；停止要真的停（`is_running()` 從 trait 外看）；每次 daemon 重啟後 holder 都還在，新 runtime 找得回、停得掉（D3） |
| Notifier | `NTF-1`–`4` | 欄位原樣、3,000 字多位元組 body 不截斷、不修剪空白、順序不變 |
| Clock | `CLK-1`–`4` | unix 毫秒、UTC（對照 `utc_now_unix_ms()`）、不倒退、不凍結 |
| Runner | `RUN-1`–`9` | 輸出逐位元組、256 KiB 不卡；逾時 2 秒內回報，`sh` 與它啟動的子程序都停掉（標記檔判斷）；在指定目錄跑 |

每條規則至少有一個故意弄壞的實作（mutant），列在 CONTRACTS.md 那一列；`tests/contract_teeth/` 跑全部 mutant，並檢查規則表、case、mutant 三者互相對得上（見 [TESTING.md](TESTING.md)）。接真實作時 fixture 多實作的方法：`ForgeFixture::base_head`、`ForgeFixture::base_contains`（例如 `git merge-base --is-ancestor`）、`RuntimeFixture::is_running`（只拿持久狀態）、`ClockFixture::utc_now_unix_ms`；`DriverFixture::turn_timeout` 可選（預設 10 秒）。

daemon 重啟：`RuntimeFixture`、`DriverFixture`、`StoreFixture` 各有一個 `Persisted` 型別（持久狀態：真實作是 run 目錄與 holder 程序、backend、SQLite 檔；不是 trait 物件，也不讓任何 trait 物件活著）、`persisted(&self)` 與 `boot(&Persisted) -> Self`（從持久狀態開機一個新 fixture）。`DriverFixture::emit_while_down(&Persisted)`：daemon 不在時讓 agent 自己跑完一輪（不經過 driver）。契約 case 拿到的 fixture 是 by value，重啟類 case 先把它轉成持久狀態再 drop。

重啟類 case 都走同一個 daemon 生命週期（`contract::daemon_lifecycle`：做事 → 閒置 → 做事 → 檢查 4 次開機，每次先復原，兩次開機之間沒有實例活著、backend 照常動），細節與 fixture 要求（走真的持久層，不用全域 `static` 或記憶體 DB；第 5、6 施工關要跨真的 process 重啟跑）見 [CONTRACTS.md](CONTRACTS.md)。

## git fixture

`GitFixture::new(label)` 在 `<tmp>/agend-test-git-<label>-*` 建出（`label` 含 `:` 或 `;`、或暫存目錄含 `:`（Windows 是 `;`）時回 `InvalidInput`，什麼都不建：git 拿這個字元切 `GIT_CEILING_DIRECTORIES`，路徑含了就等於沒有 ceiling）：

| 路徑 | 內容 |
|---|---|
| `canonical()` | repo，`main` 上有一個初始 commit，`origin` 指向下面的 bare repo，`main` 已 push |
| `origin()` | bare 的 team origin（`origin.git`） |
| `add_worktree(name, branch, from)` | `worktrees/<name>`：canonical 的 linked worktree，在新 branch 上；`name` 是 `.git`（不分大小寫）就 panic，不建任何東西 |

其他：`branch(name, from)`、`commit(dir, file, message)`（回新的 head）、`rev_parse`、`is_ancestor`、`git(dir, args)`（失敗就 panic 並印 stderr）、`command(program, dir)`（給要自己跑程式的測試，例如第 3 施工關的 shim）。drop 時整個目錄刪掉。

威脅模型：fixture 防的是**善意但會出錯**的測試程式碼：路徑寫錯或是空的、`cd` 失敗、繼承到外面的環境變數。它**不是沙箱**，不防故意改 git 內部檔案來逃出去的測試程式碼（例如透過 `command` 自己寫 `.git/commondir` 或 `core.worktree`）；這不在範圍內。理由跟第 3 施工關的 shim 一樣：要擋的是會發生的失誤，不是對抗性的程式碼；每補一個洞就冒出下一個，擋不完，只會讓 fixture 越來越複雜。只支援 unix（專案的平台）；Windows 的檔名規則（結尾的 `.`、8.3 短檔名）不處理。

衛生規則（每個 command 都套用）：

- **只在自己建的 repo 裡跑**：`dir` 必須是絕對路徑，解析後落在 `canonical()`、`origin()` 或 `add_worktree` 建的 worktree（含子目錄）裡，否則 panic；解析後任何一段是 `.git`（不分大小寫，例如 `canonical/.git`、`canonical/.git/worktrees/<w>`）也 panic，在那裡跑 git 或寫檔等於改 repo 自己的 metadata。`root()`、`root/worktrees` 與 `root()` 下其他目錄都拒絕：它們不是 repo，git 從那裡找 repo 會往上走，所以 discovery 一律從 fixture 的 repo 開始。`GIT_CEILING_DIRECTORIES=<root>` 是第二道防線（repo 的 `.git` 被刪掉時擋住往上找）。
- `git(dir, args)` 的第一個參數必須是子指令；`-C`、`--git-dir`、`--work-tree`、`--namespace`、`-c` 等全域選項一律 panic（repo 用 `dir` 指定）。
- `commit` 的 `dir` 只能是 canonical 或 linked worktree（不收 bare origin）；檔名只能是單純的相對路徑，任何一段是 `.git`（不分大小寫）或 symlink 就 panic，檔案已存在且 hard link 數大於 1 也 panic（unix），寫檔前確認上層目錄解析後仍在 work tree 裡；所有檢查都在寫檔之前，被拒絕的呼叫不留下任何檔案。
- **不檢查的**：子指令後面的路徑參數（例如 `worktree add <path>`），以及 `command(program, dir)` 除了 `dir` 以外的參數；測試要自己只傳 fixture 裡的路徑。
- 一律 `Command::current_dir(<絕對路徑>)`，不用 process 的 cwd；`root()` 已解析 symlink（macOS 的 `/var` → `/private/var`）。
- `GIT_CONFIG_GLOBAL=/dev/null`、`GIT_CONFIG_NOSYSTEM=1`、`GIT_CEILING_DIRECTORIES=<root>`；固定 author／committer 與日期，所以同樣的內容、parent、訊息得到同樣的 commit id（不同 commit 請用不同訊息）。
- 移除繼承來的 `GIT_*`、`AGEND_*`（含 `GIT_DIR`、`GIT_WORK_TREE`、`AGEND_HOME`）。這是 `command()` 當下的快照；之後才在父程序設定的變數，只有固定清單（`GIT_DIR`、`GIT_WORK_TREE`、`GIT_INDEX_FILE`、`GIT_COMMON_DIR`、`GIT_OBJECT_DIRECTORY`、`GIT_ALTERNATE_OBJECT_DIRECTORIES`、`GIT_NAMESPACE`、`GIT_CONFIG`、`GIT_CONFIG_PARAMETERS`、`GIT_CONFIG_COUNT`、`GIT_CONFIG_SYSTEM`、`GIT_DISCOVERY_ACROSS_FILESYSTEM`、`AGEND_HOME`）會被移除，其他的仍會傳給 git。

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
