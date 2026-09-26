# 第 9 施工關：agend CLI（`cli`）

> **TL;DR**
> - agent 命令、操作者命令、status、doctor、init。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：等前一個施工關完成後開工；開工時把標「開工時細化」的步驟寫定。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**未開始**（2026-09-24）

## 範圍

- 11 個 agent 命令（D17）
- 操作者命令
- `agend status`
- `agend doctor`、`agend init`（服務註冊在第 13 施工關）
- `agend doctor` 列出所有 holder 並標出孤兒（DB 沒有的 instance）（第 4 施工關 P2，使用者 2026-09-25 決定）
- `agend daemon restart` 的 D2 重啟預檢：從第 6 施工關移來（[gate-06 P7](gate-06-daemon-holder.md#p7d2-的重啟預檢)，使用者 2026-09-26 追認）——新 binary 先用最新 DB 快照的複本跑 migration 加 `quick_check`，再用暫存 home 起一個自己的 holder 跑一次（hello、Spawn、Shutdown），都過了才切換，任何一步失敗就留在舊版

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept cli` 通過，並印出下方「你親自驗收」用到的 demo
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

1. 對有假 agent 的 daemon 跑每個 agent 命令。

   ```bash
   ~/.cargo/bin/cargo xtask accept cli
   ```

   應該看到：每個命令一段：輸入、輸出、exit code；錯誤訊息都附正確命令。

   - [ ] 通過

2. `agend doctor`。

   ```bash
   agend doctor
   ```

   應該看到：列出每一項檢查與結果（git 版本、backend、磁碟…）。

   - [ ] 通過

3. 故意弄壞：只藏起 opencode（agend、codex、claude 仍找得到）。

   ```bash
   T=$(mktemp -d)
   ln -s "$PWD/target/debug/agend" "$T/agend"
   ln -s "$(command -v codex)" "$T/codex"
   ln -s "$(command -v claude)" "$T/claude"
   PATH="$T:/usr/bin:/bin" "$T/agend" doctor; echo "exit=$?"
   ```

   應該看到：opencode 那一行是失敗，說明在 PATH 上找不到 opencode，並附安裝指令（例如 `brew install opencode`）；codex、claude 兩行仍是 ok；`exit=1`（確切措辭開工時細化）。

   - [ ] 通過

4. 在暫存 HOME 跑 `agend init`（開工時細化：`--non-interactive` 是暫定參數名）。

   ```bash
   HOME=$(mktemp -d) agend init --non-interactive
   ```

   應該看到：建立 home 與 `config.toml`、`general` team 與一個 agent，最後跑 doctor。

   - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- （尚無）

## 下一步

```bash
cat docs/gates/gate-09-cli.md
~/.cargo/bin/cargo xtask accept cli
```
