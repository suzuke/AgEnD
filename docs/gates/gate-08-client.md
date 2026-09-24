# 第 8 關：agend-client + protocol server（整合關）（`client`）

> **TL;DR**
> - CLI 連 daemon、重試、版本檢查。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這關才算完成。
> - 下一步：等前一關完成後開工；開工時把標「開工時細化」的步驟寫定。

## 狀態

**未開始**（2026-09-24）

## 範圍

- agend-client：同步連線、重試 10 秒、版本檢查
- daemon 的 protocol server

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-client`、`~/.cargo/bin/cargo test -p agend-daemon` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept client` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

先讓 `agend` 指到這個 repo 建出來的 binary（第 13 關之前沒有安裝程式；在 repo 根目錄執行）：

```bash
~/.cargo/bin/cargo build -p agend && alias agend="$PWD/target/debug/agend"
```

1. 故意弄壞：daemon 停著時用最小的連線探測（開工時細化：`agend debug ping` 是暫定名稱，屬於這關；`agend status` 要到第 9 關才有）。

   ```bash
   time agend debug ping
   ```

   應該看到：大約 10 秒後印出清楚的錯誤（daemon 沒在跑、怎麼啟動），exit 非 0；`time` 顯示約 10 秒。

   - [ ] 通過

2. 啟動 daemon 後再跑（啟動方式同第 6 關）。

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
