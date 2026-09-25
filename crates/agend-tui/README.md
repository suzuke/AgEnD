# agend-tui

> **TL;DR**
> - attention-first TUI：先看「需要你」，再看各 team；畫面層已完成，資料目前接假來源（第 11 施工關提前的部分）。
> - 記住：**畫面只讀 `source::Fleet`、只透過 `source::Source` 動作**，不知道資料從哪來；接真 daemon 時用 `agend-client` 實作 `Source`。
> - 下一步：`~/.cargo/bin/cargo run -q -p agend-tui --example tui_fake` 自己操作；`~/.cargo/bin/cargo xtask accept tui` 看 demo。

## 負責

- 首頁：跨 team 的「需要你」＋每個 team 一個區塊（agent 狀態數量、進行中的目標、最近變更）
- 「需要你」：展開項目看脈絡摘要（D37）與對話（D35），選選項或用自由文字回答；已讀與已解決分開，agent 追問後回到未讀
- team 頁：目標、Agents、流水線三個 tab
- Task Detail（repo 只在這裡）、Agent Detail、單一 agent 終端（`t`，目前是唯讀快照）
- `/` 快速跳轉；英文與繁中，執行中按 `L` 切換
- daemon 斷線畫面與自動重連

## 不負責

- 直接連 holder 或 socket（lib 裡沒有 socket 程式碼）
- 自己推算 agent 狀態或「需要你」是否解決（照 source 回報）
- 分割視窗（v2.0 不做）、滑鼠（見第 11 施工關頁「待你追認」T12）

## 模組

| 模組 | 職責 |
|---|---|
| `source` | `Source` trait、`Fleet`（catalog + 事件重播）、`Catalog`（protocol v1 還列不出來的 team／task／agent） |
| `source::scripted` | 記憶體裡的腳本假來源與 demo 資料（測試與 demo 用） |
| `app` | `App`：導覽堆疊、按鍵、`tick`（拉事件、斷線重連）、底部說明（只列目前有用的鍵） |
| `ui` | `Row` 與畫面繪製、選取反白（不含邊框與框線）、寬字元寬度 |
| `home` | 首頁 |
| `attention` | 「需要你」畫面 |
| `team` | team 頁 |
| `task_detail` | task 細節（repo 只在這裡出現） |
| `agent_detail` | agent 細節 |
| `terminal` | 終端快照畫面、事件的一行說明 |
| `finder` | `/` 快速跳轉 |
| `i18n` | `Language`、`Text` 字串表 |

## 按鍵

| 鍵 | 動作 |
|---|---|
| `↑` `↓`／`k` `j`、`PgUp` `PgDn` | 移動選取；到底後繼續捲動 |
| `→`／`Enter` | 完全相同：進下一層（或選這個選項） |
| `←`／`Esc` | 回上一層（首頁不動作，不會離開） |
| `t` | 開這一列的 agent 終端；沒有 agent 或沒有輸出就只顯示一行訊息 |
| `/` | 快速跳轉（agent → 終端、task → Task Detail、team → team 頁） |
| `1` `2` `3`、`Tab`／`Shift-Tab` | team 頁切 tab |
| `1`–`9`、`a` | 「需要你」畫面：選選項、用自由文字回答 |
| `h`、`!` | 首頁、「需要你」 |
| `L` | 切換英文／繁中（只有大寫） |
| `r` | 斷線時立即重試 |
| `q`／`Ctrl-C` | 離開 |

## 依賴規則

- 一般依賴：`agend-core`、`agend-client`、`ratatui`（只開 `crossterm` + `std`）、`unicode-width`
- dev 依賴：`agend-testkit`（假 daemon）、`serde_json`（demo 的 socket client）
- 不可依賴 SQLite、`agend-daemon`（`cargo xtask check-deps`）

## 入口

- `agend_tui::App::new(Box<dyn Source>, Language)`、`App::key`、`App::tick`、`ui::render`
- `agend_tui::render_to_string`：畫到 `TestBackend`，給測試與 demo
- 範例：`tui_fake`（互動，`--daemon` 改接 testkit 假 daemon，`--lang zh-TW`；`F2` 停／啟 daemon、`F3` 假 agent 追問）、`tui_accept`（`cargo xtask accept tui` 跑的 demo）
- 之後：`agend app` 子命令（第 8 施工關後）

## 下一步

```bash
~/.cargo/bin/cargo run -q -p agend-tui --example tui_fake -- --lang zh-TW
~/.cargo/bin/cargo xtask accept tui
```
