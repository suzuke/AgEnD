# agend（binary）

> **TL;DR**
> - 唯一 binary：CLI、daemon、holder、TUI、shim 都在裡面。
> - 記住：**argv[0] 分派在 `main` 第一行**；以 `git`／`gh`／`kill`／`killall`／`pkill` 名稱執行時就是 shim，以 git hook 名稱（`reference-transaction`、`pre-push`…，由 `$AGEND_HOME/hooks/` 的 symlink）執行時就是 agend 的 git hook。
> - 下一步：第 10 施工關驗證：`cargo xtask accept pipeline`。操作員 home 預設 `$HOME/.agend`，可用 `AGEND_HOME` 覆寫；agent 必須有明確 home。

## 第 10 施工關（已驗收，2026-10-02）

`team add|list|join|set-workflow`、`workflow list|show|check|apply`、operator／agent task create、task cancel（選填 `--reason`）與 agent 流水線回報已接通；`doctor` 加入真 sandbox probe。操作見 [pipeline runtime](../../docs/architecture/pipeline-runtime.md)。

## 第 11 施工關 C 段（已驗收並合併 #145）

真 App 的完整畫面、鍵鼠／paste／歷史、多視窗、resize、正常／unwind 還原與 20 次程序清理已有原生回歸。完整 fake U17 與真 Codex 0.159.3 首次 U17 已由全新 verifier 核實；使用者同意只開放 0.159.3，使用者已有實機紀錄並要求剩餘行為自動驗證；最終 head verifier／CI、清理與已確認合併紀錄見 [驗收收尾](../../docs/gates/gate-11c-closeout.md)。 [版本政策](../../docs/gates/gate-11c-codex-input.md)。

## 負責

- argv[0] 分派
- CLI：agent 命令與操作者命令（D17）
- `doctor`、`init`（第 9 施工關）、debug
- `home::resolve`：操作員可預設 `$HOME/.agend`，明確 `AGEND_HOME` 必須為非空絕對路徑、有 `fleet.yaml`（v1）就拒絕；CLI、`agend daemon`、`doctor`、`init`、`debug` 共用（第 9 施工關 P3）
- 第 13 施工關：服務註冊、`agend uninstall`、`agend telegram setup`（請 daemon 配對，本 crate 沒有 Telegram client）
- `holder <instance-id>`：argv[0] 分派之後、CLI 解析之前就交給 `agend_holder::run`（第 4 施工關 P1）
- `daemon`：同樣在 CLI 解析之前交給 `agend_daemon::daemon::run`，只在前景跑、共用 `home::resolve`（第 6 施工關 P1）；tokio runtime 在那裡面建，CLI 路徑不建
- `app [--lang en|zh-TW]`（第 11 施工關 B 段）：`agend_tui::run` + `ClientSource`；home 與 caller 同 CLI（`home::resolve`、`AGEND_INSTANCE`）；stdout 不是終端機 → `agend app needs a terminal`、exit 2

## 不負責

- 在 argv[0] 分派前做任何事
- 一般 agent／operator CLI 路徑上建 tokio runtime、讀設定、開 DB；`doctor` 為 checks readiness 建短暫 runtime，`init`／preflight 的 DB 檢查也屬例外

## 模組

| 模組 | 職責 |
|---|---|
| `main` | argv[0] 分派 |
| `cli` | `clap`（derive、無顏色）解析、`--json`、錯誤碼與 exit code（0／1／2）；`--version` 在 clap 之前回（啟動 < 10 ms）；`--help` 先列範例 |
| `cli::agent` | agent 命令：`status`、`send`（UUID v4 訊息 id、`--level`、斷線用同一個 id 重送）、`inbox [--after]`、`done`／`result`／`review` 帶 ticket、`ask`、`block`／`unblock`／`remind`（先問 `status` 拿 task）、`task create` |
| `cli::operator` | 操作者命令：`status`（全貌）、`instance add|remove|list`、`daemon restart`（預檢結果、等舊連線 EOF、等新的 `boot_id`）、`task cancel` |
| `home` | `AGEND_HOME` 的解析（見上） |
| `doctor` | `agend doctor`：home、daemon、git、claude／codex／opencode、holders、disk、Telegram 本機設定（空 allowlist 為 fail）；非 ok 一定附 `fix:`；有 fail 就 exit 1 |
| `init` | `agend init`：建 home（0700，已存在不動）與完整 config.toml（0600、不覆寫既有檔案、拒絕 symlink）→ doctor → 下一步 |
| `debug` | `agend debug ping [--count N --interval MS]`（協定版本與 instance 數；daemon 重啟中重試 10 秒）、`agend debug watch`（全貌摘要＋之後的事件；斷線每 500 ms 重連、重拿全貌）；socket 由解析後的 home 算，身分取 `AGEND_INSTANCE` |
| `setup` | 執行 `agend_core::setup` 的規則：在 `PATH` 上找程式（跳過 `$AGEND_HOME/bin`）、跑 `--version`（5 秒）、`statvfs`、home 大小；第 13 施工關加寫 unit 檔、註冊服務、安裝／移除 shim |

## 依賴規則

- 一般依賴：所有 `agend-*` library crate（除 testkit）；`clap`（derive，不開預設功能：沒有顏色、沒有建議）；`libc`（doctor 的 `statvfs`、`access`）；`serde`、`serde_json`（`--json`）
- dev 依賴：`agend-testkit`；`libc`（測試只對自己的子程序或自己 lock 檔裡的 pid 送訊號）；`tokio`（第 8 施工關：CLP-8 在測試程序裡跑 daemon 的 server）；`rusqlite`（第 7 施工關：做一個 schema v3 的 `agend.db`，放第 6 施工關的 codex 列）
- dev 依賴另含 `portable-pty`（與 holder 同為 0.9，沒有新增 package）：只用於真 `agend app` 的外層 PTY，讀 kernel termios、送 resize 與收實際輸出。
- example `fake_codex`：只給測試用的 `codex` CLI 替身（`agend_testkit::fake_agent::codex_cli`），`cargo test -p agend` 會一起編到 `target/<profile>/examples/fake_codex`；不安裝
- example `cli_demo`：第 9 施工關的 demo（`cargo xtask accept cli`），跟 `tests/cli.rs` 共用 `tests/common/`

## 入口

- `agend --version`、`agend --help`（每個子命令 `--help` 先列範例）
- agent 命令（`AGEND_INSTANCE` 有設）：`status`、`send <to> "<message>" [--level queue|steer|interrupt]`、`inbox [--after <message-id>]`、`done <ticket>`、`result <ticket> "<summary>"`、`review approve|changes <ticket> …`、`ask "<question>" [--option …]`、`block "<reason>"`、`unblock`、`remind <90s|30m|2h>`、`task create --role <role> "<title>"`
- 操作者命令：`status`、`instance add <name> <backend> [--dir <path>] [--program <path>] [-- <args>…]`、`instance remove <name> [--yes]`、`instance list`、`daemon restart [--binary <path>]`、`task cancel <task>`、`task create … --team <team>`、`doctor`、`init`
- `agend daemon`（前景；Ctrl-C 停 daemon，agent 繼續跑）；`agend daemon preflight <dir>`（重啟中的 daemon 自己跑，不是給人用的）
- `agend debug ping`、`agend debug watch`（唯讀；共用操作員預設 home）
- 以 `git` 名稱執行 → `agend_shim::run`
- 以 git hook 名稱執行（git 從 `$AGEND_HOME/hooks/` 呼叫） → `agend_shim::run`（`Tool::Hook`）

watch 的 task_changed 顯示事件 TaskView 的 current_stage；舊 peer 未帶該欄位時仍顯示原摘要。

## 第 13B 使用者服務（施工中）

`agend service plan [--manager launchd|systemd] [--json]` 產生可審閱的 user-service 定義與目的路徑；不寫檔、不呼叫服務管理器。daemon 執行檔規劃保存於 home 的 `service/agend`，避免綁定施工用 target。launchd 保留 process group，systemd 用 `KillMode=process`；`service install --no-start` 可先發布自有定義與 receipt；註冊、status 和預設保留資料的 uninstall 已接上，仍待完整真服務驗收。systemd 的操作另核對已載入的 argv／環境／drop-in／停止行為，並核對 live MainPID 的 executable、argv、home 與程序世代；enable／disable 使用 `--no-reload`，避免驗證後隱含載入其他設定。

## 下一步

```bash
cargo run -p agend -- --version
```

## 第 12A Claude helpers

同一 binary 的 `agend channel --instance <id>` 與 `agend hook <event>` 在一般 clap 前分派；需要 AGEND_HOME／AGEND_INSTANCE，只經同步 client 1.5，沒有 SQLite 或 Tokio runtime。channel 是 MCP JSON-RPC stdio，提供 agend_ack；hook 先保存事件，Stop 離線回 `{}`。owned spool 原子發布／fsync，入庫才刪。`agend hooks` 的 git hook 管理維持原入口。Claude push 啟動設定、Driver 及 task／review 已接入施工分支；inbox 路徑不套設定。ACK 保留 pipeline 的原 dispatch id，delivery／session 核對 UUID v4；task 完成不代替 ACK。啟動提示與選定真 CLI 尚待驗收，見 [Driver 進度](../../docs/gates/gate-12a-driver.md)。

2026-10-06 啟動診斷取得四份相同真 frame，結果 CAPTURED、零工作訊息。
新增完整 Ready 提示 fixture 的 native P6 回歸只跑 shell producer／真 daemon／holder，
核 SessionStart 先到、三鍵完成後穩定五秒、Ready 不加鍵及 halted=1；不啟動真 Claude。
[第二次完整 smoke](../../docs/gates/gate-12a-observed-smoke-v2.md)亦在 initial idle 逾時：A idle、B unknown，24 份 frame、零工作訊息。
新增已錄製的 how-does 完整 Ready literal 與同一 native P6 回歸；未知提示仍拒絕。
新真 CLI 計畫仍需另行授權；native 通過不等於模型通訊通過。

2026-10-06 [D41](../../docs/decisions/d41.md) 允許 Ready 建議內容可變，其他完整畫面／版本／路徑／尺寸不變。
`claude_startup::startup_variable_ready_suggestions_replay_actual_v5_and_both_widths` 經真 daemon／holder／PTY 重播 v5 兩份捕獲及 140 欄變體，核五秒初始 idle 與 Ready 不加鍵；
`startup_variable_ready_rejects_unknown_footer_and_split_hint_without_idle_or_more_keys` 拒絕未知 footer／分行建議。
既有無 SessionStart、人工控制、結果不明與四次開機回歸維持；這些測試不啟動真 Claude、不送模型訊息。

明確資料刪除使用 `uninstall --delete-data --confirm-home <canonical home>`；預設保留資料，拒絕仍有 Git workspace 或跨掛載的 home。鎖檔保留、並行與重試限制見[服務安裝](../../docs/architecture/service-install.md)。

13C 施工中的 `backend import`／`backend inspect` 提供操作員匯入原生 executable 與內容核對；不執行、不啟用、不宣稱 canary 通過。介面與限制見[版本管理](../../docs/architecture/backend-versions.md)。

13C `backend switch prepare <instance> --version <version> [--previous <id>]` 經 daemon 驗目標 canary 並暫停新投遞；`status <instance>` 讀持久紀錄，`cancel <instance> --switch-id <id>` 精確取消並恢復。要求 client protocol 1.8，只允許操作員；逾時查 status，不重送 mutation。prepare 尚不停止 holder 或啟用新版，完整切換／回滾仍施工中。

13C 新增 `backend switch activate／rollback --switch-id`：精確持久 ID、目的版本准入、投遞排空與 native idle 後停止受管 holder；Committed／Restoring 保持暫停，核新 holder 綁定及 readiness 後才釋放。Activated／Committed 回滾先保存 RollbackPrepared；daemon 重啟後由定期協調器繼續。三 backend 原生假版本往返與目的 holder 消失後自動回退已驗；目前代原生 AgentExited 亦可核精確 holder 後回退，Codex 已驗。driver 斷線不當作退出；完整 crash matrix 尚未完成。

換版的 `problem` 保存具體失敗原因與首次等待時間；driver 斷線、啟動失敗或自動回退被拒時，在「需要你」顯示 `backend-switch:<instance>:<switch-id>`，並由 `backend switch status` 顯示原因。daemon 重啟恢復同一通知，pipeline 同步不移除它。通知沒有一般 Retry 動作；操作員先查狀態，仍循正式換版／回退入口。成功啟用、回退、取消或移除 instance 才清除通知。這不代表斷線本身授權停止存活 holder；啟動逾時政策仍待完成。

目的版本提交為 Committed 或 Restoring 時，同一交易保存 300 秒 activation deadline；重啟、重複觀察及問題通知都不重新計時，真正開始還原才建立新的期限。到期且仍未通過 native readiness，下一次協調檢查保存逾時問題、保持投遞暫停與原 holder。到期不等於閒置，不授權強制停止；稍後通過 readiness 仍可完成並清除通知／期限。舊持久紀錄若沒有 deadline，未完成時明確提示缺少期限，不能當作重新獲得五分鐘。

13D 新增 `agend telegram setup begin --token-env NAME`（或 `--token-file /absolute/private/file`）、`status`、`poll --id ID`、`confirm --id ID --chat CHAT --user USER [--topic TOPIC]`、`cancel --id ID`。token 只由 daemon 解析，CLI 不讀秘密或聯絡 Telegram；操作不重送，斷線查 status。確認保存收據；再執行 `agend telegram setup apply --id ID`，由操作員 CLI 保留原文、備份並加入 Telegram 設定。不同的既有 Telegram 設定拒絕覆寫，相同設定重跑不變；套用後需重啟 daemon 生效。

13E `install_home` 現在以真 CLI doctor JSON 驗故障與恢復：缺 home／權限過寬、Telegram 損壞且原文不變、sandbox 工具缺失及原生恢復、三 backend PATH 遺失及只允許 --version 的 fixture 恢復；每個非 ok 結果須有 fix，exit code 必須對應所有 checks 的 fail。這不涵蓋磁碟容量、登入、版本相容或真服務健康的完整驗收。

doctor 的 service 列唯讀檢查安裝收據、私有 executable／定義與 manager 狀態；沒有安裝仍可前景執行。service running 只代表 manager 的觀察，daemon 就緒另由 daemon 列判斷。它不取得安裝鎖，不進行 start／stop／reload，檔案異動時保留原檔並要求檢查。
