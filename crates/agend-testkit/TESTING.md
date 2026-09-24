# agend-testkit 測試

> **TL;DR**
> - 測假實作本身、7 個契約 suite（對假實作全過、對故意弄壞的包裝會失敗）、假 daemon、3 個假 agent 程式（以真的子程序跑）。
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
| `tests/contract_teeth.rs` | 每個 suite 抓得到一種漂移：merge 不看 head、CAS 不看版本、事件不看 cursor、送達停在 Queued、recover 忘了 holder、通知被截斷、秒當毫秒、逾時帶 exit code；失敗的正是那一條 |
| `tests/fake_daemon.rs` | hello 必須在前、major 不合的錯誤訊息、狀態與未知請求、事件身分（沒帶、舊 attempt、別的關卡、重播都 `stale_result`）、backlog 再即時事件、請示回答 |
| `tests/fake_codex.rs` | turn 完成事件帶回 threadId／turnId；steer（錯的 turn id 被拒）、queue 自動出列成新 turn、interrupt；approval 等待決定；只有 resume 過的 thread 才推事件；長路徑 symlink |
| `tests/fake_opencode.rs` | SSE 事件順序、同步 prompt 回覆、忙碌排隊、abort 標 `MessageAbortedError`、REST 補歷史、status |
| `tests/fake_claude.rs` | Stop hook block 多一輪（`stop_hook_active` false → true）、Esc 中斷不觸發 Stop、hook payload、channel 包裝、未知 channel server 的錯誤 |

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
