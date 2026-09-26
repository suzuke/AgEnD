# agend-holder 測試

> **TL;DR**
> - 同程序測試跑 `server::serve` 加真的 PTY 與 bash；跨程序測試（真的 binary）在 `crates/agend/tests/holder_process.rs`。
> - 記住：每個 holder 一律用 `Shutdown` 停；測試不對自己沒起的 pid 送訊號。
> - 下一步：RTM-1..9 對真的 agent runtime 已在第 6 施工關跑（`crates/agend/tests/holder_runtime.rs`）。

## 怎麼跑

```bash
~/.cargo/bin/cargo test -p agend-holder
~/.cargo/bin/cargo test -p agend --test holder_process   # 跨程序
~/.cargo/bin/cargo run -q -p agend-holder --example holder_probe -- demo   # 需要先 build agend
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `pty::tests` | 控制鍵位元組；agent 環境＝`Spawn.env` + 預設 `TERM`；寫入佇列滿了回 `Busy`、不卡住 |
| `screen::tests` | 快照是純文字、去行尾空白、200 欄不換行、寬字完整；`ESC[6n` 的回覆進寫入佇列；佇列滿或沒有 agent 時丟掉 |
| `exit::tests` | 真的子程序的 exit 7 與 SIGKILL 對應到 `code`／`signal` |
| `paths::tests` | instance id 規則；socket 路徑超過 100 bytes 拒絕並印路徑與長度；lock 判斷存活 |
| `tests/server.rs` 快照與串流 | 重連後快照最後一列接上串流第一個位元組：數字連續、不漏不重 |
| `tests/server.rs` 控制鍵 | `y` 送到 bash；不認得的鍵回 `unknown_control_key`，之後的鍵計數證明沒寫任何位元組 |
| `tests/server.rs` 結束 | `exit 7` 與 `kill -KILL $$` 在每次連線都回報 |
| `tests/server.rs` 環境 | agent 只看到 `Spawn.env`、`TERM`，以及 portable-pty 固定加的 `SHELL` |
| `tests/server.rs` 連線 | 第二個 `Spawn` 回 `already_spawned` 且不動 agent；新連線接手、舊的被關；版本不合回錯誤、holder 照常服務；落後超過上限被斷線（測試用 64 KiB，正式是 1 MiB：`Config::lag_limit`） |
| `tests/server.rs` 終端查詢 | bash 送 `ESC[6n`，讀回 holder 的 `ESC[1;1R` |
| `tests/server.rs` 停止 | 忽略 HUP 的 agent 在 5 秒後被 SIGKILL；一般 agent 很快停；agent 已結束時，它留下、忽略 HUP 的子程序也被清掉；process group 不留任何程序 |
| `tests/server.rs` 上限 | `Resize` 65535／0／1001 回 `invalid_size`，1000×1000 仍很快；超過 1 MiB 的請求行回 `request_too_large` 並斷線；一個 byte 一個 byte 送的 hello 在 10 秒左右被斷線 |
| `tests/server.rs` 斷線後的 Shutdown | 落後被斷線的 client 送的 `Shutdown` 仍會停掉 holder；被取代的舊 client 送的不會 |
| `tests/server.rs` 安全網 | agent 結束且沒人連 → `Idle`；agent 還在跑時不觸發；刪掉 `AGEND_HOME` → 停 agent 並 `HomeDeleted` |
| `agend/tests/holder_process.rs::lock_probe_never_reports_a_stale_or_zero_pid_while_a_holder_starts` | verifier r1 的 s8：舊 pid 的鎖檔 + 緊密的 `is_running` 輪詢，200 次啟動：只會看到新 holder 的 pid，絕不是 0 或舊 pid，啟動也不會被探測擋掉（舊程式在第 2 次就失敗：`probe reported pid 0`） |
| `agend/tests/holder_process.rs` | 啟動器結束後 holder 還在；四次開機各是獨立程序、pid 與 socket inode 不變、計數器一直變大；重複啟動 exit 1；agent 送 TERM／HUP／INT／QUIT 給 holder 無效；真程序的安全網；路徑太長不建任何目錄 |
| `holder_probe demo` | 「你親自驗收」用的 demo（`cargo xtask accept holder` 會跑） |

## 用到的假實作

- `agend_testkit::tempdir::TempDir`（每個測試自己的短 `AGEND_HOME`）
- agent 用 `/bin/bash`、`/usr/bin/env`、`yes`、`sleep`

## 安全規則

- 每個測試有看門狗（server 120 秒、跨程序 180 秒）：超時就結束整個測試程序，不會卡到 CI 的 job 上限；等待子程序、`serve` 結束都有期限

- 停止一律 `Shutdown`；保底清理只對「這個測試從自己的 lock 檔讀到、且大於 1」的 pid 送 SIGKILL
- 絕不送訊號給 `-1`、`0`、負數（holder 自己的 `Shutdown` 例外：只送給自己 spawn 的 agent 的 process group，且 assert 大於 1）
- 檢查殘留只用唯讀的 `ps`

## 還沒測的

- [ ] 螢幕分類器 fixture 用 holder 畫面產生（需要真 backend 的原始 PTY 位元組錄製，見第 4 施工關頁「待你追認」）
- [ ] Linux 上的行為（CI 的 ubuntu 會跑同一組測試）
- [x] 對真的 agent runtime 跑 RTM-1..9（第 6 施工關：`crates/agend/tests/holder_runtime.rs`）

## 下一步

```bash
~/.cargo/bin/cargo test -p agend-holder
```
