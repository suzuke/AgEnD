# agend（binary）

> **TL;DR**
> - 唯一 binary：CLI、daemon、holder、TUI、shim 都在裡面。
> - 記住：**argv[0] 分派在 `main` 第一行**；以 `git`／`kill`／`killall`／`pkill` 名稱執行時就是 shim。
> - 下一步：第 9 施工關實作 CLI 命令；目前有 `--version`、`--help` 與 `holder`（第 4 施工關）。

## 負責

- argv[0] 分派
- CLI：agent 命令與操作者命令（D17）
- `doctor`、`init`（第 9 施工關）、debug
- 第 13 施工關：服務註冊、`agend uninstall`、`agend telegram setup`（請 daemon 配對，本 crate 沒有 Telegram client）
- `holder <instance-id>`：argv[0] 分派之後、CLI 解析之前就交給 `agend_holder::run`（第 4 施工關 P1）
- 之後：`daemon`、`app` 子命令

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
| `debug` | 除錯子命令 |
| `setup` | 執行 `agend_core::setup` 的規則：跑探測指令、寫 unit 檔、註冊服務、安裝／移除 shim（第 13 施工關） |

## 依賴規則

- 一般依賴：所有 `agend-*` library crate（除 testkit）
- dev 依賴：`agend-testkit`

## 入口

- `agend --version`、`agend --help`
- 以 `git` 名稱執行 → `agend_shim::run`

## 下一步

```bash
cargo run -p agend -- --version
```
