# 第 2 施工關：agend-testkit（`testkit`）

> **TL;DR**
> - 共用測試基礎設施：每個 trait 的假實作、契約測試、假 daemon、假 agent 程式。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：照「你親自驗收」跑 5 步，填「驗收紀錄」；先看「待你追認」的 25 個決定（A20–A23 是 verifier r3 那一輪加的：daemon 重啟 hook、訊息 id 冪等、FRG-3 改寫與一個 core 型別提案；A24 是 verifier r4：重啟類規則改走 daemon 生命週期；A25 是 verifier r5 這一輪：fixture 改成從持久狀態開機、多一次閒置開機，重啟／持久化 case 在第 5、6 施工關要跨真的 process 跑）。

## 狀態

**完成**（2026-09-25）：#108 已 merge（`6d7b540`），verifier r6 CONFIRMED，使用者親自驗收通過。「待你追認」A1–A25 另外確認。

## 範圍

- 每個 trait 的假實作
- 契約測試套件（同一套測試跑假實作與真實作）；規則編號表 [CONTRACTS.md](../../crates/agend-testkit/CONTRACTS.md)，每條規則至少一個 case、一個故意弄壞的實作（mutant）
- 假 daemon（protocol v1）
- 假 agent：假 codex app-server、假 opencode serve、帶 hook 的假 claude

## 自動驗收（完成定義）

- [x] `~/.cargo/bin/cargo test -p agend-testkit` 單獨通過（2026-09-25）
- [x] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨（2026-09-25）
- [x] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）（2026-09-25）
- [x] `~/.cargo/bin/cargo xtask accept testkit` 通過，並印出下方「你親自驗收」用到的 demo（2026-09-25）
- [x] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。所有指令都在 repo 根目錄跑。

1. 跑 demo。

   ```bash
   ~/.cargo/bin/cargo xtask accept testkit
   ```

   應該看到（約 1 分鐘，第一次要編譯）：

   - `== fake codex app-server ==`：`-> turn/start ...`，之後 `<- turn/started`、兩個 `<- item/completed`（`userMessage`、`"text":"fake reply: say hello"`）、`<- turn/completed {... "status":"completed" ...}`，最後 `fake-codex-app-server exited cleanly (exit status: 0)`。
   - `== fake opencode serve ==`：`<- 204`，接著 8 個 `<- event`（從 `server.connected` 到 `session.idle`），最後 `fake-opencode-serve exited cleanly (exit status: 0)`。
   - `== fake claude with hooks ==`：`> <channel source="agend" ...>`、`Stop hook error: queued message from dev-2: ...`、`Interrupted · What should Claude do instead?`、`fake-claude exited cleanly (exit status: 0)`；hook 清單依序是 `SessionStart source=startup`、`UserPromptSubmit ...`、`Stop stop_hook_active=false`、`Stop stop_hook_active=true`、`UserPromptSubmit prompt="long task"`（Esc 之後**沒有** Stop）。
   - `== fake daemon (client protocol v1) ==`：attempt 1 的 `done` 得到 `"code":"stale_result"`，attempt 2 得到 `"result":"accepted"`。
   - 最後兩行：`testkit demo: all fake agents exited cleanly; all contract suites pass` 與 `gate 2 (testkit): checks passed`。

   **這步在驗什麼**：三個假 agent 程式真的以子程序啟動、走真的 socket／HTTP 說完一輪話並正常結束，假 daemon 擋掉過期的結果。壞了的話，後面接 codex／opencode／claude driver 的施工關就沒有可重現的對手可以測。

   - [x] 通過

2. 看契約測試摘要。

   ```bash
   ~/.cargo/bin/cargo xtask accept testkit 2>&1 | grep '^contract'
   ```

   應該看到剛好 7 行（總數 = [CONTRACTS.md](../../crates/agend-testkit/CONTRACTS.md) 各 trait 的 case 數，Forge 的 FRG-5、Driver 的 DRV-6、Store 的 STO-4 各有 2 個 case）：

   ```text
   contract Driver: fake 10/10 pass
   contract Forge: fake 10/10 pass
   contract Store: fake 13/13 pass
   contract Runtime: fake 9/9 pass
   contract Notifier: fake 4/4 pass
   contract Clock: fake 4/4 pass
   contract Runner: fake 9/9 pass
   ```

   真實作那一側在各自的施工關才接上（Store 第 5、agent runtime 第 6、Driver 第 7／12、Runner 與 forge local 第 10、Notifier 與 forge github 第 12）。

   **這步在驗什麼**：7 個假實作都守住規則表上的每一條。壞了的話，用假實作測的上層程式（流水線、daemon）拿到的是真實作不會有的行為，測試綠了也不代表接上真的會對（v1 #1483）。

   - [x] 通過

3. 單獨啟動一個假 agent，用 curl 跟它說話。

   ```bash
   ~/.cargo/bin/cargo build -q -p agend-testkit --bins
   sleep 5 | ./target/debug/fake-opencode-serve --port 47123 &
   sleep 1
   curl -s -X POST http://127.0.0.1:47123/session -d '{}'; echo
   BODY='{"parts":[{"type":"text","text":"hi"}]}'
   curl -s -X POST http://127.0.0.1:47123/session/ses_fake0001/message -d "$BODY"; echo
   wait; echo "exit=$?"
   ```

   應該看到：`fake-opencode-serve listening on http://127.0.0.1:47123`、`{"id":"ses_fake0001"}`、一行含 `"text":"fake reply: hi"` 的 JSON；約 5 秒後 `sleep` 結束、stdin 關閉，印出 `exit=0`。port 47123 被占用就換一個數字（三處一起換）。

   **這步在驗什麼**：假 agent 在測試框架之外也能單獨用，講的是真的 HTTP，stdin 關了就乾淨結束。壞了的話，別的 crate 的測試（或你手動除錯）沒辦法拿它當真的 backend 用，行程也可能留著不結束。

   - [x] 通過

4. 故意弄壞：讓假 Forge 在 head 不符時回報錯的 head。

   ```bash
   perl -pi -e 's/actual_head: current,/actual_head: request.expected_head.clone(),/' crates/agend-testkit/src/fakes/forge.rs
   ~/.cargo/bin/cargo test -p agend-testkit --test contract_fakes forge 2>&1 | grep -E 'contract Forge|FAIL forge|test result'
   ```

   應該看到：

   ```text
   contract Forge: fake 8/10 pass, 2 FAIL
     FAIL forge.merge_with_stale_head_echoes_actual_head (FRG-6): expected HeadChanged { actual_head: "0000000000000000000000000000000000000002" }, got HeadChanged { actual_head: "0000000000000000000000000000000000000001" }
     FAIL forge.merge_needs_the_whole_head (FRG-8): expected_head "" (head 0000000000000000000000000000000000000001): expected HeadChanged { actual_head: "0000000000000000000000000000000000000001" }, got HeadChanged { actual_head: "" }
   test result: FAILED. 0 passed; 1 failed; ...
   ```

   還原並確認回到全綠：

   ```bash
   git checkout -- crates/agend-testkit/src/fakes/forge.rs
   ~/.cargo/bin/cargo test -p agend-testkit --test contract_fakes forge 2>&1 | grep 'test result'
   ```

   應該看到 `test result: ok. 1 passed; 0 failed; ...`，且 `git status --short` 沒有輸出。

   **這步在驗什麼**：契約 suite 真的抓得到假實作漂移，而且失敗訊息指出是哪一條規則（FRG-6、FRG-8）。壞了的話，假實作可以悄悄偏離規則而測試照樣全綠，正是這個施工關要防的事。

   - [x] 通過

5. 看規則覆蓋：每條規則都有 mutant，而且都被抓到。

   ```bash
   ~/.cargo/bin/cargo test -p agend-testkit --test contract_teeth -- --nocapture 2>&1 \
     | grep -E '^mutant (SkipsFirstAfterCursor|AcceptsFutureVersions|RealKillsShOnly|DaemonScoped|Handoff|RewriteOnChange|SharedMem) |test result'
   ```

   應該看到（約 20 秒；毫秒數會不同）：

   ```text
   mutant AcceptsFutureVersions (STO-6): failing rules ["STO-6", "STO-7"] in 0 ms
   mutant SharedMem (STO-12): failing rules ["STO-4", "STO-12"] in 0 ms
   mutant Handoff (RTM-8): failing rules ["RTM-8"] in 0 ms
   mutant DaemonScoped (RTM-8): failing rules ["RTM-8", "RTM-9"] in 0 ms
   mutant RewriteOnChange (RTM-8): failing rules ["RTM-8"] in 0 ms
   mutant SkipsFirstAfterCursor (DRV-6): failing rules ["DRV-6", "DRV-6"] in 612 ms
   mutant RealKillsShOnly (RUN-8): failing rules ["RUN-8"] in 9485 ms
   test result: ok. 4 passed; 0 failed; ...
   ```

   七行 `mutant` 的順序可能不同；每行括號裡的規則都出現在它的 `failing rules` 裡（同一條規則有兩個 case 都失敗就出現兩次）。`DaemonScoped` 是 v1 的行為：daemon 結束時把自己啟動的 holder 一起殺掉，新的 daemon 只認得自己啟動的。`Handoff` 撐得過一次重啟、撐不過第二次：recover 把持久的 holder 登記讀走刪掉，只記在自己的物件裡，第三個 daemon 就找不回 holder。`RewriteOnChange` 在打開時把 holder 登記清空、只在啟動或停止 holder 時寫回：只做復原的閒置開機之後，下一個 daemon 就找不回 holder。`SharedMem` 是記憶體裡的 DB，只要還有一個 handle 開著資料就在：契約先把 case 自己的 fixture 丟掉，兩次開機之間沒有 handle 開著，資料就不見了。

   **這步在驗什麼**：規則表上每條規則都有一個故意弄壞的實作，而且 suite 在標著那條規則的 case 上失敗；覆蓋測試另外確認表、case、mutant 三者對得上。壞了的話，就回到之前的打地鼠：某條規則其實沒人驗，要等 verifier 一條條找出來。

   - [x] 通過

## 待你追認

owner 睡著時由實作者決定、可以反悔的事。每項：決定 · 理由 · 反悔的成本。不同意就在這裡打叉並寫原因。

| # | 決定 | 理由 | 反悔成本 |
|---|---|---|---|
| A1 | 契約 suite 寫成「case 清單 + fixture trait」，由 `block_on` 在呼叫端執行緒驅動；需要 tokio 的真實作在 fixture 裡進 runtime | 不讓 testkit 依賴 async runtime；同一份 case 可跑假與真 | 改成 async case：改 `contract.rs` 與 7 個 suite 的呼叫方式 |
| A2 | 契約只釘文件寫明的規則；不確定的（重複 submit、停止未知 holder、未知 cursor、送達一定被確認、Store 起始版本）不釘，假實作的選擇寫在 doc comment | 釘錯了會逼真實作去配合假實作，正好是 #1483 的反方向 | 之後有證據再加 case |
| A3 | 加依賴 `serde_json` 與 `tungstenite`（`default-features = false`、`handshake`）；HTTP／SSE 用 std 自己寫 | codex 的傳輸是 WebSocket，自己寫 frame 解析容易和真 client 不相容；HTTP 子集很小 | 換成手寫 WebSocket 或別的 crate：只動 `fake_agent/codex.rs` |
| A4 | 假 daemon 除 `stale_result` 外的錯誤碼（`hello_required`、`version_mismatch`、`invalid_request`、`unknown_request`、`unknown_ask`）是假 daemon 自己定的常數 | core 目前只定了 `STALE_RESULT`；第 8 施工關定案時要搬進 `agend_core::protocol::client` 並讓假 daemon 改用 | 改常數名：假 daemon 與它的測試 |
| A5 | 假 agent 以「stdin 結束」為正常結束訊號（exit 0），codex 假 server 收掉 socket 與 symlink | 測試與 demo 不必送 signal；holder 裡 stdin 是 PTY，不會意外結束 | 改成 SIGTERM：三個 `main` |
| A6 | 假 codex 的 approval 由 prompt 前綴 `run: ` 觸發；假 claude 在忙碌時收到的 channel 訊息一律不處理（spike C1 的最壞情況） | 需要可重現的觸發點；最壞情況逼 driver 走 Stop hook 排隊（D16） | 改觸發方式：各自模組內 |
| A7 | 假 agent 欄位只放 spike 紀錄與上表列的最少集合，未用真 schema 逐欄比對；各模組開頭列出涵蓋與未涵蓋 | spike 沒有保存 schema dump；第 7、12 施工關接真 backend 時再比對 | 補欄位：加法，不破壞既有測試 |
| A8 | `git_fixture` 不做（只留說明），假 daemon 與假 agent 只支援 unix；本頁狀態寫「實作中」而不是「驗收中」 | 不在本施工關範圍；CI 只有 ubuntu／macOS；比照第 1 施工關，verifier 前仍是實作中 | 第 3 施工關需要時加 git fixture |
| A9 | `ForgeFixture` 多一個 `base_head()`：契約從 trait 外面讀 base branch 的 head，確認 merge 讓 base 移到 merge commit、head 不對時 base 不動 | 只看 work branch 的 head 抓不到「回報 `HeadChanged` 卻已經 merge」（verifier r1 HIGH）；trait 本身沒有讀 base 的方法 | 改成別的觀察方式（例如列出 merge 紀錄）：改 `contract/forge.rs` 與各 fixture |
| A10 | Runner 逾時 case：`sleep 3; touch timed-out-command-finished` 以 200 ms 逾時，要在 2 秒內回報，約 4.5 秒時標記檔不能存在（用標記檔判斷指令被停掉，不送 signal 探測） | 餘裕大（2 秒是逾時的 10 倍）不易 flake；代價是每跑一次 Runner suite 多約 4.5 秒（加上 A17 共約 9 秒） | 調常數：`contract/runner.rs` 開頭 |
| A11 | 假 claude 的 transcript 寫在專案內 `.claude/fake-transcripts/`；假 daemon 在 hello 之前收到無效 JSON 也回 `hello_required` 並關閉（照 README） | 測試的暫存專案 drop 時一起刪掉，不再共用 `<tmp>/fake-claude/`；文件與程式一致 | 改路徑或改回 `invalid_request`：各一處 |
| A12 | 契約規則寫成編號表 [CONTRACTS.md](../../crates/agend-testkit/CONTRACTS.md)（52 條），mutant 名寫在每一列；`tests/contract_teeth/` 的覆蓋測試比對表、case（`Case::rule`）、註冊的 mutant 三者，mutant 測試只要求「至少一個標著該規則的 case 失敗」 | 打地鼠的根因是沒有完整規則清單；名字寫在表上，刪掉任何一個 mutant 覆蓋測試就失敗；一個 bug 常同時違反幾條規則，要求「只有那條失敗」會很脆 | 換成別的格式：改表與 `main.rs` 的解析函式 |
| A13 | 新增 NTF-3：title／body 前後空白與空行不修剪 | 「推送帶完整內容」的延伸；verifier r2 的 N1（trim）要被擋。風險：Telegram 伺服器本身會去掉前後空白，真 Notifier 的 fixture 要讀送出的請求，不是聊天室 | 刪掉這條：CONTRACTS.md 一列、`notifier.rs` 一個 case、一個 mutant |
| A14 | 新增 NTF-4：通知依送出順序到達 | 第 2 施工關原有的 case，文件沒寫 | 同上 |
| A15 | 新增 CLK-2：連續讀取不倒退 | 第 2 施工關原有，文件沒寫；系統時鐘被校時可能倒退，真 Clock（第 5／10 施工關）若直接讀系統時鐘可能要夾住上一個值 | 刪掉或改成「允許倒退 N 毫秒」：`clock.rs` |
| A16 | 新增 RUN-4：stdout、stderr 各 256 KiB 要完整回來、不卡住（5 秒逾時） | 由「輸出不變」延伸；verifier r2 的 RN3（結束後才讀管線）在 200 KB 會假逾時 | 改大小或逾時：`runner.rs` 常數 |
| A17 | 新增 RUN-8：逾時時整個 process group 一起停掉，用子 `sh` 寫標記檔來判斷（`sh -c 'sleep 3; touch timed-out-child-finished'; true`）；不處理 `setsid` 脫離的程序 | owner 2026-09-25 要求；Runner suite 因此多約 4.5 秒 | 改成「所有子孫」（含 setsid）：真 runner 要改用 cgroup／job object，case 不變 |
| A18 | fixture 多三個方法：`RuntimeFixture::is_running`（從 trait 外看 holder 是否活著）、`ClockFixture::utc_now_unix_ms`（外部 UTC 參考）、`DriverFixture::turn_timeout`（有預設 10 秒） | 「stop 只是藏起來」「時區差」只有從 trait 外面才看得到，同 A9 的 `base_head()`；turn_timeout 讓假實作包裝的 mutant 不用等 10 秒 | 各自一個方法，接真實作前可改名 |
| A19 | `tests/contract_teeth/real_runner.rs`：測試裡的真 `sh` runner（`process_group(0)`，逾時只 `kill -s KILL -- -<子程序 pid>`），全部正確時通過整個 Runner 契約，也當 RUN-4／8／9 的 mutant 底座；不放進 library | 證明新規則真實作做得到；runner 實作屬第 10 施工關，testkit 不放 production 邏輯 | 第 10 施工關的真 runner 接上後，可改用它的旋鈕版本 |
| A20 | daemon 重啟寫成 fixture hook：`RuntimeFixture::restart`、`DriverFixture::restart`、`StoreFixture::reopen`，都是 `fn(&self) -> Self`（在同一份持久狀態上建新的 trait 物件）；case 先 drop 舊的再建新的。新規則 RTM-8、RTM-9、STO-12，DRV-6 多一個重啟 case。假實作把「活過重啟」的狀態（holder 表、backend 事件、store 資料）放在共用的 `Arc` 裡，`calls()` 每個物件各自一份 | verifier r3 M1（HIGH）、M3、M4：D3 的「重啟不斷線」原本只在同一個物件裡驗，`DaemonScoped` 能通過；`&self` 讓 case 可以依序建多個 daemon、明確 drop 舊的 | 改成別的形狀（例如 factory）：三個 fixture trait、三個假實作的 hook、6 個 case |
| A21 | 新增 DRV-9：同一個訊息 id 送第二次，`deliver` 不是錯誤，也不多一個 turn；去重放在 driver（`FakeDriver` 改成照 id 去重）。case 等滿 `turn_timeout` 看有沒有第二個 `TurnCompleted`，所以 `FakeDriverFixture::turn_timeout` 設 200 ms（假 driver 在 `deliver` 裡就完成 turn） | verifier r3 M2；delivery.md「以 id 冪等，只有一套去重」。「第二次不是錯誤」是冪等的一般意思，文件沒逐字寫。真 driver 在第 7 施工關每跑一次多等一個 `turn_timeout` | 去重改放 daemon：刪 DRV-9、`FakeDriver` 的 `delivered` 集合，「不釘」寫「去重在 daemon」 |
| A22 | FRG-3 改寫：change id 可以是空字串（forge local 沒有 change id），兩個不同 branch 的 change 都有 id 時 id 不同；mutant `SubmitWithoutId` 改成必須通過的測試 `forge_without_change_ids_passes`，新 mutant `SameIdForEveryChange`。**提案（未做）**：`agend_core::traits::SubmittedChange.id` 改成 `Option<String>`，讓「沒有 change id」在型別上看得到，和 `PipelineEvent::Submitted { change_id: Option<_> }` 一致 | verifier r3 F1：舊規則逼 local forge 生出一個 id，和 GLOSSARY、pipeline.md、`state.rs` 可完成證明相反。core 型別不在本輪範圍，所以先用空字串 | 採納提案：改 core 型別、`FakeForge`、FRG-3 的 case 與 mutant（約 20 行） |
| A23 | 「merge 落在目前的 base 上、不弄丟之前的 merge」只寫成 CONTRACTS.md Forge 段的「建議，未釘」 | 文件只有 CAS `update-ref` 機制、沒有逐字規則；`ForgeFixture` 看不到 ancestry | 要釘：加一個讀 base 歷史的 fixture 方法、一條規則、一個 mutant |
| A25 | fixture 改成從持久狀態開機：`RuntimeFixture`、`DriverFixture`、`StoreFixture` 的 `restart`／`reopen` 換成 `type Persisted` + `persisted(&self)` + `boot(&Persisted) -> Self`；`RuntimeFixture::is_running` 與 `DriverFixture::emit_while_down` 只拿 `Persisted`；`Case::check` 拿 fixture by value，重啟類 case 先把它轉成持久狀態再 drop。生命週期變成 4 次開機：做事 → 閒置（只復原）→ 做事 → 檢查，兩次開機之間沒有實例活著、確認 holder 還在跑。DRV-9 每次做事的開機送兩個 id，之後重送全部（舊的先）；DRV-6 重啟後也從每個較舊的 cursor 補回。假實作多三個持久狀態型別：`FakeHolders`、`FakeBackend`、`FakeStoreFile`。verifier r5 的七個實作註冊成 mutant，另加 `FrozenRegistry`、`FrozenDatabase`（重啟後寫的沒存下來，讓開機 4 有牙齒）；共 80 個 mutant。CONTRACTS.md 與第 5、6 施工關頁面加一條：重啟／持久化類的契約 case 要對真實作、跨真的 process 重啟跑（分開的 process、真的檔案／DB） | verifier r5：每次開機都做事，打開時清空、只在變動時寫回的實作（`RewriteOnChange`、`TruncOnOpen`）能通過；case 自己的 fixture 活過整個生命週期，只要還有一個實例活著就不丟資料的實作（`SharedMem`、`LiveJournal`、`LastOneOut`）能通過；DRV-9 只用一個 id、DRV-6 只從最新的 cursor 補回（`LastIdDedup`、`ReadAck`）。同一個 process 裡的契約分不出「真的存下來」與「藏在 process 裡」，所以更嚴的持久化檢查放到有真檔案、真 DB、真 process 的第 5、6 施工關。代價：DRV-9 真 driver 每跑一次等 3 次 `turn_timeout`（預設各 10 秒）；DRV-6 真 driver 多一些 `events` 讀取 | 回到 `fn(&self) -> Self`：三個 fixture trait、三個假實作、case 的 `check` 型別；拿掉閒置開機：`contract.rs` 的 `LIFECYCLE` 一行（`RewriteOnChange`、`TruncOnOpen` 會通過） |
| A24 | 重啟類規則（DRV-6、DRV-9、STO-4、STO-12、RTM-8、RTM-9）改走同一個 daemon 生命週期 `contract::daemon_lifecycle`：3 次開機，每次先做開機復原（`recover_holders`；從上一個 daemon 最後的 cursor 補回事件；打開 store）、檢查不變量、再做事，兩次開機之間 backend 照常動。`DriverFixture` 多一個必須實作的 `emit_while_down`（daemon 不在時 agent 自己跑完一輪，不經過 driver）；STO-4 多一個跨重新開啟的 case。CONTRACTS.md 寫明 fixture 要求：真實作的重啟要走真的持久層（DB 檔、run 目錄與 socket），不用 process 內的全域 `static` | verifier r4：r3 的重啟 case 只重啟一次、兩次開機之間沒事發生，狀態只在 Rust 物件裡（`CounterStore`、`ObjDedup`）、第一次讀就消耗（`Handoff`）、丟掉斷線期間的事件（`Gap`）的實作都能通過。一個共用 helper 讓每條重啟類規則得到同樣的處理，之後加規則不會再漏。代價：DRV-9 case 要等 3 次 `turn_timeout`（真 driver 預設 10 秒，每跑一次約 30 秒） | 改開機次數：`contract.rs` 的 `BOOTS`；拿掉 `emit_while_down`：一個 fixture 方法、DRV-6 case 的一段檢查、`Gap` mutant |

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
| 2026-09-25 | 通過 | 使用者在 `v2`（`5035b2e`）跑完步驟 1–5。步驟 1、2 使用者自行比對；步驟 3–5 的輸出由 agent 逐項比對相符（第 4 步還原後 `git status` 乾淨）。步驟 3 原本的長 curl 行複製時被截斷，已改成先存 `BODY` 變數。「待你追認」A1–A25 另外確認。 |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-25 使用者親自驗收步驟 1–5 通過，狀態改為完成；步驟 3 的 curl 改成先存變數，避免複製時截斷。
- 2026-09-25 verifier r6 CONFIRMED（`00fbf45`）；#108 squash merge 為 `6d7b540`；狀態改為驗收中。
- 2026-09-25 修 verifier r5（REFUTED @ 6d471e8，owner 核准的最後一輪）：生命週期多一次閒置開機（做事 → 閒置 → 做事 → 檢查），fixture 改成從持久狀態開機（`Persisted` + `boot`），case 自己的 fixture 在第一次開機前就 drop，daemon 不在時的 backend 動作只經過持久狀態；DRV-9 重啟前送兩個 id、重啟後先重送舊的，DRV-6 重啟後也從較舊的 cursor 補回；r5 的 `RewriteOnChange`、`TruncOnOpen`、`LastIdDedup`、`ReadAck`、`SharedMem`、`LiveJournal`、`LastOneOut` 與新的 `FrozenRegistry`、`FrozenDatabase` 註冊為 mutant（共 56 條規則、80 個 mutant）；helper mutation H1–H8（H6、H8 改成讀 case 的 fixture 就編譯不過）與新的 H9–H13 全部抓到；CONTRACTS.md、第 5、6 施工關頁面加「重啟／持久化 case 跨真的 process 重啟跑」（待你追認）（`feat/gate-02-testkit`，draft PR #108）。
- 2026-09-25 修 verifier r4（REFUTED @ ea61cba）：重啟類 case 只重啟一次，狀態只在物件裡或第一次讀就消耗的實作能通過。改成共用的 daemon 生命週期 helper（3 次開機、每次先復原、兩次開機之間 backend 照常動，新 fixture 方法 `DriverFixture::emit_while_down`），DRV-6／DRV-9／STO-4／STO-12／RTM-8／RTM-9 都走它；r4 的 `Handoff`（RTM-8）、`CounterStore`（STO-4）、`ObjDedup`（DRV-9）、`Gap`（DRV-6）註冊為 mutant；CONTRACTS.md 寫明 fixture 要走真的持久層；FRG-4、FRG-9、DRV-9 的推得標出來源，DRV-6 改引 ARCHITECTURE 程序模型規則 2；共 56 條規則、71 個 mutant（`feat/gate-02-testkit`，draft PR #108）。
- 2026-09-25 修 verifier r3（REFUTED @ e251dff）：加 daemon 重啟 hook（`RuntimeFixture::restart`、`DriverFixture::restart`、`StoreFixture::reopen`），新規則 RTM-8／RTM-9（D3 重啟不斷線，擋 `DaemonScoped`）、STO-12（重新開啟後資料與版本還在）、DRV-9（訊息 id 冪等，`FakeDriver` 改成去重），DRV-6 多重啟補回 case；FRG-3 改成允許空 change id（local）；推得的規則標出來源；共 56 條規則、67 個 mutant（`feat/gate-02-testkit`，draft PR #108）。
- 2026-09-25 修 verifier r2（REFUTED @ 2a7f2f7）：契約改成編號規則表 CONTRACTS.md（52 條，5 條待追認），每個 case 標規則編號，每條規則至少一個 mutant（61 個，含 r2 的 D1、D2、S1–S3、F1、F4、R1、R2、N1–N3、C1、C2、RN1–RN3），覆蓋測試比對表／case／mutant；新增 fixture 方法 `is_running`、`utc_now_unix_ms`、`turn_timeout`；測試裡的真 `sh` runner（process group）通過整個 Runner 契約；「你親自驗收」加第 5 步與每步「這步在驗什麼」（`feat/gate-02-testkit`，draft PR #108）。
- 2026-09-25 修 verifier r1（REFUTED @ e7650cd）：Forge 契約用 `base_head()` 看 merge 是否真的發生（HIGH）；Store 連續寫入版本嚴格遞增；Runner 逾時有時間上限並確認指令被停掉；假 daemon drop 關閉已開連線；新增 `tests/fake_knobs.rs`（`fail_next`／`calls()`／`merges()`，31 個手動 mutation 全抓到）；假 claude transcript 移進專案、假 codex 短 socket 不留目錄；hello 前無效 JSON 回 `hello_required`。故意弄壞的包裝測試 8 → 12（`feat/gate-02-testkit`，draft PR #108）。
- 2026-09-25 實作完成、自動驗收通過：7 個假實作與 7 個契約 suite（36 條，另有 8 個故意弄壞的包裝測試）、假 daemon、3 個假 agent 程式、`accept testkit` demo；fmt、workspace clippy、workspace test、check-deps、accept 通過；待 verifier 與你親自驗收（`feat/gate-02-testkit`，draft PR）。

## 下一步

```bash
~/.cargo/bin/cargo xtask accept testkit
cat crates/agend-testkit/README.md
```
