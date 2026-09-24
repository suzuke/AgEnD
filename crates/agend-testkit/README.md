# agend-testkit

> **TL;DR**
> - 共用測試基礎設施；只能當 dev-dependency。
> - 記住：**假實作要跑和真實作同一套契約測試**，才不會漂移（v1 #1483）。
> - 下一步：第 2 施工關實作。

## 負責

- 每個 trait 的假實作
- 契約測試套件
- 假 daemon（protocol v1）
- 假 agent：假 codex app-server、假 opencode serve、帶 hook 的假 claude
- git 與 binding 快照 fixture
- 暫存目錄

## 不負責

- production 邏輯
- 呼叫真 backend

## 模組

| 模組 | 職責 |
|---|---|
| `tempdir` | `TempDir`：唯一暫存目錄，drop 時刪除 |
| `fakes` | trait 假實作 |
| `contract` | 契約測試套件 |
| `fake_daemon` | 假 daemon |
| `fake_agent` | 假 agent 程式 |
| `git_fixture` | 暫存 repo 與 binding fixture |

## 依賴規則

- 一般依賴：`agend-core`
- 任何 crate 都不能把它當一般依賴（`cargo xtask check-deps`）

## 入口

- `agend_testkit::tempdir::TempDir`

## 下一步

```bash
cargo test -p agend-testkit
```
