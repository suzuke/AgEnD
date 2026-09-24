# 第 4 關：agend-holder（`holder`）

> **TL;DR**
> - 每個 instance 一個 holder：PTY、畫面、附屬程序、holder 協定。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這關才算完成。
> - 下一步：等前一關完成後開工；開工時把標「開工時細化」的步驟寫定。

## 狀態

**未開始**（2026-09-24）

## 範圍

- 以 PTY 啟動 agent、注入環境
- 畫面維護與快照
- 附屬程序持有
- exit code 回報
- holder 協定 server（版本協商、斷線重連）

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-holder` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept holder` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 跑 demo：holder 包一個每秒加一的 bash 計數器。

   ```bash
   ~/.cargo/bin/cargo xtask accept holder
   ```

   應該看到：印出畫面快照，裡面的數字在增加。

   - [ ] 通過

2. 故意弄壞：探測 client 中途斷線再重連。

   操作：同一次輸出，找 `reconnect`

   應該看到：重連後拿到的畫面數字比斷線前大，沒有中斷或重來。

   - [ ] 通過

3. bash 結束。

   操作：同一次輸出，找 `exit`

   應該看到：holder 回報 exit code（demo 用固定值，例如 7），與 bash 的結束碼一致。

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
cat docs/gates/gate-04-holder.md
~/.cargo/bin/cargo xtask accept holder
```
