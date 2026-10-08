# xtask

> **TL;DR**
> - 開發者工具：`cargo xtask check-deps`、`cargo xtask accept <施工關>`、`cargo xtask record <backend>`。
> - 記住：**crate 邊界規則由 `check-deps` 強制**；規則是 `xtask/src/check_deps.rs` 裡的資料。
> - 下一步：改依賴後跑 `cargo xtask check-deps`。

## 負責

- `check-deps`：
  1. shim／client／tui 在 `cargo tree -e normal,build --target all` 裡沒有被禁止的 crate（dev 依賴不檢查）：shim 與 client 不可有 async runtime、SQLite、agend-daemon；tui 不可有 SQLite、agend-daemon（crossterm 會帶 `mio`，所以不擋 runtime）；daemon 不可有 agend-holder、agend-shim（第 6 施工關）、agend-client（第 8 施工關 P10）
  2. agend-testkit 不是任何 crate 的一般依賴
  3. agend-core：`cargo metadata` 顯示沒有 build script、沒有 `[features]`、唯一直接依賴是停用 default features 且只開 `derive` + `alloc` 的 serde（D32）；而且能以 `--all-features`、`-F unsafe-code` 對無 std 的 `thumbv7em-none-eabihf` 編譯（見 `check_core.rs`）
  4. target 沒裝時印 `SKIPPED` 並失敗；`--allow-skip` 才不失敗（仍印 SKIPPED）

`SKIPPED` 代表**沒有驗證**，不是通過。

- 本機要驗證：`rustup target add thumbv7em-none-eabihf`，再跑 `~/.cargo/bin/cargo xtask check-deps`（不加 `--allow-skip`）。Homebrew 的 `cargo` 沒有額外 target，一定會 SKIPPED。
- `--allow-skip` 只在你明白這一項沒驗證時用；它仍印出 SKIPPED。
- CI 一定會跑這一項（不加 `--allow-skip`）。

- `accept core`：跑 workspace fmt、workspace clippy、core tests、protocol compatibility tests、check-deps，再執行 core example 的 protocol 與 code workflow demo。
- `accept testkit`：對 agend-testkit 跑 fmt、clippy、test（含 7 個契約 suite 與假 agent 程式測試），再跑 check-deps，然後 build 假 agent binary、執行 `testkit_demo` example（三個假 agent 各一段往來並正常結束、假 daemon 的事件身分、契約摘要）。
- `accept shim`：對 agend-shim 跑 fmt、clippy、test，加上 `agend` 的 argv[0] 分派測試與 check-deps，再 build `agend` 並執行 `agend-shim` 的 `shim_demo` example（以 `git`／`kill`／`pkill` 名稱在暫存 repo 裡跑真的 binary）
- `accept holder`：對 agend-holder 與 agend 跑 fmt、clippy、test（含跨程序的 `holder_process`），再跑 check-deps，然後 build `agend`、執行 `holder_probe demo`（`== detach` 到 `== shutdown` 各段）。
- `accept codex`：對 agend-daemon、agend-testkit、agend 跑 fmt、clippy、test（含 DRV-1..9 對 codex driver、四次開機、真 daemon 與 `sh` 包裝），再跑 check-deps，然後 build `agend` 與 example `fake_codex`、執行 `agend-daemon` 的 `codex_demo` example（`== busy`／`== idempotent`／`== crash-window`／`== approval`／`== restart`（含反向檢查）／`== resume`／`== sweep`／`== give-up`／`== app-server-dies`／`== first-start-interrupted`／`== legacy`／`== cleanup`），最後一行 `gate 7 (codex): checks passed`。不跑真 codex（`codex_live` 只有使用者手動跑）。
- `accept client`：對 agend-client、agend-daemon、agend-testkit、agend-core、agend 跑 fmt、clippy、test（含 CLP 契約對假 daemon 與真 `agend daemon`、mutant），再跑 check-deps，然後 build `agend`、執行 `agend-daemon` 的 `client_demo` example（`== contract` 每條 `CLP-n` 印 `fake`／`real` 兩行與反向檢查，之後 `== version`／`== slow-client`／`== socket`／`== retry`／`== terminal`／`== restart`／`== cleanup`），最後一行 `gate 8 (client): checks passed`。
- `accept tui`：對 agend-tui、agend-client、agend-daemon、agend-testkit、agend 跑 fmt、clippy、test（含 CLP-18..20 對假、真 daemon 與 mutant、TUI 對真 daemon），再跑 check-deps，然後執行 `tui_accept` example（假 daemon 經 `agend-client`：`== screens`／`== navigate`／`== resolve`／`== disconnect`／`== retry`／`== terminal`／`== input`，每段有檢查），再 build `agend`、執行 `agend` 的 `tui_real` example（真 `agend daemon`：`== real daemon`／`== retry`／`== terminal`／`== input`／`== reconnect`／`== agend app`），最後一行 `gate 11 (tui): checks passed`。
- `demo adapters`：先建置正式 agend 與假 producer，執行 Claude driver／bridge／holder、OpenCode driver／bridge、Telegram transport／inbound／topic、daemon 關機與 Retry、G4 共用已讀、未知通知處置及 doctor 原生案例。沒有真模型或外部 API 呼叫；真測紀錄另驗。`accept adapters` 對 core／daemon／client／tui／testkit／agend 跑 checks 及 no-std，再執行 demo；GitHub forge 尚未合入，本入口不宣稱完整第 12 關通過。
- 其他 `accept <施工關>`：對該施工關的 crate 跑 fmt、clippy、test，再跑 check-deps；demo 隨各施工關加入
- `record <backend> [情境…] --sandbox <腳本>`：build `agend-record`（agend-testkit），在 `<腳本>`（寫入沙箱）裡對**真的** CLI 錄製到 `mktemp -d /private/tmp/agend-rec-out-XXXX`，再在沙箱外把成功的錄製檔複製進 `crates/agend-testkit/transcripts/<backend>/`（見 [RECORDER.md](../crates/agend-testkit/RECORDER.md)）。沒有 `--sandbox` 就不跑

第 11 施工關 accept tui 先建置真 agend、fake_codex 與 codex_u17_probe，再跑原 checks／fake 與真 daemon TUI demos，最後跑共用 integration test 情境的完整 U17 fake demo。真 codex_u17_live 不在 acceptance 或 CI 執行，必須明確 opt-in。

## 不負責

- 擋刻意繞過（例如改 xtask、加長 allowlist）：靠 code review
- 產生 protocol JSON schema（規劃中，未實作）
- 公開發布 release（打包與發布分開，打包命令不建立 tag／上傳）
- 錄製器本身（在 agend-testkit；xtask 不依賴 testkit，只執行它的 binary）

## 模組

| 模組 | 職責 |
|---|---|
| `check_deps` | 規則與檢查 |
| `check_core` | agend-core 的結構檢查：`cargo metadata` 規則與無 std 編譯 |
| `accept` | 13 個施工關的 crate 對照與執行 |
| `core_demo`、`shim_demo` | 第 1、3 施工關的 demo（子程序執行 example） |
| `adapters_demo` | 共用 Claude／OpenCode／Telegram 原生案例；不跑真模型／外部 API |
| `record` | 在寫入沙箱裡跑 `agend-record`，複製錄製檔 |

## 依賴規則

- 一般依賴：`serde_json`（metadata）；`agend-core` 與 `toml`（workflow golden 測試）只作為 xtask 測試的 dev-dependency，core acceptance demo 由子程序執行獨立 example，避免 checker 連結待檢查的 core；透過 `$CARGO` 執行 cargo，無 std 編譯時用同一個 toolchain 的 rustc
- workspace 根目錄在執行時用 `cargo locate-project --workspace` 從目前目錄找，所以在 repo 副本裡跑會檢查副本本身
- 不屬於 release binary

## 入口

- `cargo xtask check-deps`、`cargo xtask accept <1-13 或名稱>`（alias 在 `.cargo/config.toml`）

## 細節

### 施工關對照

`cargo xtask accept <編號或名稱>`：

| 編號 | 名稱 | 施工關頁 |
|---|---|---|
| 1 | `core` | [docs/gates/gate-01-core.md](../docs/gates/gate-01-core.md) |
| 2 | `testkit` | [docs/gates/gate-02-testkit.md](../docs/gates/gate-02-testkit.md) |
| 3 | `shim` | [docs/gates/gate-03-shim.md](../docs/gates/gate-03-shim.md) |
| 4 | `holder` | [docs/gates/gate-04-holder.md](../docs/gates/gate-04-holder.md) |
| 5 | `store` | [docs/gates/gate-05-store.md](../docs/gates/gate-05-store.md) |
| 6 | `daemon-holder` | [docs/gates/gate-06-daemon-holder.md](../docs/gates/gate-06-daemon-holder.md) |
| 7 | `codex` | [docs/gates/gate-07-codex.md](../docs/gates/gate-07-codex.md) |
| 8 | `client` | [docs/gates/gate-08-client.md](../docs/gates/gate-08-client.md) |
| 9 | `cli` | [docs/gates/gate-09-cli.md](../docs/gates/gate-09-cli.md) |
| 10 | `pipeline` | [docs/gates/gate-10-pipeline.md](../docs/gates/gate-10-pipeline.md) |
| 11 | `tui` | [docs/gates/gate-11-tui.md](../docs/gates/gate-11-tui.md) |
| 12 | `adapters` | [docs/gates/gate-12-adapters.md](../docs/gates/gate-12-adapters.md) |
| 13 | `install` | [docs/gates/gate-13-install.md](../docs/gates/gate-13-install.md) |

### 禁止清單

| 群組 | crate（`*` = 前綴） |
|---|---|
| async runtime | `tokio`、`tokio-*`、`async-std`、`smol`、`mio` |
| database | `rusqlite`、`libsqlite3-sys`、`sqlx`、`sqlx-*` |
| network | `hyper`、`hyper-*`、`reqwest`、`ureq`、`h2`、`socket2`、`tungstenite`、`tokio-tungstenite`、`teloxide` |
| process | `portable-pty`、`nix`、`signal-hook`、`signal-hook-*` |

| crate | 禁止 |
|---|---|
| `agend-core` | serde 以外的依賴、default features、derive／alloc 以外的 serde features，以及任何 `[features]`，由 `check_core.rs` 以 `cargo metadata` 檢查 |
| `agend-shim` | async runtime、database、`agend-daemon` |
| `agend-client` | async runtime、database、`agend-daemon` |
| `agend-daemon` | `agend-holder`、`agend-shim`（第 6 施工關 P9）、`agend-client`（第 8 施工關 P10：server 與 client 各自編碼） |
| 所有 crate | `agend-testkit` 當一般依賴 |

shim／client 檢查 normal 與 build 依賴，不檢查 dev 依賴。`network`、`process` 群組目前沒有規則使用，保留當文件。`libc` 不在清單內。

## 下一步

```bash
~/.cargo/bin/cargo xtask check-deps   # rustup 的 cargo；Homebrew cargo 會 SKIPPED
cargo xtask accept core
```

第 12C 整合後 `demo adapters`／`accept 12` 包含正式 GitHub Forge／strict base policy／原生 daemon 重啟與清理案例。這些全為離線原生測試；真 GitHub 驗收另記 gate-12c，不在 CI 呼叫外部 API。

## Native release 打包（13E 施工中）

`cargo xtask release --out /absolute/new/directory` 要求乾淨的已提交 checkout，以 `--locked --release` 建置本機 target 的 agend，執行 `--version` 核對 Cargo metadata，再產生 tar.gz、SHA256SUMS 與 manifest.json（版本、target、source commit、binary／archive SHA-256）。支援 macOS／Linux 的 x86_64／aarch64 native build，沒有交叉執行或假稱跨平台驗證。輸出目錄必須不存在且位於 checkout 外。

壓縮檔含 executable、README 與 LICENSE；暫存 payload 完成後移除。失敗可能保留尚未完成的輸出目錄，必須檢查後清理；不自動重用。此命令只打包，不建立 tag、不改服務、不上傳 GitHub。Brew／發布 workflow 與全新 HOME 五分鐘驗收仍待完成。

`.github/workflows/release.yml` 可手動建置 macOS／Linux × Intel／ARM64 原生產物，只有 contents:read 與 14 天 Actions artifacts，沒有公開發布。`scripts/verify_release.py` 核來源提交／target、archive 與 binary 雜湊、精確 tar 清單，再於暫存 HOME 執行解壓 binary 的 --version。runner 版本是建置環境，不表示已驗所有較舊 OS；四平台實跑與 Brew 接線仍待驗證。
