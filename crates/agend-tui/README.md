# agend-tui

> **TL;DR**
> - attention-first TUI：先看「需要你」，再看各 team；`agend app` 經 `agend-client` 接真 daemon（第 11 施工關 B 段），終端即時更新、按 `i` 才能打字。
> - 記住：**畫面只讀 `source::Fleet`、只透過 `source::Source` 動作**；真的來源是 `source::client::ClientSource`，lib 裡沒有 socket 程式碼。
> - 下一步：`~/.cargo/bin/cargo run -q -p agend-tui --example tui_fake` 自己操作；`~/.cargo/bin/cargo xtask accept tui` 看 demo。

## 第 10 施工關（已驗收，2026-10-02）

pipeline task detail 顯示 repo、關卡種類、agent、受阻理由與 WIP archive 路徑；人工核准可 approve／退回修改（先輸入理由），task failed 可 acknowledge。讀取資料與動作仍只經 Source 和 client protocol。

## 負責

- 首頁：跨 team 的「需要你」＋每個 team 一個區塊（agent 狀態數量、進行中的目標、最近變更）
- 「需要你」：展開項目看脈絡摘要（D37）與對話（D35），選選項或用自由文字回答；已讀與已解決分開，agent 追問後回到未讀
- team 頁：目標、Agents、流水線三個 tab
- Task Detail（repo 只在這裡）、Agent Detail、單一 agent 終端（`t`：即時畫面；`i` 進輸入模式、`Ctrl-]` 離開）
- 「需要你」的非請示項目：`actions` 是可選的列（retry、approve、request_changes、acknowledge），收到 daemon 的 `attention_resolved` 才消失
- `/` 快速跳轉；英文與繁中，執行中按 `L` 切換
- daemon 斷線畫面與自動重連（重連一律重拿全貌；版本不合不自動重試）
- 互動迴圈 `agend_tui::run`（`agend app` 與 `tui_fake` 共用）

## 不負責

- 直接連 holder 或 socket（lib 裡沒有 socket 程式碼；連線只經 `agend-client`）
- 自己推算 agent 狀態或「需要你」是否解決（照 source 回報）
- 決定誰能打字、誰能 `retry`（daemon 依 `hello` 的 `caller` 決定，D17）
- 改 agent 的 PTY 大小、滑鼠、bracketed paste
- 分割視窗（v2.0 不做）、滑鼠（見第 11 施工關頁「待你追認」T12）

## 模組

| 模組 | 職責 |
|---|---|
| `source` | `Source` trait、`Snapshot`（連上時的 `Catalog` ＋需要你清單）、`Fleet`（快照＋之後的事件）、`TerminalEvent` |
| `source::client` | `ClientSource`：三條連線（事件、請求、終端），每條阻塞讀的連線一條 thread；`FleetView` → `Catalog` 的轉換 |
| `source::scripted` | 記憶體裡的腳本假來源與 demo 資料（測試與 demo 用） |
| `app` | `App`：導覽堆疊、按鍵、`tick`（拉事件、終端更新與重訂、斷線重連）、輸入模式、底部說明（只列目前有用的鍵） |
| `ui` | `Row` 與畫面繪製、選取反白（不含邊框與框線）、寬字元寬度 |
| `home` | 首頁 |
| `attention` | 「需要你」畫面 |
| `team` | team 頁 |
| `task_detail` | task 細節（repo 只在這裡出現） |
| `agent_detail` | agent 細節 |
| `terminal` | 終端畫面（標題：即時／最後的畫面（已停止）／已結束，重試中／輸入中）、事件的一行說明 |
| `finder` | `/` 快速跳轉 |
| `i18n` | `Language`、`Text` 字串表 |

## 按鍵

| 鍵 | 動作 |
|---|---|
| `↑` `↓`／`k` `j`、`PgUp` `PgDn` | 移動選取；到底後繼續捲動 |
| `→`／`Enter` | 完全相同：進下一層（或選這個選項） |
| `←`／`Esc` | 回上一層（首頁不動作，不會離開） |
| `t` | 開這一列的 agent 終端；沒有 agent 或沒有輸出就只顯示一行訊息 |
| `i` | 終端畫面：進輸入模式（只有即時的終端） |
| `Ctrl-]`（或 `Ctrl-5`） | 輸入模式：離開；**其他每個鍵都送給 agent**（包括 `q`、`Esc`、`←`、`L`、`Ctrl-C`） |
| `/` | 快速跳轉（agent → 終端、task → Task Detail、team → team 頁） |
| `1` `2` `3`、`Tab`／`Shift-Tab` | team 頁切 tab |
| `1`–`9`、`a` | 「需要你」畫面：選選項或操作（`重試`）、用自由文字回答 |
| `h`、`!` | 首頁、「需要你」 |
| `L` | 切換英文／繁中（只有大寫） |
| `r` | 斷線時立即重試 |
| `q`／`Ctrl-C` | 離開 |

## 依賴規則

- 一般依賴：`agend-core`、`agend-client`、`ratatui`（只開 `crossterm` + `std`）、`unicode-width`
- dev 依賴：`agend-testkit`（假 daemon、proxy）、`serde_json`
- 不建 async runtime（D11）：阻塞讀交給 std thread，主 thread 經 channel 拿
- 不可依賴 SQLite、`agend-daemon`（`cargo xtask check-deps`）

## 入口

- `agend_tui::run(Box<dyn Source>, Language)`：互動迴圈（raw mode、alternate screen、每 100 ms 一次 `tick`）；`run_with` 多一個先看按鍵的 hook（demo 鍵）
- `agend_tui::source::client::ClientSource::new(socket, caller)`：`agend app` 用它
- `agend_tui::App::new(Box<dyn Source>, Language)`、`App::key`、`App::tick`、`ui::render`
- `agend_tui::render_to_string`：畫到 `TestBackend`，給測試與 demo
- 範例：`tui_fake`（互動，`--daemon` 改經 `ClientSource` 接 testkit 假 daemon，`--lang zh-TW`；`F2` 停／啟 daemon、`F3` 假 agent 追問）、`tui_accept`（`cargo xtask accept tui` 跑的假 daemon demo；真 daemon 那段是 `crates/agend/examples/tui_real.rs`）
- `agend app [--lang en|zh-TW]`（`crates/agend`）

## 下一步

```bash
~/.cargo/bin/cargo run -q -p agend-tui --example tui_fake -- --lang zh-TW
~/.cargo/bin/cargo xtask accept tui
```
