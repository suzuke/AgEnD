# AgEnD v2

> **TL;DR**
> - 這是 AgEnD（Agent Engineering Daemon）v2：異質 agent 團隊的自主 merge 流水線。
> - 狀態：**pre-alpha，設計階段**。目前只有 crate 骨架與設計文件，沒有可用功能。
> - 下一步：先讀 [AGENTS.md](AGENTS.md)，再照 [docs/ROADMAP.md](docs/ROADMAP.md) 開第 1 施工關。

## 這是什麼

一個常駐的 daemon，讓 claude、codex、opencode 組成的 agent 團隊自己完成「派工 → 開發 → checks → 互審 → merge」。

人只處理例外：請示、門檻卡住、agent 卡住。

不拼終端多工器、不拼支援的 agent 數量、不拼手機 App。

| 差異化 | 怎麼做 |
|---|---|
| 機制保證的 merge 門檻 | 核准綁 head SHA + checks + git shim；需要人工時在 workflow 加 `approval(by = "human")` |
| 跨廠牌 | claude / codex / opencode 互審；額度用盡時改派給別家 |
| 派工時就避免衝突 | daemon 知道每個 task 動到的檔案；merge 前 rebase、依序合併 |
| git 防護內建 | shim 只進 agent 的 PATH，擋 agent 自建 branch/worktree 與改 main |

第一版範圍：macOS + Linux；backend 為 claude / codex / opencode。

## 系統圖

![AgEnD v2 系統架構](docs/images/system.svg)

agent 側沒有任何 daemon 子程序；agent 與附屬程序都由 holder 持有，所以 daemon 可以隨時重啟或升級；daemon、holder、shim 都執行已安裝的 release 版，開發中的 AgEnD 在另一個 clone。

圖註：

- Telegram 與 GitHub 不走 protocol v1，而是由 daemon 的 `notifier`、`forge` adapter 連出去；圖上以「經 notifier」「經 forge」標示。
- 「CLI、hook、結構化事件」虛線是三個 agent 共用的回報路徑（圖上從最右邊畫出，代表全部）；hook 只有 claude 有。
- 這張圖是唯一版本：`docs/images/system.svg`（手寫 SVG，直接改這個檔案；顏色與字級在檔頭 `<style>`，會依 GitHub 淺色／深色主題切換）。其他文件只連結到這裡。

## Repo 結構

| 路徑 | 內容 |
|---|---|
| `crates/agend-core` | 純邏輯：型別、protocol、trait、流水線狀態機、policy、螢幕分類器 |
| `crates/agend-daemon` | 唯一的大型 I/O 層：入口、領域、adapter |
| `crates/agend-holder` | 每個 instance 一個：PTY、畫面、附屬程序 |
| `crates/agend-shim` | git 與 kill 防護；只讀 binding 快照 |
| `crates/agend-client` | 同步 I/O 連 daemon、重試、版本檢查 |
| `crates/agend-tui` | attention-first TUI |
| `crates/agend` | 唯一 binary：argv[0] 分派、CLI |
| `crates/agend-testkit` | dev-only：假實作、契約測試、假 daemon、假 agent |
| `xtask/` | `cargo xtask check-deps`、`cargo xtask accept <施工關>` |
| `docs/` | 架構、決策、backend 行為、v1 教訓、施工順序 |

每個 crate 目錄都有 `README.md`（負責什麼）與 `TESTING.md`（怎麼測）。

## 建置與測試

需要 Rust 1.96.0（`rust-toolchain.toml` 已釘住；rustup 會自動安裝）。

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo xtask check-deps
```

## 文件地圖

| 想知道 | 讀 |
|---|---|
| 怎麼開始工作 | [AGENTS.md](AGENTS.md) |
| 名詞怎麼用 | [docs/GLOSSARY.md](docs/GLOSSARY.md) |
| 系統怎麼組成 | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| 決定了什麼、為什麼 | [docs/DECISIONS.md](docs/DECISIONS.md) |
| 三個 backend 實測行為 | [docs/BACKEND-BEHAVIORS.md](docs/BACKEND-BEHAVIORS.md) |
| v1 踩過的坑 | [docs/V1-LESSONS.md](docs/V1-LESSONS.md) |
| 施工順序與驗收 | [docs/ROADMAP.md](docs/ROADMAP.md) |

## 授權

Apache-2.0，見 [LICENSE](LICENSE) 與 [NOTICE](NOTICE)。

## 下一步

```bash
cat AGENTS.md
cargo test --workspace && cargo xtask check-deps
```
