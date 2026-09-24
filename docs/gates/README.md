# 關卡頁

> **TL;DR**
> - 13 關各有一頁：範圍、自動驗收、你親自驗收的步驟、驗收紀錄、進度紀錄。
> - 記住：**一關要等你跑完「你親自驗收」並填好「驗收紀錄」才算完成**。
> - 下一步：打開目前這一關（第 1 關，提案中）逐條確認提案。

## 索引

| 關 | 頁面 | 範圍 | 狀態 | 自動驗收指令 |
|---|---|---|---|---|
| 1 | [`core`](gate-01-core.md) | agend-core | 提案中 | `cargo xtask accept core` |
| 2 | [`testkit`](gate-02-testkit.md) | agend-testkit | 未開始 | `cargo xtask accept testkit` |
| 3 | [`shim`](gate-03-shim.md) | agend-shim | 未開始 | `cargo xtask accept shim` |
| 4 | [`holder`](gate-04-holder.md) | agend-holder | 未開始 | `cargo xtask accept holder` |
| 5 | [`store`](gate-05-store.md) | agend-daemon：store | 未開始 | `cargo xtask accept store` |
| 6 | [`daemon-holder`](gate-06-daemon-holder.md) | daemon ↔ holder | 未開始 | `cargo xtask accept daemon-holder` |
| 7 | [`codex`](gate-07-codex.md) | codex driver + 送達 | 未開始 | `cargo xtask accept codex` |
| 8 | [`client`](gate-08-client.md) | agend-client + protocol server | 未開始 | `cargo xtask accept client` |
| 9 | [`cli`](gate-09-cli.md) | agend CLI | 未開始 | `cargo xtask accept cli` |
| 10 | [`pipeline`](gate-10-pipeline.md) | 流水線 | 未開始 | `cargo xtask accept pipeline` |
| 11 | [`tui`](gate-11-tui.md) | agend-tui | 未開始 | `cargo xtask accept tui` |
| 12 | [`adapters`](gate-12-adapters.md) | 其餘 adapter | 未開始 | `cargo xtask accept adapters` |
| 13 | [`install`](gate-13-install.md) | 安裝與發布 | 未開始 | `cargo xtask accept install` |

狀態只用這五個：未開始、提案中、實作中、驗收中、完成（後面加日期）。狀態改變時，同步更新該關頁面與 [ROADMAP 的狀態欄](../ROADMAP.md)。

## 範本

每一頁都用同樣的章節，順序不變：

| 章節 | 內容 |
|---|---|
| TL;DR | 3 行：這關做什麼、要記住的一件事、下一步 |
| 狀態 | 五種之一 + 日期 |
| 範圍 | 這關要交付的東西 |
| 開工前提案 | 只有還有設計問題的關卡才有；每項：問題 · 建議 · 理由 · 替代方案 · `- [ ] 使用者確認` |
| 自動驗收（完成定義） | `cargo test -p`、clippy `-D warnings`、`check-deps`（不能是 SKIPPED）、`cargo xtask accept <關>`、crate README／TESTING 已更新、verifier |
| 你親自驗收 | 編號步驟：指令、應該看到什麼、`- [ ] 通過`；至少一步是「故意弄壞 → 看到它失敗或拒絕」；還不能寫定的標「開工時細化」 |
| 驗收紀錄 | 你填：日期、結果、備註 |
| 進度紀錄 | 日期 + 一行 + commit／PR，新的在上面 |
| 下一步 | 可以直接複製的指令 |

## 下一步

```bash
cat docs/gates/gate-01-core.md
```
