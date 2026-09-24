# agend-core 測試

> **TL;DR**
> - 測純函式、no-std 型別與 workflow 狀態機；protocol wire shape 在 xtask integration tests 驗證。
> - 記住：Codex fixture 是 PTY 擷取；Claude fixture 是 spike 紀錄中的 prompt 文字，並非完整 holder 畫面擷取。
> - 下一步：跑 `cargo xtask accept core`，比對實際狀態機 transcript。

## 怎麼跑

```bash
cargo test -p agend-core
cargo xtask accept core
```

`accept core` 會跑 workspace fmt、workspace clippy、core tests、xtask protocol compatibility tests、`check-deps`，再啟動 `agend-core` 的 `core_demo` example。demo 實際呼叫 protocol hello、assignment、busy、debounce 與 pipeline 狀態機；JSON wire shape 由 `cargo test -p xtask --test protocol_compat` 驗證。

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `model::tests` | backend 名稱與 delivery 狀態轉換；branch producer／consumer 往返 |
| `protocol::tests`、`protocol::client::tests`、`protocol::holder::tests` | major 不相容會有明確錯誤；daemon 可與仍支援的舊 holder major 協商 |
| `xtask/tests/protocol_compat.rs` | exact JSON wire shape、unknown tagged variants、忽略 additive fields；approval request 不接受 caller 指定 head |
| `pipeline::task::tests` | workflow 版本固定、reopen／supersede／關係檢查 |
| `pipeline::workflow::tests` | 三個內建 workflow、repo 要求、角色、approval、on_fail 驗證 |
| `pipeline::state::tests` | code stage 轉換、失敗回 work、head 變更失效、D14 核准保留、merge gate |
| `policy::assign::tests` | reviewer 跨 backend、作者排除、原作者返工、額度轉派、fanout 名額、wait cycle |
| `policy::busy::tests` | codex steer；claude／opencode steer 退成 interrupt；queue 不變 |
| `policy::debounce::tests` | busy 立即生效，idle 穩定 5 秒，busy 會取消待定 idle |
| `policy::conflict::tests` | pairwise 重疊檔案排序與去重 |
| `policy::merge_gate::tests` | merge 必須 checks 通過且核准綁目前 head；rebase 以 patch-id 判斷保留 |
| `screen::tests` | backend-specific startup prompt 片段比對，不分類一般畫面；Claude 的片段不是完整 holder 擷取 |

## Protocol JSON Lines

core 只提供 serde 型別，不含 JSON codec。`xtask/tests/protocol_compat.rs` 用 serde_json 測已知訊息的 wire shape、未知 tag 的 payload 忽略，以及同 major 的 additive-field 規則。`ReviewApprove` 只帶 task id；daemon 依 authenticated review binding 綁定 head。

PTY bytes 在 protocol 型別中使用 `bytes_base64` 欄位；base64 實際編碼和解碼由 holder／client adapter 負責。

## Crate 邊界

- `cargo xtask check-deps` 以 no-std target 編譯 core，檢查 `unsafe-code` 與依賴 allowlist。
- core 沒有 testkit 依賴。所有單元測試都在純資料與純函式上執行。
- screen fixture 來源與證據等級見 `tests/fixtures/screens/README.md`。除 Codex 外，Claude 目前只有 spike prompt 片段；新規則需附 holder 真實畫面與 backend 版本證據。

## 下一步

```bash
cargo xtask accept core
```
