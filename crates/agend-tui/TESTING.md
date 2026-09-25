# agend-tui 測試

> **TL;DR**
> - 畫面測試用 ratatui `TestBackend` 畫成文字，比對關鍵行；按鍵測試送按鍵序列比對導覽堆疊與畫面。
> - 記住：**餵畫面的資料由真的 producer 產生**：腳本假來源與 testkit 假 daemon 發出的都是 client protocol v1 型別，沒有手寫 JSON。
> - 下一步：`~/.cargo/bin/cargo test -p agend-tui`。

## 怎麼跑

```bash
~/.cargo/bin/cargo test -p agend-tui
~/.cargo/bin/cargo xtask accept tui          # 另外跑 demo：畫面、導覽、回答、斷線
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `i18n::tests` | `L` 在英文與繁中之間切換；`{}` 依序填入 |
| `source::tests` | 回答後離開「需要你」、追問後回來；依 event id 由舊到新 |
| `ui::tests` | 靠右對齊、CJK 算兩格、截斷加 `…` 且右欄保留 |
| `tests/screens.rs` | 首頁（需要你在最上、各 team 區塊、首頁沒有 repo）、繁中在 80×24／100×30／140×40 靠右對齊、team 三個 tab（不混別的 team 的成員）、Task Detail（repo 只在這裡）、Agent Detail、終端快照、需要你展開（脈絡摘要、對話、選項、非請示項目寫明沒有操作）、`/` 結果（含 CJK 查詢）、選取反白不含邊框、太小的終端與 70×20 捲到底 |
| `tests/navigation.rs` | 每種畫面 `→` 與 `Enter` 結果相同、`←` 回上一層並還原選取、tab 只用數字與 Tab 切、`t` 從各種列開對的 agent（沒有 agent／沒有輸出只給訊息）、`/` 的三種目的地與 `Esc`、`L` 不改狀態（小寫 `l` 不動作）、已讀不等於已解決、選項與自由文字回答、空的需要你、目前關卡開請示、`q`／`Ctrl-C`、斷線畫面與重連回到原畫面、啟動時沒有 daemon |
| `tests/daemon_source.rs` | 接 testkit 假 daemon 的真 socket：畫面與腳本假來源一字不差、即時事件、回答真的送到 daemon、終端快照、停掉 daemon 顯示斷線、換新 daemon 後重連 |

## 用到的假實作

- `source::scripted::ScriptedSource`：記憶體事件記錄；回答規則同假 daemon（`AskThread::accepts`、每次回答一個 `ask_updated`）；`ScriptHandle` 可以發事件、開請示、追問、讓 daemon 離線
- `agend_testkit::fake_daemon::FakeDaemon`：真的 unix socket + JSON Lines；`open_ask` 建立帶 task 與脈絡摘要的請示
- `examples/support/daemon_source.rs`：接假 daemon 的 `Source`（demo 與測試共用，第 8 施工關後換成 `agend-client`）

所有測試都不啟動子程序；假 daemon 在測試行程裡、在自己的暫存目錄。

## 還沒測的

- [ ] 真 daemon（第 8 施工關後）
- [ ] 終端即時串流與輸入（第 11 施工關正式接）
- [ ] 真終端機裡的外觀（色彩、字型）：由你親自驗收 A2–A5 看

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-tui
```
