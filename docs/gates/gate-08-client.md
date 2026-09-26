# 第 8 施工關：agend-client + protocol server（整合施工關）（`client`）

> **TL;DR**
> - CLI 連 daemon、重試、版本檢查。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：等前一個施工關完成後開工；開工時把標「開工時細化」的步驟寫定。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**未開始**（2026-09-24）

## 範圍

- agend-client：同步連線、重試 10 秒、版本檢查
- daemon 的 protocol server
- 從第 11 施工關畫面層移來的協定缺口（TUI 提前用假來源踩出來的，見 [gate-11 待你追認](gate-11-tui.md#待你追認) G1–G4）：加 team／task／agent 的 list 請求（或快照事件）與 task 關卡清單、agent 結構化狀態（G1）；`attention_required` 加「解決後能放行多少工作」與等待起始時間兩個欄位（G2）；非請示的「需要你」項目加操作（重試、暫停…）與清除事件（G3）；加已讀狀態的請求與事件，TUI 與 Telegram 共用（G4）

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-client`、`~/.cargo/bin/cargo test -p agend-daemon` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept client` 通過，並印出下方「你親自驗收」用到的 demo
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

1. 故意弄壞：daemon 停著時用最小的連線探測（開工時細化：`agend debug ping` 是暫定名稱，屬於這個施工關；`agend status` 要到第 9 施工關才有）。

   ```bash
   time agend debug ping
   ```

   應該看到：大約 10 秒後印出清楚的錯誤（daemon 沒在跑、怎麼啟動），exit 非 0；`time` 顯示約 10 秒。

   - [ ] 通過

2. 啟動 daemon 後再跑（啟動方式同第 6 施工關）。

   ```bash
   agend debug ping
   ```

   應該看到：馬上回應 daemon 的協定版本，exit 0。

   - [ ] 通過

3. 命令執行中重啟 daemon。

   操作：開工時細化：一個會持續幾秒的命令 + 另一個終端重啟 daemon

   應該看到：命令最後仍成功完成。

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
cat docs/gates/gate-08-client.md
~/.cargo/bin/cargo xtask accept client
```
