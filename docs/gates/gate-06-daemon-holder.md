# 第 6 施工關：daemon ↔ holder（整合施工關）（`daemon-holder`）

> **TL;DR**
> - agent runtime adapter：daemon 重啟時 agent 不斷線。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：等前一個施工關完成後開工；開工時把標「開工時細化」的步驟寫定。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**未開始**（2026-09-24）

## 範圍

- daemon 端的 holder 協定 client（agent runtime adapter）
- daemon 重啟後重連 holder、取回畫面
- daemon 開機時與之後每 24 小時跑一次 store 的 `prune` 與每日 DB 快照（第 5 施工關 P8/P9）
- 開 DB 時重試到 10 秒，因為重啟時 EXCLUSIVE lock 交接需要時間（第 5 施工關風險）

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-daemon`、`~/.cargo/bin/cargo test -p agend-holder` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept daemon-holder` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] 重啟／持久化類的契約 case（RTM-8、RTM-9，見 [CONTRACTS.md](../../crates/agend-testkit/CONTRACTS.md)）對真實作、跨真的 process 重啟跑（分開的 process、真的檔案／DB）**（已追認 2026-09-25，第 2 施工關 A25）**
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.x.y`（目前是 `agend 0.0.0`）。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

1. 前景啟動 daemon，並起一個假 agent（計數器）在 holder 裡。

   操作：開工時細化：例如 `agend daemon --foreground` + `agend debug spawn-fake counter`

   應該看到：daemon log 顯示 holder 已連線；計數器在跑。

   - [ ] 通過

2. 故意弄壞：Ctrl-C 停掉 daemon，再啟動一次。

   操作：同上的啟動指令

   應該看到：agent 的計數器沒有中斷（數字接著增加）；daemon 取回的畫面就是當下的畫面。

   - [ ] 通過

3. 確認沒有重複的 holder。

   ```bash
   pgrep -fl "agend holder"
   ```

   應該看到：只有一個 holder 程序（重啟 daemon 沒有多生一個）。

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
cat docs/gates/gate-06-daemon-holder.md
~/.cargo/bin/cargo xtask accept daemon-holder
```
