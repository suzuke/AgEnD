# AGENTS.md — 所有 agent 的唯一入口

> **TL;DR**
> - 這是 AgEnD v2 的工作入口；人和任何 AI agent（Claude、Codex、opencode…）都從這裡開始。本 repo 不設 CLAUDE.md。
> - 記住：**crate 邊界是架構**；改動前後都跑 `cargo xtask check-deps`。
> - 下一步：照「先讀什麼」讀前四份文件，然後看「目前狀態」。

## 專案目的

AgEnD（Agent Engineering Daemon）v2：異質 agent 團隊（claude／codex／opencode）的自主 merge 流水線。
daemon 負責派工、worktree、checks、互審綁 head、merge；人只處理例外。

## 先讀什麼（依序）

1. [README.md](README.md)：是什麼、系統圖、repo 結構。
2. [docs/GLOSSARY.md](docs/GLOSSARY.md)：名詞表；人和 agent 用同一套詞（例如「關卡」≠「施工關」）。
3. [docs/ROADMAP.md](docs/ROADMAP.md)：13 個施工關、目前在哪個施工關、完成定義、進度紀錄；目前這個施工關的頁面在 [docs/gates/](docs/gates/README.md)。
4. [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)：程序模型、crate 地圖、daemon 分層。
5. 你要動的 crate 的 `README.md` 與 `TESTING.md`。
6. 需要時：[docs/DECISIONS.md](docs/DECISIONS.md)、[docs/BACKEND-BEHAVIORS.md](docs/BACKEND-BEHAVIORS.md)、[docs/V1-LESSONS.md](docs/V1-LESSONS.md)。
7. 選讀，最後才看：[docs/research/](docs/research/README.md)，原始證據；要質疑或重新驗證某個決定時才查。

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
| `xtask/` | `check-deps`、`accept <施工關>` |
| `docs/` | 設計文件；`docs/decisions/`、`docs/backends/`、`docs/architecture/` 是分頁 |
| `docs/gates/` | 13 個施工關各一頁：範圍、驗收步驟（含你親自驗收）、驗收紀錄、進度紀錄 |

## Crate 邊界規則

| 規則 | 怎麼強制 |
|---|---|
| `agend-core` 不用 std（`#![no_std]` + `alloc`），所以沒有檔案、程序、網路、環境變數、thread、stdio、時鐘；時間只經 `Clock` trait | `cargo xtask check-deps` 對無 std 的 target 編譯 core |
| `agend-core` 沒有 unsafe（擋 FFI） | `cargo xtask check-deps` 的無 std 編譯帶 `-F unsafe-code`；原始碼的 `#![forbid(unsafe_code)]` 給 IDE 即時提示 |
| `agend-core` 沒有 build script、沒有 `[features]`；唯一直接依賴是停用預設功能且只開 `derive` + `alloc` 的 `serde`（D32，第 1 施工關提案 P7） | `cargo xtask check-deps`（`cargo metadata`） |
| `agend-shim`、`agend-client` 不依賴 async runtime、SQLite、`agend-daemon`（啟動要輕） | `cargo xtask check-deps` |
| `agend-holder` 不依賴 async runtime、SQLite、`agend-daemon`（一跑好幾天，只用 std thread；第 4 施工關 P1） | `cargo xtask check-deps` |
| `agend-tui` 不依賴 SQLite、`agend-daemon`（只經 daemon protocol 讀資料，第 5 施工關 P2；不擋 async runtime——crossterm 帶 `mio`，第 11 施工關 T9） | `cargo xtask check-deps` |
| `agend-testkit` 只能當 dev-dependency | `cargo xtask check-deps` |
| `agend-daemon` 不依賴 `agend-holder`、`agend-shim`（daemon 只經 holder 協定和子命令跟它們互動，第 6 施工關 P9） | `cargo xtask check-deps` |
| `agend-daemon` 不依賴 `agend-client`（server 和 client 分開寫協定，讓 CLP 契約抓到兩邊不一致，第 8 施工關 P10） | `cargo xtask check-deps` |
| 模組之間只透過 `agend_core` 的 trait 與型別溝通 | code review |
| 只有符合四條準則才新增 crate（見 ARCHITECTURE） | code review |

這些保護擋的是意外，不是刻意繞過；刻意的改動靠 code review。

CI（`.github/workflows/ci.yml`）在 ubuntu 與 macOS 跑同一組檢查。

`SKIPPED` 代表**沒有驗證**，不是通過。

- 本機要驗證：`rustup target add thumbv7em-none-eabihf`，再跑 `~/.cargo/bin/cargo xtask check-deps`（不加 `--allow-skip`）。Homebrew 的 `cargo` 沒有額外 target，一定會 SKIPPED。
- `--allow-skip` 只在你明白這一項沒驗證時用；它仍印出 SKIPPED。
- CI 一定會跑這一項（不加 `--allow-skip`）。

## Git 工作流程（必守）

> **TL;DR** 所有實作都在自己的 branch + git worktree 上做，用 PR 合併；**絕不直接改整合 branch**。

- 整合 branch：**`v2`**（`main` 目前是舊版 TypeScript；切換後整合 branch 改為 `main`，規則不變）。
- 禁止：在 `v2` 或 `main` 上直接 commit、push、`--force`；在別人的 worktree 裡改東西。
- 一律用 PR 合併；合併方式與時機由使用者決定，agent 不自行 merge。

開工：

```bash
git fetch origin
git worktree add ../AgEnD-<主題> -b <類型>/<主題> origin/v2   # 類型：feat／fix／docs／test／build
cd ../AgEnD-<主題>
```

收工：

```bash
git push -u origin <類型>/<主題>
gh pr create --base v2
# PR 合併後
git worktree remove ../AgEnD-<主題> && git branch -d <類型>/<主題>
```

## 指令

```bash
cargo build --workspace
cargo test --workspace
cargo test -p <crate>                                  # 單一 crate
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all                                        # 或 -- --check
cargo xtask check-deps
cargo xtask accept <施工關>                                # 1–13 或名稱，如 core
```

工具鏈釘在 `rust-toolchain.toml`（1.96.0，含 rustfmt、clippy）。

## 完成定義（兩種）

| 範圍 | 條件 |
|---|---|
| 一個改動 | 下方清單：fmt、clippy、測試、check-deps、文件 |
| 一個施工關 | 「一個改動」的全部 + `cargo xtask accept <施工關>` 的 demo + fresh-context verifier 重跑並嘗試推翻 + 使用者完成該施工關「你親自驗收」並填驗收紀錄 + 使用者確認（見 [ROADMAP](docs/ROADMAP.md#每個施工關的完成定義)） |

### 帶使用者親自驗收

使用者（有 ADHD）做「你親自驗收」時，由 agent 帶著走，**不要叫使用者自己去讀施工關頁找指令**：

1. 一次只給一步：先給 `cd` 位置，再給可直接複製的指令（長行拆短，例如 JSON 先存變數）。
2. 一兩句說「這步在驗什麼、壞了會怎樣」，再用短表列出要找的關鍵字。
3. 等使用者貼輸出 → agent 逐項比對、標 ✅／❌ → 才給下一步。
4. 全部做完，由 agent 開 PR 寫驗收紀錄（使用者自行比對的步驟要註明）。

### 一個改動

- [ ] `cargo fmt --all -- --check` 通過
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 通過
- [ ] `cargo test --workspace` 通過（至少 `cargo test -p <改到的 crate>`）
- [ ] `cargo xtask check-deps` 通過
- [ ] 行為變了 → 對應的 crate `README.md`／`TESTING.md` 與 `docs/` 已更新
- [ ] 新名詞已寫進名詞表（[docs/GLOSSARY.md](docs/GLOSSARY.md)）
- [ ] 在自己的 branch + worktree 上完成，以 PR 合併進 `v2`（沒有直接改 `v2`／`main`）


## 寫作與程式規則

| 項目 | 規則 |
|---|---|
| 名詞 | 新增或改名的名詞，先更新 [docs/GLOSSARY.md](docs/GLOSSARY.md)；程式識別字與名詞表一致 |
| 文件 | 繁體中文為主；程式識別字、指令、路徑、技術名詞保留原文 |
| 程式輸出 | 英文：CLI 輸出、錯誤訊息、log、`--help`、測試名稱與 assert 訊息 |
| 程式註解與 `//!` | 英文 |
| 文件格式 | 開頭 3 行 TL;DR；先結論；表格與清單；深入背景放「細節」；需要動作的文件以「下一步」結尾；超過約 150 行就拆（`docs/research/` 的原始紀錄除外，原樣保存） |
| 測試 | 測 consumer 時用真的 producer 產生輸入，不手寫格式（v1 #1493） |
| 骨架 | 不寫假實作或佔位邏輯；還沒決定的東西只寫 `//!` 說明 |

## 決策在哪

- 已確認的決策：[docs/DECISIONS.md](docs/DECISIONS.md)（D1–D37）。
- 沒有新證據不要重開討論。要推翻：先補證據，再提新的決策編號，由使用者確認。
- 文件間衝突時：後來的決策優先於規劃本文。

## 目前狀態

只看一個地方：[docs/ROADMAP.md](docs/ROADMAP.md) 的狀態欄與最下面的「進度紀錄」。這裡不另外抄一份。

規則：

- 每次完成一件事，就在 ROADMAP「進度紀錄」加一行（日期 + 一行 + commit／PR）。
- 施工關狀態改變時，同步更新該施工關頁面（`docs/gates/gate-NN-*.md`）的「狀態」與「進度紀錄」，以及 ROADMAP 的狀態欄。

## 下一步

```bash
cat docs/ROADMAP.md
cat crates/agend-core/README.md crates/agend-core/TESTING.md
cargo xtask accept core
```
