# agend-tui 測試

> **TL;DR**
> - 畫面測試用 ratatui `TestBackend` 畫成文字，比對關鍵行；按鍵測試送按鍵序列比對導覽堆疊與畫面；`ClientSource` 測試經 `agend-client` 連 testkit 假 daemon 的真 socket。
> - 記住：**餵畫面的資料由真的 producer 產生**：腳本假來源與 testkit 假 daemon 發出的都是 client protocol 型別，沒有手寫 JSON（只有模擬 `event_gap` 的 proxy 送一行 daemon 的錯誤）。
> - 下一步：`~/.cargo/bin/cargo test -p agend-tui`；真 daemon 那側在 `~/.cargo/bin/cargo test -p agend --test tui_daemon`。

## 第 10 施工關驗證

真 pipeline task 的 repo／kind／agent 與 action note 由 client protocol 提供；退回修改使用文字編輯器，空理由不送。Gate 10 的程序測試驗證 daemon 的核准／退回／retry／acknowledge 規則。

`client_source` 的同時存活 labs 上限為 4，保留所有 socket／terminal 行為測試，避免 macOS 預設 256 fd 下的平行 fixture 資源耗盡；可在 `ulimit -n 180` 下重跑該 suite。

## C 段 Source 路徑（局部驗證）

`tests/full_source.rs` 使用真 holder parser 與 fake daemon 的實際 socket，驗控制交接／舊 owner 拒絕、viewport request id、舊 daemon 不啟用新能力、producer 阻塞時 UI seam 不等 ack，以及關閉後 queued input 不重送。控制回覆 mailbox 滿時明確關閉，不默默丟 ack；測試限制 producer 速率以隔離 server reply queue 與 UI mailbox。20 次 full stream 開關後，新增 reader／writer 全數退出，daemon connections 與 fd 回到基準。

App 已選用此路徑；真 daemon／holder／PTY 的單次 Source 輸入與 thread join 在 `agend --test full_terminal_contract` 通過；完整 App 的局部證據見下節；尚未認證完整 native TUI 的 fd 清理。初次 fd 檢查早於背景 view 清理，原失敗 log 保留；改為期限內等實際 fd／connection 清理，沒有放寬基準斷言。

## C 段 App 路徑（局部驗證）

`tests/full_app.rs` 的 15 個 cases 由真 holder parser 產生畫面／modes，再經 fake daemon 的實際 socket 操作。涵蓋相符尺寸才開輸入、取得控制途中 resize／捲動、三種本機退出鍵、另一視窗交接、舊能力唯讀、cells 色彩／樣式／寬字、application cursor／keypad、paste 整段拒絕、SGR／legacy／UTF-8 mouse、狀態列及區外、Shift／tracking 分流、歷史固定／淘汰、唯讀 live grid 捲動、停止與重連不恢復控制。

同時存活的 App fixture 上限為 3，全部案例仍執行；原 macOS fd 耗盡 log 保留。`terminal::native` 注入 output error 及 unwind，檢查 mouse／paste／focus／cursor reset。這是 writer 層證據，實際 Terminal／iTerm2 外觀與 unwind 的終端狀態仍要人工驗收。

`full_source` 的 overflow case 逐次等實際 producer 收件，送到 64-reply 邊界再等 worker 關閉，確認 explicit overflow；不靠五秒內送件數假設。原 `f999bbf` macOS CI 的 51／53 次送件失敗保留。

原生 `agend --test tui_daemon`／`tui_real` 已改用 C 段唯讀與完整尺寸確認。fake `tui_accept` 的輸入段亦注入同一真 parser；一般舊協定 Source 的直接輸入／錯誤配對契約仍保留。詳見 [App 驗證紀錄](../../docs/gates/gate-11c-app-validation.md)。

## 怎麼跑

```bash
~/.cargo/bin/cargo test -p agend-tui
~/.cargo/bin/cargo test -p agend --test tui_daemon   # 真 agend daemon（約 30 秒）
~/.cargo/bin/cargo xtask accept tui                  # 另外跑 demo：假 daemon 與真 daemon
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `i18n::tests` | `L` 在英文與繁中之間切換；`{}` 依序填入 |
| `source::tests` | 回答後離開「需要你」、追問後回來；依 event id 由舊到新；`attention_resolved` 拿掉項目、同一個 id 再發只有一項、依 `waiting_since` 排序；項目的 agent 是 `instance_id`、`if_ignored` 用協定的；跟著事件的 catalog 收 `instance_changed`（沒有 instance＝移除），重播的來源不收 |
| `source::client::tests` | 全貌 → `Catalog`：關卡由 `current_stage` 推、沒有 repo、`failed`／codex、agent 的 task 是它持有的未完成 task |
| `app::tests` | 按鍵轉位元組（UTF-8、`\r`、0x7f、方向鍵、`Ctrl-字母`、`Alt`）；只有 `Ctrl-]`、`Ctrl-5`、0x1D 離開輸入模式 |
| `ui::tests` | 靠右對齊、CJK 算兩格、截斷加 `…` 且右欄保留 |
| `tests/screens.rs` | 首頁、繁中在三種大小靠右對齊、team 三個 tab、Task Detail（repo 只在這裡）、Agent Detail、終端（即時、`i` 進輸入模式）、需要你展開（「不處理的話」、脈絡摘要、對話、選項、沒有操作的項目寫「沒有可用的操作」）、`/` 結果、選取反白不含邊框、底部說明只列有用的鍵、太小的終端與捲到底 |
| `tests/navigation.rs` | 每種畫面 `→` 與 `Enter` 相同、`←` 還原選取、tab、`t`、`/`、`L`、已讀不等於已解決、回答、斷線與重連（腳本假來源） |
| `tests/owner_steps.rs` | 第 11 施工關頁「你親自驗收」A2–A4 逐鍵照做（繁中） |
| `tests/client_source.rs` | `ClientSource` 對假 daemon：全貌畫出首頁（`g11-2` 經 `instance_id` 算「需要你」）、demo 的需要你清單與腳本來源一樣；別的 client 送 `retry` 後 `attention_resolved` 拿掉項目；假 daemon 延後事件時 `accepted` 之後項目仍在（P4）；沒有操作的項目與 agent 的 `forbidden`；`event_gap`（proxy）與 daemon 停掉都進斷線畫面、重連重拿全貌並回到原畫面、沒有留下 reader thread；版本不合不自動重試、`r` 試一次；終端 500 ms 內換成新畫面、每秒最多約 5 次、最後一段輸出補畫、閒著不重拿；只有終端連線被關時只重連它、不進斷線畫面、輸入模式不恢復；開關終端 20 次後只剩事件 thread、假 daemon 只剩 2 條連線；停止的 agent 顯示最後畫面、`i` 不進輸入模式、又跑起來自動變即時；空畫面只給訊息；舊 peer App 維持唯讀並提示升級；C 段輸入另走 `full_app`；codex 的 `not_supported` 留在終端連線、不變成 `retry` 的回覆；agent 打字被 `forbidden`；太高的畫面跟著最後一列、往上捲就停、標題固定；斷線時每 500 ms 才重連一次、`r` 立刻試；重連後選取的東西不在了就回到第一列；instance 變成 `failed` 時離開輸入模式、打的字不送出 |

## 用到的假實作

- `source::scripted::ScriptedSource`：記憶體事件記錄；回答規則同假 daemon；`ScriptHandle` 可以發事件、開請示、追問、加有操作的項目、讓終端印東西、讓 daemon 離線
- `agend_testkit::fake_daemon::FakeDaemon`：真的 unix socket + JSON Lines；每個 instance 的畫面、`push_terminal_bytes`、`terminal_inputs`、`hold_resolved_events`、`drop_terminal_subscribers`、`open_connections`
- `agend_testkit::contract::client::proxy::Proxy`：把一行改成 daemon 的 `event_gap`
- `examples/support/demo_daemon.rs`：把 demo 的 catalog、畫面、事件放進假 daemon（demo 與測試共用）

所有測試都不啟動子程序；假 daemon 在測試行程裡、在自己的暫存目錄。真 daemon 的測試在 `crates/agend/tests/tui_daemon.rs`（暫存 home `/tmp/g11.t-<pid>-<n>`）。

## 還沒測的

- [ ] 真終端機裡的外觀（色彩、字型）與按鍵：`Ctrl-]` 在 macOS Terminal／iTerm2、非美式鍵盤上實際送出什麼，由你親自驗收 B4 看（自動測試只驗 crossterm 的三種回報）
- [ ] 真 daemon 的請示（第 10 施工關前沒有）

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-tui
```
