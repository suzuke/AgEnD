# 第 2 施工關：agend-testkit（`testkit`）

> **TL;DR**
> - 共用測試基礎設施：每個 trait 的假實作、契約測試、假 daemon、假 agent 程式。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：照「你親自驗收」跑 4 步，填「驗收紀錄」；先看「待你追認」的 11 個決定。

## 狀態

**實作中**（2026-09-25）：自動驗收通過（`feat/gate-02-testkit`，draft PR）；待 fresh-context verifier 與你親自驗收。

## 範圍

- 每個 trait 的假實作
- 契約測試套件（同一套測試跑假實作與真實作）
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

   - [ ] 通過

2. 看契約測試摘要。

   ```bash
   ~/.cargo/bin/cargo xtask accept testkit 2>&1 | grep '^contract'
   ```

   應該看到剛好 7 行：

   ```text
   contract Driver: fake 7/7 pass
   contract Forge: fake 8/8 pass
   contract Store: fake 8/8 pass
   contract Runtime: fake 4/4 pass
   contract Notifier: fake 2/2 pass
   contract Clock: fake 2/2 pass
   contract Runner: fake 5/5 pass
   ```

   真實作那一側在各自的施工關才接上（Store 第 5、agent runtime 第 6、Driver 第 7／12、Runner 與 forge local 第 10、Notifier 與 forge github 第 12）。

   - [ ] 通過

3. 單獨啟動一個假 agent，用 curl 跟它說話。

   ```bash
   ~/.cargo/bin/cargo build -q -p agend-testkit --bins
   sleep 5 | ./target/debug/fake-opencode-serve --port 47123 &
   sleep 1
   curl -s -X POST http://127.0.0.1:47123/session -d '{}'; echo
   curl -s -X POST http://127.0.0.1:47123/session/ses_fake0001/message -d '{"parts":[{"type":"text","text":"hi"}]}'; echo
   wait; echo "exit=$?"
   ```

   應該看到：`fake-opencode-serve listening on http://127.0.0.1:47123`、`{"id":"ses_fake0001"}`、一行含 `"text":"fake reply: hi"` 的 JSON；約 5 秒後 `sleep` 結束、stdin 關閉，印出 `exit=0`。port 47123 被占用就換一個數字（三處一起換）。

   - [ ] 通過

4. 故意弄壞：讓假 Forge 在 head 不符時回報錯的 head。

   ```bash
   perl -pi -e 's/actual_head: current,/actual_head: request.expected_head.clone(),/' crates/agend-testkit/src/fakes/forge.rs
   ~/.cargo/bin/cargo test -p agend-testkit --test contract_fakes forge 2>&1 | grep -E 'contract Forge|FAIL forge|test result'
   ```

   應該看到：

   ```text
   contract Forge: fake 7/8 pass, 1 FAIL
     FAIL forge.merge_with_stale_head_echoes_actual_head: expected HeadChanged { actual_head: "0000000000000000000000000000000000000002" }, got HeadChanged { actual_head: "0000000000000000000000000000000000000001" }
   test result: FAILED. 0 passed; 1 failed; ...
   ```

   還原並確認回到全綠：

   ```bash
   git checkout -- crates/agend-testkit/src/fakes/forge.rs
   ~/.cargo/bin/cargo test -p agend-testkit --test contract_fakes forge 2>&1 | grep 'test result'
   ```

   應該看到 `test result: ok. 1 passed; 0 failed; ...`，且 `git status --short` 沒有輸出。

   - [ ] 通過

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
| A10 | Runner 逾時 case：`sleep 3; touch timed-out-command-finished` 以 200 ms 逾時，要在 2 秒內回報，約 4.5 秒時標記檔不能存在（用標記檔判斷指令被停掉，不送 signal 探測） | 餘裕大（2 秒是逾時的 10 倍）不易 flake；代價是每跑一次 Runner suite 多約 4.5 秒 | 調常數：`contract/runner.rs` 開頭 |
| A11 | 假 claude 的 transcript 寫在專案內 `.claude/fake-transcripts/`；假 daemon 在 hello 之前收到無效 JSON 也回 `hello_required` 並關閉（照 README） | 測試的暫存專案 drop 時一起刪掉，不再共用 `<tmp>/fake-claude/`；文件與程式一致 | 改路徑或改回 `invalid_request`：各一處 |

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-25 修 verifier r1（REFUTED @ e7650cd）：Forge 契約用 `base_head()` 看 merge 是否真的發生（HIGH）；Store 連續寫入版本嚴格遞增；Runner 逾時有時間上限並確認指令被停掉；假 daemon drop 關閉已開連線；新增 `tests/fake_knobs.rs`（`fail_next`／`calls()`／`merges()`，31 個手動 mutation 全抓到）；假 claude transcript 移進專案、假 codex 短 socket 不留目錄；hello 前無效 JSON 回 `hello_required`。故意弄壞的包裝測試 8 → 12（`feat/gate-02-testkit`，draft PR #108）。
- 2026-09-25 實作完成、自動驗收通過：7 個假實作與 7 個契約 suite（36 條，另有 8 個故意弄壞的包裝測試）、假 daemon、3 個假 agent 程式、`accept testkit` demo；fmt、workspace clippy、workspace test、check-deps、accept 通過；待 verifier 與你親自驗收（`feat/gate-02-testkit`，draft PR）。

## 下一步

```bash
~/.cargo/bin/cargo xtask accept testkit
cat crates/agend-testkit/README.md
```
