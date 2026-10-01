# agend（binary）

> **TL;DR**
> - 唯一 binary：CLI、daemon、holder、TUI、shim 都在裡面。
> - 記住：**argv[0] 分派在 `main` 第一行**；以 `git`／`kill`／`killall`／`pkill` 名稱執行時就是 shim，以 git hook 名稱（`reference-transaction`、`pre-push`…，由 `$AGEND_HOME/hooks/` 的 symlink）執行時就是 agend 的 git hook。
> - 下一步：第 9 施工關的 CLI（agent 命令、`instance add|remove|list`、`daemon restart`、`status`、`doctor`、`init`）draft PR：`cargo xtask accept cli`。每個命令都要 `AGEND_HOME`（沒有預設，第 13 施工關再定）。

## 負責

- argv[0] 分派
- CLI：agent 命令與操作者命令（D17）
- `doctor`、`init`（第 9 施工關）、debug
- `home::resolve`：`AGEND_HOME` 一律必須設、要絕對路徑、有 `fleet.yaml`（v1）就拒絕；CLI、`agend daemon`、`doctor`、`init`、`debug` 共用（第 9 施工關 P3）
- 第 13 施工關：服務註冊、`agend uninstall`、`agend telegram setup`（請 daemon 配對，本 crate 沒有 Telegram client）
- `holder <instance-id>`：argv[0] 分派之後、CLI 解析之前就交給 `agend_holder::run`（第 4 施工關 P1）
- `daemon`：同樣在 CLI 解析之前交給 `agend_daemon::daemon::run`，只在前景跑、要 `AGEND_HOME`（第 6 施工關 P1）；tokio runtime 在那裡面建，CLI 路徑不建
- `app [--lang en|zh-TW]`（第 11 施工關 B 段）：`agend_tui::run` + `ClientSource`；home 與 caller 同 CLI（`home::resolve`、`AGEND_INSTANCE`）；stdout 不是終端機 → `agend app needs a terminal`、exit 2

## 不負責

- 在 argv[0] 分派前做任何事
- CLI 路徑上建 tokio runtime、讀設定、開 DB

## 模組

| 模組 | 職責 |
|---|---|
| `main` | argv[0] 分派 |
| `cli` | `clap`（derive、無顏色）解析、`--json`、錯誤碼與 exit code（0／1／2）；`--version` 在 clap 之前回（啟動 < 10 ms）；`--help` 先列範例 |
| `cli::agent` | agent 命令：`status`、`send`（UUID v4 訊息 id、`--level`、斷線用同一個 id 重送）、`inbox [--after]`、`done`／`result`／`review` 帶 ticket、`ask`、`block`／`unblock`／`remind`（先問 `status` 拿 task）、`task create` |
| `cli::operator` | 操作者命令：`status`（全貌）、`instance add|remove|list`、`daemon restart`（預檢結果、等舊連線 EOF、等新的 `boot_id`）、`task cancel` |
| `home` | `AGEND_HOME` 的解析（見上） |
| `doctor` | `agend doctor`：home、daemon、git、claude／codex／opencode、holders、disk；非 ok 一定附 `fix:`；有 fail 就 exit 1 |
| `init` | `agend init`：建 home（0700，已存在不動）→ doctor → 下一步 |
| `debug` | `agend debug ping [--count N --interval MS]`（協定版本與 instance 數；daemon 重啟中重試 10 秒）、`agend debug watch`（全貌摘要＋之後的事件；斷線每 500 ms 重連、重拿全貌）；socket 由 `AGEND_HOME` 算，身分取 `AGEND_INSTANCE` |
| `setup` | 執行 `agend_core::setup` 的規則：在 `PATH` 上找程式（跳過 `$AGEND_HOME/bin`）、跑 `--version`（5 秒）、`statvfs`、home 大小；第 13 施工關加寫 unit 檔、註冊服務、安裝／移除 shim |

## 依賴規則

- 一般依賴：所有 `agend-*` library crate（除 testkit）；`clap`（derive，不開預設功能：沒有顏色、沒有建議）；`libc`（doctor 的 `statvfs`、`access`）；`serde`、`serde_json`（`--json`）
- dev 依賴：`agend-testkit`；`libc`（測試只對自己的子程序或自己 lock 檔裡的 pid 送訊號）；`tokio`（第 8 施工關：CLP-8 在測試程序裡跑 daemon 的 server）；`rusqlite`（第 7 施工關：做一個 schema v3 的 `agend.db`，放第 6 施工關的 codex 列）
- example `fake_codex`：只給測試用的 `codex` CLI 替身（`agend_testkit::fake_agent::codex_cli`），`cargo test -p agend` 會一起編到 `target/<profile>/examples/fake_codex`；不安裝
- example `cli_demo`：第 9 施工關的 demo（`cargo xtask accept cli`），跟 `tests/cli.rs` 共用 `tests/common/`

## 入口

- `agend --version`、`agend --help`（每個子命令 `--help` 先列範例）
- agent 命令（`AGEND_INSTANCE` 有設）：`status`、`send <to> "<message>" [--level queue|steer|interrupt]`、`inbox [--after <message-id>]`、`done <ticket>`、`result <ticket> "<summary>"`、`review approve|changes <ticket> …`、`ask "<question>" [--option …]`、`block "<reason>"`、`unblock`、`remind <90s|30m|2h>`、`task create --role <role> "<title>"`
- 操作者命令：`status`、`instance add <name> <backend> [--dir <path>] [--program <path>] [-- <args>…]`、`instance remove <name> [--yes]`、`instance list`、`daemon restart [--binary <path>]`、`task cancel <task>`、`task create … --team <team>`（第 10 施工關前回 `not_supported`）、`doctor`、`init`
- `agend daemon`（前景；Ctrl-C 停 daemon，agent 繼續跑）；`agend daemon preflight <dir>`（重啟中的 daemon 自己跑，不是給人用的）
- `agend debug ping`、`agend debug watch`（唯讀；需要 `AGEND_HOME`）
- 以 `git` 名稱執行 → `agend_shim::run`
- 以 git hook 名稱執行（git 從 `$AGEND_HOME/hooks/` 呼叫） → `agend_shim::run`（`Tool::Hook`）

## 下一步

```bash
cargo run -p agend -- --version
```
