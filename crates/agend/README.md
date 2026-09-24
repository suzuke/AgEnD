# agend（binary）

> **TL;DR**
> - 唯一 binary：CLI、daemon、holder、TUI、shim 都在裡面。
> - 記住：**argv[0] 分派在 `main` 第一行**；以 `git`／`kill`／`killall`／`pkill` 名稱執行時就是 shim。
> - 下一步：第 9 關實作 CLI 命令；目前只有 `--version`、`--help`。

## 負責

- argv[0] 分派
- CLI：agent 命令與操作者命令（D17）
- `doctor`、`init`、debug
- 之後：`daemon`、`holder`、`app` 子命令

## 不負責

- 在 argv[0] 分派前做任何事
- CLI 路徑上建 runtime、讀設定、開 DB

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
