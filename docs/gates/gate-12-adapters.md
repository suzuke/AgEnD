# 第 12 施工關：其餘 adapter（`adapters`）

> **TL;DR**
> - claude、opencode driver、forge github、Telegram。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：等前一個施工關完成後開工；開工時把標「開工時細化」的步驟寫定。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**未開始**（2026-09-24）

## 範圍

- claude driver（D16）
- opencode driver
- forge github
- notifier：Telegram

### 從其他施工關帶來的筆記（開工時處理）

- **G4 已讀狀態**：第 8 施工關的協定缺口 G4（已讀狀態）決定移到本關（第 6 施工關留下的 TUI 本地已讀在那之前都夠用），理由是要 daemon 記已讀的唯一原因是跟 Telegram 共用（見 [gate-08-client P6](gate-08-client.md#p6真-daemon-本關做哪些請求g4-移走)）。
- **claude Bash 工具的 PATH 重排**：第 7 施工關發現 macOS login zsh 的 `path_helper` 會把 shim 排到後面（P4，已用 `ZDOTDIR` 修正 codex 這一路）；任何用 login shell 跑指令的 agent 工具都一樣，claude 的 Bash 工具要在本關實測（見 [gate-03-shim 已知限制](gate-03-shim.md#已知限制)）。
- **未查證的使用者提供事實**：claude 的 TUI 按 ctrl+enter 可以把訊息「強送」給正在工作的 agent（使用者 2026-09-26 提供，未在 repo 查證）；對應三級忙碌策略的 steer／插隊，本關的 claude driver 開工時查證。

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-daemon` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept adapters` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 真 CLI 一致性檢查（必要；使用者已決定 2026-09-25）：`claude --version`、`opencode --version` 和 `crates/agend-testkit/transcripts/{claude,opencode}/` 錄製檔 header 的 `version` 相同，不同就先用錄製器重錄（[RECORDER.md](../../crates/agend-testkit/RECORDER.md#重錄cli-升版時)）；`~/.cargo/bin/cargo test -p agend-testkit --test conformance` 通過
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.x.y`（目前是 `agend 0.0.0`）。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

1. 三個真 backend 互傳訊息。

   操作：開工時細化

   應該看到：claude、codex、opencode 三個 agent 各收到一則並回覆，狀態都到 `confirmed`。

   - [ ] 通過

2. forge github：在 sandbox repo 跑完整流水線。

   操作：開工時細化

   應該看到：GitHub 上出現 PR，checks 通過後被 daemon merge，PR 顯示 merged。

   - [ ] 通過

3. Telegram：手機收到「需要你」。

   操作：開工時細化

   應該看到：手機收到訊息；你在手機上回覆後，TUI 的該項變成已解決。

   - [ ] 通過

4. 故意弄壞：把 `config.toml` 裡 Telegram 的 allowlist 改成空的（開工時細化：確切鍵名；目前沒有清空 allowlist 的命令），然後跑 doctor。

   ```bash
   agend doctor; echo "exit=$?"
   ```

   應該看到：Telegram 那一行是失敗：allowlist 是空的、所有訊息會被丟棄，並附手動修正方式：在 `config.toml` 的 allowlist 加回你的 chat id（鍵名開工時細化）；`exit=1`（v1 #2207 的情境）。第 13 施工關之後，doctor 改為指向 `agend telegram setup`。

   - [ ] 通過

5. 真 CLI 一致性檢查（必做；使用者已決定 2026-09-25）。

   ```bash
   claude --version; opencode --version
   head -1 crates/agend-testkit/transcripts/claude/one_turn.jsonl crates/agend-testkit/transcripts/opencode/one_turn.jsonl
   ~/.cargo/bin/cargo test -p agend-testkit --test conformance
   ```

   應該看到：兩個版本各自和錄製檔 header 的 `"version"` 相同；最後 `test result: ok. 4 passed`。版本不同：先重錄那個 backend（`~/.cargo/bin/cargo xtask record claude --sandbox ~/Documents/Hack/AgEnD-ops/record-sandbox.sh`，或 `opencode`；會跑真的 CLI、花少量 token，見 [RECORDER.md](../../crates/agend-testkit/RECORDER.md)）再跑一次；檢查不過就改假 agent，不改錄製檔。

   **這步在驗什麼**：driver 測試用的假 claude／假 opencode 和你機器上真的 CLI 形狀一致。壞了的話，driver 對假的全綠、接上真的才出錯（v1 #1483）。

   - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-25 使用者決定：真 CLI 一致性檢查（錄製器 + `tests/conformance.rs`）列為必要完成條件（`feat/backend-recorder`）。

## 下一步

```bash
cat docs/gates/gate-12-adapters.md
~/.cargo/bin/cargo xtask accept adapters
```
