# 第 13 關：安裝與發布（`install`）

> **TL;DR**
> - 服務註冊、uninstall、telegram setup、打包與發布。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這關才算完成。
> - 下一步：等前一關完成後開工；開工時把標「開工時細化」的步驟寫定。

## 狀態

**未開始**（2026-09-24）

## 範圍

- launchd／systemd 服務註冊
- `agend uninstall`（移除服務與 shim；刪資料前先問）
- `agend telegram setup`（由 daemon 的 notifier 配對）
- `xtask release`、brew formula、GitHub release workflow、`cargo install`

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-core`、`~/.cargo/bin/cargo test -p agend-daemon`、`~/.cargo/bin/cargo test -p agend` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept install` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 全新 HOME 從 release 安裝到第一個 task 完成。

   操作：開工時細化：下載 release 產物 → `agend init` → 建立 task

   應該看到：印出總耗時，小於 5 分鐘。

   - [ ] 通過

2. 故意弄壞：每個 doctor 檢查。

   ```bash
   ~/.cargo/bin/cargo xtask accept install
   ```

   應該看到：每個檢查一段：「弄壞 → doctor 顯示修正指令 → 照做後恢復」。

   - [ ] 通過

3. 解除安裝。

   ```bash
   agend uninstall
   ```

   應該看到：刪資料前先問；結束後 `launchctl list | grep agend`（或 `systemctl --user list-units | grep agend`）沒有結果，agent PATH 的 shim 不見了。

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
cat docs/gates/gate-13-install.md
~/.cargo/bin/cargo xtask accept install
```
