# 第 11 施工關：agend-tui（`tui`）

> **TL;DR**
> - attention-first TUI；畫面層提前做（你同意與第 3–10 施工關並行，放寬 D22），資料先接假來源，draft PR 不 merge。
> - 記住：**畫面只讀 `Source`，不知道資料從哪來**；接真 daemon 要等第 8 施工關把 client protocol 定案。
> - 下一步：照「你親自驗收」A 段由 agent 帶著走一遍；「待你追認」T1–T18、G1–G4 逐項決定。

**先看這條**：B 段（接真 daemon）的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑 B 段開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。A 段只用 `cargo`，不用 `agend`。

## 狀態

**實作中（畫面層提前，資料接假來源）**（2026-09-25）：draft PR，畫面、按鍵、斷線畫面已完成並有測試；資料來自腳本假來源與 testkit 假 daemon。接真 daemon 在第 8 施工關之後。

## 範圍

- 首頁（「需要你」+ 各 team 區塊）、team 頁（目標／Agents／流水線）、Task Detail、Agent Detail
- attach 單一 agent 的終端（`t`；目前是唯讀快照）
- `/` 快速跳轉、英文／繁中切換（`L`）
- 「需要你」：已讀與已解決分開；選項或自由文字回答（D35）
- daemon 斷線畫面與自動重連
- 之後（第 8 施工關後）：用 `agend-client` 實作 `Source`、`agend app` 子命令、終端即時串流與輸入

## 自動驗收（完成定義）

- [x] `~/.cargo/bin/cargo test -p agend-tui` 單獨通過（畫面層，2026-09-25）
- [x] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨（畫面層，2026-09-25）
- [x] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）（畫面層，2026-09-25）
- [x] `~/.cargo/bin/cargo xtask accept tui` 通過，並印出下方「你親自驗收」用到的 demo（畫面層，2026-09-25）
- [x] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新（畫面層）
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

由 agent 帶著走：一次一步，你貼輸出，agent 逐項比對。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。

### A. 畫面層（假資料；步驟本身待你追認）

在這個 PR 的 worktree（`~/Documents/Hack/AgEnD-v2-tui`）或 merge 後的 repo 根目錄跑。終端機至少 100×30。

1. 跑 demo。

   ```bash
   ~/.cargo/bin/cargo xtask accept tui 2>&1 | grep -E '^(==|tui demo|gate 11)'
   ```

   應該看到（第一次要編譯約 1 分鐘）：

   ```text
   == gate 11 (tui) == docs/gates/gate-11-tui.md
   == screens (fake daemon over its socket, 100x30)
   == navigate (scripted keys; each line: keys -> breadcrumb | selected row or first line)
   == resolve (read is not resolved; answering removes the item)
   == disconnect (stop the fake daemon while the TUI is open)
   tui demo: screens, navigation, resolve and disconnect checks passed
   gate 11 (tui): checks passed
   ```

   想看畫面本身：拿掉 `| grep ...`，每個畫面以 `   | ` 開頭印出，英文與繁中各一次。

   **這步在驗什麼**：四個畫面、按鍵、回答請示、斷線，全部透過 testkit 假 daemon 的真 socket（client protocol v1）跑，而且每段都有自動檢查。壞了的話，畫面層和協定之間的接縫就沒有證據，之後接真 daemon 會一次冒出很多問題。

   - [ ] 通過

2. 開互動版，看首頁。

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-tui --example tui_fake -- --lang zh-TW
   ```

   應該看到：

   | 位置 | 要找的 |
   |---|---|
   | 第 3 行 | `━━ 需要你 · 3 ━━` |
   | 第一項（已選取，有 `›`） | `Regression suite fails 3 of 10 runs…`，右邊 `新  archfix · T-45` |
   | 下面 | `┏ archfix`、`┏ research`、`┏ general` 三個區塊，標題右邊是 agent 狀態數量 |
   | `general` 區塊 | `沒有進行中的目標` |

   選取的那一列從 `›` 到右邊反白，但左邊的 `▌`／`┃` 不反白。

   **這步在驗什麼**：attention-first 的版面（需要你在最上、依 team 分組、repo 不出現在首頁）和 DEMO-01 的選取樣式。壞了的話，你一打開就得自己找哪裡需要你。

   - [ ] 通過

3. 已讀不等於已解決；回答後才消失（接著第 2 步的畫面）。

   操作：`Enter`（展開第一項）→ `↓` 三次（移到第二項）→ `h`（回首頁）

   應該看到：還是 `需要你 · 3`，但第一項右邊的 `新` 不見了（已讀）。

   操作：`Enter` → `1`

   應該看到：底下訊息 `已送出對 A-1 的回答：fixed seed 42`，清單變 `需要你 · 2 項待處理`，這一項不見了。

   操作：`F3`（假 agent dev-2 追問）→ `h`

   應該看到：`需要你 · 3`，第一項變成 `Seed 42 hides the flake. Also run 20 times nightly?`，右邊又有 `新`（追問是新的問題，算未讀）；`Enter` 展開後看得到 `你（tui）：fixed seed 42` 這段歷史。

   **這步在驗什麼**：D35 的請示是對話，已讀和已解決分開。壞了的話，只是看一眼就把事情「處理掉」，agent 會卡著等一個永遠不會來的回答。

   - [ ] 通過

4. `t` 看 agent 終端、`←`／`→` 上下層、`/` 搜尋、`L` 切語言。

   操作與應該看到：

   | 操作 | 應該看到 |
   |---|---|
   | `h`，`↓` 移到 `● Fix lock-order inversion`，按 `t` | `dev-1 的終端 · 唯讀快照`，內容有 `cargo test --workspace` |
   | `←` | 回首頁，選取還在同一列 |
   | `↑` 移到 `┏ archfix`，按 `→` | team 頁，第一行 `AgEnD › archfix`，`[1 目標]` |
   | `2`，`↓` 兩次到 `qa-1`，按 `t` | 底下訊息 `qa-1 沒有終端輸出。`，畫面不變 |
   | `←` | 回首頁，`┏ archfix` 仍是選取 |
   | `/`，打 `rev`，`Enter` | `reviewer-1 的終端`；`←` 回到按 `/` 的地方 |
   | `L` | 同一個畫面換成英文（`L 中文`）；再按一次換回來 |
   | `q` | 離開，終端機恢復正常 |

   **這步在驗什麼**：DEMO-01 五輪決定的導覽：`←`／`→` 永遠是上一層／下一層、`t` 從任何有 agent 的列直接開終端、`/` 是覆蓋層不是一層、`L` 不改狀態。壞了的話，你會在畫面之間迷路。

   - [ ] 通過

5. 故意弄壞：TUI 開著時停掉假 daemon（真的 socket）。

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-tui --example tui_fake -- --daemon
   ```

   操作：`↓` 三次、`→`（進 archfix team 頁）→ `F2`（停掉假 daemon）

   應該看到：`━━ Daemon disconnected`、`Lost the connection to the daemon: the daemon closed the connection`、`Reconnect attempt N failed: cannot connect to …`（N 會增加），沒有舊資料，程式沒有當掉。

   操作：`F2`（啟動新的假 daemon）

   應該看到：一秒內 `Reconnected to the daemon.`，畫面回到 `AgEnD › archfix`。`q` 離開。

   **這步在驗什麼**：daemon 不在時 TUI 說清楚發生什麼事、自己重試，daemon 回來後回到原本的畫面。壞了的話，daemon 重啟一次 TUI 就當掉或顯示過期資料（v1 的設定錯誤都是「靜靜不動」）。

   - [ ] 通過

### B. 接真 daemon（第 8 施工關後接上）

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.x.y`（目前是 `agend 0.0.0`）。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

1. 對有假 agent 的 daemon 開 TUI（第 8 施工關後接上；daemon 與假 agent 的啟動方式同第 6 施工關）。`agend app` → 首頁最上面是「需要你」，下面每個 team 一個區塊。

   - [ ] 通過
2. 處理一項請示（第 8 施工關後接上）：選一項 → 選一個動作 → 該項從「需要你」消失；只是看過不會消失。

   - [ ] 通過
3. 看 agent 輸出（第 8 施工關後接上）：選一個 agent → `t` → 顯示該 agent 的終端畫面（即時串流）。

   - [ ] 通過
4. 導覽（第 8 施工關後接上）：`←`／`→`、`/`、`L` 同 A 段第 4 步。

   - [ ] 通過
5. 故意弄壞（第 8 施工關後接上）：另一個終端停 daemon → TUI 顯示斷線狀態而不是當掉；daemon 回來後自動恢復。

   - [ ] 通過

## 待你追認

畫面層提前時由實作者決定、可以反悔的事。每項：決定 · 理由 · 反悔的成本 · 追認結果。G 開頭的是給第 8 施工關的協定缺口（這個 PR 沒有改 core、沒有發明 wire 訊息）。

| # | 決定 | 理由 | 反悔成本 | 追認結果 |
|---|---|---|---|---|
| T1 | 資料來源接縫是 trait `source::Source`（`connect`／`poll`／`terminal`／`answer`）；畫面只讀 `source::Fleet`，`Fleet` 只由 catalog + client protocol v1 事件（`EventData`）重建 | 畫面不知道資料從哪來；腳本假來源、假 daemon、之後的真 daemon 走同一條路 | 換介面：`source.rs` 與兩個實作 | 待追認 |
| T2 | 兩個實作：`source::scripted::ScriptedSource`（lib 內，給測試與 demo，回答規則同 testkit 假 daemon）；假 daemon 的 socket client 放 `examples/support/daemon_source.rs`（dev 專用，examples 與 tests 用 `#[path]` 共用），不放 lib、不放 `agend-client` | lib 不該有 socket 程式碼（「只透過 agend-client」）；`agend-client` 的連線與重試是第 8 施工關的範圍，現在寫會先替它做決定 | 搬進 `agend-client`：第 8 施工關本來就要做 | 待追認 |
| T3 | 「需要你」的定義：ask 的最後一筆是提問或追問才算；回答後離開、agent 追問再回來；有結論也離開 | D35 的請示是對話；「已解決」要看 daemon 回的 `ask_updated`，不是 TUI 自己決定 | 改 `Attention::waiting` 一處 | 待追認 |
| T4 | 已讀是 TUI 本機狀態：展開過的項目去掉粗體和 `new`；重開 TUI 會回到未讀，Telegram 不同步 | protocol v1 沒有已讀狀態（見 G4） | 協定加欄位後改讀 daemon 的值 | 待追認 |
| T5 | 回答方式：選項是可選取的列，`→`／`Enter` 或 `1`–`9` 選；`a` 開一行自由文字；回答後選取移到下一項 | D35 要能自由回答；DEMO-01 的 `Tab` 切焦點改成直接把選項當列，少一個模式 | 改 `attention.rs` 與 `App::key` | 待追認 |
| T6 | 斷線時整個內容換成斷線說明（原因、第幾次重連失敗、狀態都在 daemon），不顯示舊資料；每 500 ms 自動重連，`r` 立即重試；重連後重播事件、回到原本畫面與選取 | 舊資料會讓人以為還在更新；自動重連不用你動手 | 改 `ui.rs` 的斷線分支 | 待追認 |
| T7 | attach 只顯示 `terminal_snapshot` 唯讀快照；即時串流（`terminal_bytes`）與輸入（`terminal_input`）留到第 11 施工關正式接 | 假 daemon 只回一張快照；輸入要身分與權限，等真 daemon | 之後加串流與輸入，畫面結構不變 | 待追認 |
| T8 | testkit 加 `FakeDaemon::open_ask(thread, recap)`：建立帶 task 與脈絡摘要的請示，可以 `answer_ask` | 假 daemon 的 `ask` 命令建立的請示 `task_id` 永遠是 None、沒有 recap，畫面無法依 team 分組 | 移除一個方法與一個測試 | 待追認 |
| T9 | `check-deps` 加 agend-tui 規則：不可依賴 SQLite 與 agend-daemon；不擋 async runtime | TUI 是 protocol client（D11）；crossterm 會帶進 `mio`，擋 runtime 會誤擋 | 改 `RULES` 一筆 | 待追認 |
| T10 | 依賴：`ratatui 0.30`（關掉預設功能，只開 `crossterm` + `std`）與 `unicode-width 0.2`；crossterm 用 ratatui 的 re-export；dev 依賴 `agend-testkit`、`serde_json` | 與 DEMO-01 同版本；關掉預設功能少 87 個 crate | 改 `Cargo.toml` | 待追認 |
| T11 | 文字：一張表（`i18n::Text` enum），不用 i18n crate；daemon 來的標題、問題、摘要不翻譯；繁中用詞沿用 DEMO-01，但依名詞表把「步驟」改「關卡」、「流程」改「流水線」 | 與名詞表一致；資料翻譯要由 daemon 或 agent 做 | 改 `i18n.rs` | 待追認 |
| T12 | 與 DEMO-01 的差異：不做滑鼠；team 區塊的「最近變更」列不能選（同一個 task 已有目標列）；流水線 tab 是依關卡種類分組的清單，不是欄位；team 頁切 tab 時選取回到第一列；`/` 用子字串比對；沒有 `Space`（下個模擬事件）與 `r`（重設），`r` 改成斷線時立即重試；終端標題是「唯讀快照」 | KISS：先做鍵盤與畫面；滑鼠與欄位版面等你用過再決定 | 各自在對應模組補回 | 待追認 |
| T13 | 首頁「需要你」列 `→` 開「需要你」畫面並展開該項；Task Detail 只有目前關卡 `→` 會開它的請示；`t` 在請示列開提問的 agent，非請示項目開 task 持有者 | 同 DEMO-01 的層級；「誰問的」比「誰持有」更接近要看的終端 | 改 `App::open_selected` | 待追認 |
| T14 | 你親自驗收 A 段的步驟本身（上面 A1–A5，含 `F2`／`F3` 這兩個只在 `tui_fake` 範例有的 demo 鍵） | 你要求先寫定步驟再驗 | 改這一頁 | 待追認 |
| T15 | 想改但依規則沒改的文件（列給你決定）：名詞表加「資料來源（data source，`source::Source`）」與「目錄（catalog，`source::Catalog`）」；AGENTS.md 的 crate 邊界表加一列「`agend-tui` 不依賴 SQLite、`agend-daemon`」；ROADMAP 與 docs/gates/README.md 的狀態欄 | 這次的規則不准改 ROADMAP、gates/README、GLOSSARY、AGENTS、DECISIONS | 另一個 docs PR | 待追認 |
| T16 | 「需要你」展開後，在「來自 … · 任務 … · team …」下一行加「不處理的話：…」（DEMO-01 §4B 第 3 點）；`attention_required` 沒有這個欄位，所以放在 `Catalog.if_ignored`（以 task id 為鍵），demo catalog 給三句固定文字 | DEMO 規格要讓人一眼看懂不處理會怎樣；不在 TUI 自己推算 | 第 8 施工關加欄位後改讀協定；刪一個欄位與一行 | 待追認 |
| T17 | 已讀記在「哪一題」上（`Attention::read_key` = 請示 id + 問題數）：agent 追問後這一項回到未讀（粗體、`新`） | 追問是新的問題，你還沒看過；只記請示 id 會讓追問靜靜回來，容易漏看 | 改 `read_key` 一處（只用請示 id 就回到舊行為） | 待追認 |
| T18 | 「需要你」數量與 agent 的「需要你」狀態跟著目前的需要你清單重算（`Fleet::agent_state`）：有等待中的項目指向這個 agent（提問者，否則 task 持有者）就是「需要你」；catalog 說「需要你」但已經沒有等待中的項目時，手上有沒做完的 task 算「工作中」，否則「閒置」；其他狀態照 catalog | 回答 A-1 後 archfix 標題還寫 `! 1 needs you` 會讓人以為還有事；DEMO-01 也是從 task 推算數量。工作中／閒置／卡住等其他狀態協定沒給，照舊是 G1 缺口，不在 TUI 推算 | 改 `Fleet::agent_state` 一處；第 8 施工關協定給 agent 狀態後改讀協定的值 | 待追認 |
| G1 | 缺口：沒有列出 team、task、agent 的請求，也沒有 task 的關卡清單與狀態、agent 的結構化狀態（working／idle／needs you／stuck／unknown）與 backend，也沒有「不處理的話會怎樣」（見 T16）；agent 狀態只有「需要你」由 TUI 依需要你清單重算（見 T18），其餘照 catalog，事件發生後不會更新 → 現在由 `source::Catalog`（TUI 本地型別）在 connect 時給 | 畫面要分組、要畫 `■□` 進度條、要顯示 agent 狀態，只有 `task_changed`／`instance_changed` 的文字摘要不夠 | 第 8 施工關：加 list 請求（或快照事件），`Catalog` 改用 core 型別 | 待追認 |
| G2 | 缺口：`attention_required` 沒有「解決後能放行多少工作」與「開始等待的時間」→ D36 排序時 unblocks 一律 0、用 event id 代替等待時間（最舊的在前） | 不在 TUI 自己推算 | 第 8 施工關：`AttentionRequiredData` 加兩個欄位（additive） | 待追認 |
| G3 | 缺口：不是請示的「需要你」（agent 卡住、用量上限）沒有操作，也沒有「已處理」事件 → 畫面寫明「client protocol v1 還沒有處理這一項的操作」，而且一直留在清單 | 不發明 wire 訊息 | 第 8 施工關：加操作（重試、暫停…）與清除事件 | 待追認 |
| G4 | 缺口：沒有已讀狀態 → 見 T4 | 同上 | 第 8 施工關：加已讀請求與事件，TUI 與 Telegram 共用 | 待追認 |

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-25 畫面層提前（你同意並行、放寬 D22）：`Source` 接縫 + 腳本假來源 + testkit 假 daemon socket 來源（demo 用）；首頁、需要你、team 頁、Task／Agent Detail、終端快照、`/`、`L`、斷線與重連；`cargo xtask accept tui` demo；check-deps 加 agend-tui 規則；testkit 加 `FakeDaemon::open_ask`（`feat/gate-11-tui-screens`，draft PR）。

## 下一步

```bash
~/.cargo/bin/cargo xtask accept tui
```
