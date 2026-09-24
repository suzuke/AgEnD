# AGENTS.md — 所有 agent 的唯一入口

> **TL;DR**
> - 這是 AgEnD v2 的工作入口；人和任何 AI agent（Claude、Codex、opencode…）都從這裡開始。本 repo 不設 CLAUDE.md。
> - 記住：**crate 邊界是架構**；改動前後都跑 `cargo xtask check-deps`。
> - 下一步：照「先讀什麼」讀三份文件，然後看「目前狀態」。

## 專案目的

AgEnD（Agent Engineering Daemon）v2：異質 agent 團隊（claude／codex／opencode）的自主 merge 流水線。
daemon 負責派工、worktree、checks、互審綁 head、merge；人只處理例外。

## 先讀什麼（依序）

1. [README.md](README.md)：是什麼、系統圖、repo 結構。
2. [docs/ROADMAP.md](docs/ROADMAP.md)：12 關、目前在哪一關、完成定義。
3. [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)：程序模型、crate 地圖、daemon 分層。
4. 你要動的 crate 的 `README.md` 與 `TESTING.md`。
5. 需要時：[docs/DECISIONS.md](docs/DECISIONS.md)、[docs/BACKEND-BEHAVIORS.md](docs/BACKEND-BEHAVIORS.md)、[docs/V1-LESSONS.md](docs/V1-LESSONS.md)。

## Repo 地圖

| 路徑 | 內容 |
|---|---|
| `crates/agend-core` | 純邏輯；所有 crate 的根 |
| `crates/agend-daemon` | 唯一的大型 I/O 層 |
| `crates/agend-holder` | 每個 instance 一個：PTY、畫面、附屬程序 |
| `crates/agend-shim` | git／kill 防護 |
| `crates/agend-client` | 同步 I/O 連 daemon |
| `crates/agend-tui` | attention-first TUI |
| `crates/agend` | 唯一 binary |
| `crates/agend-testkit` | dev-only 測試基礎設施 |
| `xtask/` | `check-deps`、`accept <關>` |
| `docs/` | 設計文件；`docs/decisions/`、`docs/backends/`、`docs/architecture/` 是分頁 |

## Crate 邊界規則

| 規則 | 怎麼強制 |
|---|---|
| `agend-core` 不依賴 tokio、rusqlite、任何 process／network crate、其他 `agend-*`；不用 `std::process`、`std::net` | `cargo xtask check-deps` |
| `agend-shim`、`agend-client` 不依賴 async runtime、SQLite、`agend-daemon`（啟動要輕） | `cargo xtask check-deps` |
| `agend-testkit` 只能當 dev-dependency | `cargo xtask check-deps` |
| 模組之間只透過 `agend_core` 的 trait 與型別溝通 | code review |
| 只有符合四條準則才新增 crate（見 ARCHITECTURE） | code review |

CI（`.github/workflows/ci.yml`）在 ubuntu 與 macOS 跑同一組檢查。

## 指令

```bash
cargo build --workspace
cargo test --workspace
cargo test -p <crate>                                  # 單一 crate
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all                                        # 或 -- --check
cargo xtask check-deps
cargo xtask accept <關>                                # 1–12 或名稱，如 core
```

工具鏈釘在 `rust-toolchain.toml`（1.96.0，含 rustfmt、clippy）。

## 一個改動的完成定義

- [ ] `cargo fmt --all -- --check` 通過
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 通過
- [ ] `cargo test --workspace` 通過（至少 `cargo test -p <改到的 crate>`）
- [ ] `cargo xtask check-deps` 通過
- [ ] 行為變了 → 對應的 crate `README.md`／`TESTING.md` 與 `docs/` 已更新

一關的完成定義（多了 `accept` demo、verifier、使用者確認）見 [docs/ROADMAP.md](docs/ROADMAP.md#每關的完成定義)。

## 寫作與程式規則

| 項目 | 規則 |
|---|---|
| 文件 | 繁體中文為主；程式識別字、指令、路徑、技術名詞保留原文 |
| 程式輸出 | 英文：CLI 輸出、錯誤訊息、log、`--help`、測試名稱與 assert 訊息 |
| 程式註解與 `//!` | 英文 |
| 文件格式 | 開頭 3 行 TL;DR；先結論；表格與清單；深入背景放「細節」；需要動作的文件以「下一步」結尾；超過約 150 行就拆 |
| 測試 | 測 consumer 時用真的 producer 產生輸入，不手寫格式（v1 #1493） |
| 骨架 | 不寫假實作或佔位邏輯；還沒決定的東西只寫 `//!` 說明 |

## 決策在哪

- 已確認的決策：[docs/DECISIONS.md](docs/DECISIONS.md)（D1–D23）。
- 沒有新證據不要重開討論。要推翻：先補證據，再提新的決策編號，由使用者確認。
- 文件間衝突時：後來的決策優先於規劃本文。

## 目前狀態

- 第 0 階段（backend spike）完成。
- 骨架與設計文件完成（本 commit set）。
- 第 1 關（agend-core）尚未開始，需要使用者確認後開始。

## 下一步

```bash
cat docs/ROADMAP.md
cat crates/agend-core/README.md crates/agend-core/TESTING.md
cargo xtask accept core
```
