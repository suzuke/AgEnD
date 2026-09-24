# agend-tui

> **TL;DR**
> - attention-first TUI：先看「需要你」，再看各 team。
> - 記住：**只透過 `agend-client` 跟 daemon 溝通**，和其他 client 一樣。
> - 下一步：第 11 施工關實作（沿用 DEMO-01 原型的教訓）。

## 負責

- 首頁：跨 team 的「需要你」＋每個 team 一個區塊
- team 頁：目標、Agents、流水線
- Task Detail、Agent Detail、單一 agent attach
- 英文與繁中，執行中按 `L` 切換

## 不負責

- 直接連 holder
- 自己推算 agent 狀態
- 分割視窗（v2.0 不做）

## 模組

| 模組 | 職責 |
|---|---|
| `home` | 首頁 |
| `attention` | 「需要你」；已讀與已解決分開 |
| `team` | team 頁 |
| `task_detail` | task 細節（repo 只在這裡出現） |
| `agent_detail` | agent 細節 |
| `terminal` | attach 終端串流 |
| `i18n` | `Language`、`toggled` |

## 依賴規則

- 一般依賴：`agend-core`、`agend-client`

## 入口

- `agend_tui::i18n::Language`
- 之後：`agend app` 子命令

## 下一步

```bash
cargo test -p agend-tui
```
