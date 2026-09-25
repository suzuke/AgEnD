# agend-testkit 測試

> **TL;DR**
> - 測假實作本身、7 個契約 suite（對假實作全過；[CONTRACTS.md](CONTRACTS.md) 的 52 條規則各有 mutant，每個 mutant 都被它那條規則的 case 抓到）、假 daemon、3 個假 agent 程式（以真的子程序跑）。
> - 記住：假 agent 的測試啟動 `src/bin/` 的真 binary，走真的 socket／HTTP；不在行程內呼叫。
> - 下一步：`~/.cargo/bin/cargo test -p agend-testkit`。

## 怎麼跑

```bash
~/.cargo/bin/cargo test -p agend-testkit
~/.cargo/bin/cargo xtask accept testkit     # 另外跑 demo（啟動三個假 agent、假 daemon、契約摘要）
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `fakes::*::tests` | 每個假實作的編排：失敗排隊、回應順序、逾時、crash／adopt、重複 submit、自動 turn 可關 |
| `contract::tests` | 報表格式：每條失敗都寫出 `<trait>.<case>` 與原因；case panic 也算失敗 |
| `executor::tests` | `block_on` 會在 wake 後再 poll |
| `tests/contract_fakes.rs` | 7 個契約 suite 對假實作全部通過；`run_all_fakes` 每個 trait 剛好一次 |
| `tests/contract_teeth/`（`main.rs`） | 覆蓋測試：讀 CONTRACTS.md 的規則表，每條規則至少一個 case、至少一個 mutant；表上列的 mutant 名與註冊的完全一致；case 與 mutant 都不能指向表上沒有的編號；每個前綴 1..n 連號。mutant 測試：61 個 mutant 平行各跑整個 suite，每個都要讓至少一個標著它那條規則的 case 失敗（`--nocapture` 印出每個 mutant 被哪些規則抓到） |
| `tests/contract_teeth/<trait>.rs` | 各 trait 的 mutant：包住假實作、換掉一個方法（例如 backfill 掉第一個事件、CAS 接受未來版本、prefix 比對 head、stop 只是藏起來、body 截在 64 bytes、時鐘凍結或差 8 小時） |
| `tests/contract_teeth/real_runner.rs` | 一個真的 `sh -c` runner（子程序放進自己的新 process group，逾時只對那個 group 送 SIGKILL）：全部旋鈕正確時通過整個 Runner 契約；只殺 `sh`、結束後才讀管線、在別的目錄跑，是 RUN-8、RUN-4、RUN-9 的 mutant |
| `tests/fake_knobs.rs` | 每個假實作（Forge、Driver、Store、Runtime、Runner、Notifier）的每個方法：`fail_next` 只讓下一次失敗、`calls()` 依序記下每次呼叫（含失敗的）；`FakeForge::merges`／`base_head` 只記成功的 merge |
| `tests/fake_daemon.rs` | hello 必須在前（無效 JSON 也回 `hello_required` 並關閉）、hello 之後的無效 JSON 不斷線、drop 時已開的連線讀到 EOF（2 秒內）、major 不合的錯誤訊息、狀態與未知請求、事件身分（沒帶、舊 attempt、別的關卡、重播都 `stale_result`）、backlog 再即時事件、請示回答 |
| `tests/fake_codex.rs` | turn 完成事件帶回 threadId／turnId；steer（錯的 turn id 被拒）、queue 自動出列成新 turn、interrupt；approval 等待決定；只有 resume 過的 thread 才推事件；長路徑 symlink 指到 temp dir 裡的短 socket（不留目錄） |
| `tests/fake_opencode.rs` | SSE 事件順序、同步 prompt 回覆、忙碌排隊、abort 標 `MessageAbortedError`、REST 補歷史、status |
| `tests/fake_claude.rs` | Stop hook block 多一輪（`stop_hook_active` false → true）、Esc 中斷不觸發 Stop、hook payload、channel 包裝、未知 channel server 的錯誤、transcript 在專案目錄內 |

## 花時間的地方

- Runner 契約的兩個「停掉了嗎」case 用真的時間：`sleep 3; touch timed-out-command-finished` 與 `sh -c 'sleep 3; touch timed-out-child-finished'; true` 以 200 ms 逾時跑，各等到約 4.5 秒確認標記檔沒出現（只讀檔，不送 signal 探測）。假實作也一樣等，所以 Runner suite 每跑一次約 9 秒。
- `contract_teeth` 約 15 秒：mutant 平行跑，最慢的是 Runner mutant（約 9–15 秒）與真 `sh` runner 的對照測試。

## 輸入從哪來（#1493）

- 假 daemon 與測試都用 `agend_core::protocol::client` 型別 + `serde_json` 編碼；沒有手寫 client protocol JSON。
- 契約 suite 用 core 的建構子（`Task::new`、`Workflow::builtin_code`、`model::work_branch`）產生輸入。
- backend 協定（codex、opencode、claude）是外部格式，沒有 Rust producer：測試寫的 JSON 取自 `docs/research/spike-*.md` 記錄的形狀。

## 用到的假實作

- 不適用（這裡就是假實作的家）

## 還沒測的

- [ ] 契約 suite 對真實作（各施工關接上：Store 第 5、agent runtime 第 6、Driver 第 7／12、Runner 與 Forge local 第 10、Notifier 與 Forge github 第 12）
- [ ] 假 agent 的欄位對真 backend schema 逐一比對（第 7、12 施工關）
- [ ] git fixture（第 3 施工關需要時）

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-testkit
```
