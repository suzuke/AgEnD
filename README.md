# AgEnD v2

> **TL;DR**
> - 這是 AgEnD（Agent Engineering Daemon）v2：異質 agent 團隊的自主 merge 流水線。
> - 狀態：**pre-alpha**。第 1–11 施工關已完成並合併；第 11 施工關 C 段 #145 於 2026-10-03 經使用者確認合併（`b2152db`）。其他 backend adapter 與安裝發布仍待完成。
> - 下一步：先讀 [AGENTS.md](AGENTS.md)，再看 [docs/ROADMAP.md](docs/ROADMAP.md) 的目前狀態。

## 這是什麼

目標是讓一個常駐的 daemon 協調 claude、codex、opencode，讓 agent 團隊自己完成「派工 → 開發 → checks → 互審 → merge」。

人只處理例外：請示、門檻卡住、agent 卡住。

不拼終端多工器、不拼支援的 agent 數量、不拼手機 App。

以下是設計目標；已交付的功能見「目前可用範圍」。

| 差異化 | 怎麼做 |
|---|---|
| 機制保證的 merge 門檻 | 核准綁 head SHA + checks + git shim；需要人工時在 workflow 加 `approval(by = "human")` |
| 跨廠牌 | claude / codex / opencode 互審；額度用盡時改派給別家 |
| 派工時就避免衝突 | daemon 知道每個 task 動到的檔案；merge 前 rebase、依序合併 |
| git 防護內建 | shim 只進 agent 的 PATH，擋 agent 自建 branch/worktree 與改 main |

第一版範圍：macOS + Linux；backend 為 claude / codex / opencode。

## 目前可用範圍

- **Runtime 與持久化**：git／kill／gh shim、PTY holder、SQLite store、daemon 與 client；daemon 重啟後 holder 與 agent 持續執行。
- **Codex 與 CLI**：app-server driver、訊息送達與冪等、三級忙碌策略；`status`、`send`、`inbox`、instance 管理、`daemon restart`、`doctor`、`init`。兩個假 Codex agent 中途重啟仍不漏不重的里程碑已驗收，真 Codex 另有 smoke 驗收。
- **TUI**：agend app 已接 fleet／需要你、完整終端、resize、多視窗控制、鍵鼠／貼上與 1,000 行歷史。[PR #145](https://github.com/suzuke/AgEnD/pull/145) 已合併；固定 `cfee027` 的全新 verifier CONFIRMED，四個雙平台 CI jobs 各 900 passed／0 failed／2 既有 ignored，實際 no-std 通過。使用者已有實機紀錄，並授權剩餘行為以真 producer／native／外層 PTY 自動驗收；清理已完成。Codex 僅開放已驗並獲同意的 0.159.3，其他／未知版本仍唯讀，人工輸入 thread 永久只用自己的 clientId 對帳。[驗收收尾](docs/gates/gate-11c-closeout.md) · [版本政策](docs/gates/gate-11c-codex-input.md)。

**第 10 施工關完成（[PR #143](https://github.com/suzuke/AgEnD/pull/143)，2026-10-02）**：已接通本機 pipeline、task／review／workflow／team 操作與 checks 沙箱。事件收尾修正經全新 verifier r17、Ubuntu／macOS CI 與人工補驗通過，使用者已確認合併。驗證範圍、原始失敗與兩個未執行的 explorer 見 [驗證證據](docs/gates/gate-10-verification.md)。執行方式見 [pipeline runtime](docs/architecture/pipeline-runtime.md)。Claude、OpenCode driver、GitHub forge、Telegram 在第 12 施工關，服務註冊與發布在第 13 施工關。完整狀態與驗收證據見 [ROADMAP](docs/ROADMAP.md)。

第 12A Claude 的 P1–P10 設計已確認，記為 [D40](docs/decisions/d40.md)：閒置走 channel、忙碌排隊走 Stop hook，兩者均用明確 `agend_ack`；P3／P4／P5 選 A。設計文件已於 [PR #138](https://github.com/suzuke/AgEnD/pull/138) 合併（`4390633`）；第 12A 的不自動重送 client 基礎已於 [#147](https://github.com/suzuke/AgEnD/pull/147) 合併（`8dfccf8`）；[Claude 持久化基礎 #148](docs/gates/gate-12a-store.md) 已經使用者確認合併（`7877dbe`）；[protocol 1.5／channel／Stop／ACK spool 基礎 #149](docs/gates/gate-12a-bridge.md) 已合併（`6dd552e`），使用者重驗 16 native cases 通過並清理。[共用 gh 防護 #150](docs/gates/gate-12a-gh-shim.md) 已於 2026-10-04 經使用者確認合併（`572dd73`）。[Claude Driver、啟動設定與 Interrupt](docs/gates/gate-12a-driver.md) 正在獨立 worktree 實作（draft #151）。固定 Claude 2.1.284 的 100／140 欄受控 startup 蒐證各完成三次輸入並保存主介面，經全新 verifier 有限範圍 CONFIRMED，自有暫存已清理；正式 P5／P6、真收件／工作回報與完整 12A 驗收仍未完成。

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
| `crates/agend-shim` | git／kill／gh 防護；只讀 binding 快照 |
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
