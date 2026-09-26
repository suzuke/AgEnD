# agend（binary）

> **TL;DR**
> - 唯一 binary：CLI、daemon、holder、TUI、shim 都在裡面。
> - 記住：**argv[0] 分派在 `main` 第一行**；以 `git`／`kill`／`killall`／`pkill` 名稱執行時就是 shim，以 git hook 名稱（`reference-transaction`、`pre-push`…，由 `$AGEND_HOME/hooks/` 的 symlink）執行時就是 agend 的 git hook。
> - 下一步：第 9 施工關實作 CLI 命令；目前有 `--version`、`--help`、`holder`（第 4 施工關）、`daemon`（第 6 施工關）與 `debug ping|watch`（第 8 施工關）。

## 負責

- argv[0] 分派
- CLI：agent 命令與操作者命令（D17）
- `doctor`、`init`（第 9 施工關）、debug
- 第 13 施工關：服務註冊、`agend uninstall`、`agend telegram setup`（請 daemon 配對，本 crate 沒有 Telegram client）
- `holder <instance-id>`：argv[0] 分派之後、CLI 解析之前就交給 `agend_holder::run`（第 4 施工關 P1）
- `daemon`：同樣在 CLI 解析之前交給 `agend_daemon::daemon::run`，只在前景跑、要 `AGEND_HOME`（第 6 施工關 P1）；tokio runtime 在那裡面建，CLI 路徑不建
- 之後：`app` 子命令

## 不負責

- 在 argv[0] 分派前做任何事
- CLI 路徑上建 tokio runtime、讀設定、開 DB

## 模組

| 模組 | 職責 |
|---|---|
| `main` | argv[0] 分派 |
| `cli` | 參數解析與輸出；目前 `--version`、`--help` |
| `cli::agent` | agent 命令 |
| `cli::operator` | 操作者命令（workflow、team、repo、export／import…） |
| `doctor` | `agend doctor` |
| `init` | `agend init` |
| `debug` | `agend debug ping [--count N --interval MS]`（協定版本與 instance 數；daemon 重啟中重試 10 秒）、`agend debug watch`（全貌摘要＋之後的事件；斷線每 500 ms 重連、重拿全貌）；socket 由 `AGEND_HOME` 算，身分取 `AGEND_INSTANCE` |
| `setup` | 執行 `agend_core::setup` 的規則：跑探測指令、寫 unit 檔、註冊服務、安裝／移除 shim（第 13 施工關） |

## 依賴規則

- 一般依賴：所有 `agend-*` library crate（除 testkit）
- dev 依賴：`agend-testkit`；`serde_json`（shim 測試寫 binding 快照）；`libc`（測試只對自己的子程序或自己 lock 檔裡的 pid 送訊號）；`tokio`（第 8 施工關：CLP-8 在測試程序裡跑 daemon 的 server）

## 入口

- `agend --version`、`agend --help`
- `agend daemon`（前景；Ctrl-C 停 daemon，agent 繼續跑）
- `agend debug ping`、`agend debug watch`（唯讀；需要 `AGEND_HOME`）
- 以 `git` 名稱執行 → `agend_shim::run`
- 以 git hook 名稱執行（git 從 `$AGEND_HOME/hooks/` 呼叫） → `agend_shim::run`（`Tool::Hook`）

## 下一步

```bash
cargo run -p agend -- --version
```
