# 施工路線圖：13 個施工關

> **TL;DR**
> - 依 crate 由下往上分 13 個施工關；每個施工關單獨驗收，使用者確認後才開下一個施工關（D22）。
> - 目前狀態：**第 1 施工關實作中**（P1–P7 已確認）；每個施工關狀態看下表，做了什麼看最下面的「進度紀錄」。
> - 下一步：fresh-context verifier 與使用者親自驗收第 1 施工關。

## 13 個施工關

每個施工關的細節、你親自驗收的步驟與紀錄在 [docs/gates/](gates/README.md)。

| 施工關 | 狀態 | 範圍 | 驗收（可觀察） |
|---|---|---|---|
| 1 `core` | [實作中](gates/gate-01-core.md) | agend-core：型別、兩套協定（client + holder）、trait、流水線狀態機（6 種關卡）、busy policy、去抖動、衝突偵測、merge 門檻（patch-id）、螢幕分類器 | `cargo xtask accept core` 跑測試並印出一個模擬 task 走完 `code` workflow（純邏輯，無 daemon） |
| 2 `testkit` | [未開始](gates/gate-02-testkit.md) | 每個 trait 的假實作、契約測試套件、假 daemon、假 agent 程式 | 契約測試通過；假 agent 可單獨啟動並回應 |
| 3 `shim` | [未開始](gates/gate-03-shim.md) | git／kill 防護、導向 worktree、protected-ref、快照與還原 | 在暫存 repo 以 `git` 名稱執行 shim：導向、拒絕、快照後還原 |
| 4 `holder` | [未開始](gates/gate-04-holder.md) | PTY、畫面、附屬程序、holder 協定 | `agend holder` 包 bash + 小型探測 client：讀畫面、送鍵、中途斷線重連，bash 存活 |
| 5 `store` | [未開始](gates/gate-05-store.md) | daemon store：SQLite schema、migration、保留期限、每日快照 | in-memory 測試；xtask 命令印出資料表 |
| 6 `daemon-holder` | [未開始](gates/gate-06-daemon-holder.md) | 整合施工關：agent runtime adapter | 真 daemon + 真 holder；重啟 daemon，agent 與畫面存活 |
| 7 `codex` | [未開始](gates/gate-07-codex.md) | codex driver + 送達模型、三級忙碌策略 | 對假 app-server；可選的真 codex smoke test |
| 8 `client` | [未開始](gates/gate-08-client.md) | 整合施工關：agend-client + protocol server | CLI 連得上；daemon 重啟時會重試 |
| 9 `cli` | [未開始](gates/gate-09-cli.md) | agend CLI：agent 命令、操作者命令、status；安裝相關只做 `doctor`、`init`（讓前面各施工關能在本機跑；`init` 的服務註冊步驟在第 13 施工關補上） | 對假 daemon 驗每個命令的輸出與錯誤；再對真 daemon |
| 10 `pipeline` | [未開始](gates/gate-10-pipeline.md) | daemon：pipeline、git、runner、forge local、supervisor、reconcile | 假 driver + 暫存 repo：task 從派工走到 merge |
| 11 `tui` | [未開始](gates/gate-11-tui.md) | attention-first TUI（沿用 DEMO-01 原型的教訓：`github.com/suzuke/agend-attention-tui-demo`，private） | 先餵假事件，再接真 daemon |
| 12 `adapters` | [未開始](gates/gate-12-adapters.md) | claude + opencode driver、forge github、telegram | 先對假實作，再做真 backend smoke test |
| 13 `install` | [未開始](gates/gate-13-install.md) | 安裝與發布（最後一個施工關）：服務註冊、`agend uninstall`、`agend telegram setup`（由 daemon 配對）、`xtask release`、brew、GitHub release、`cargo install` | CI 用全新 HOME + 假 agent，從安裝到第一個 task 完成 < 5 分鐘；每個 `doctor` 檢查都有「故意弄壞 → 看到修正指令」的測試 |

## 第 1 施工關：開工前先提案、經使用者確認才實作

7 項提案（P1–P7）列在 [gate-01-core.md](gates/gate-01-core.md#開工前提案)，使用者 2026-09-25 確認，記為決策 D26–D32。實作草稿是在確認前寫的；草稿作者 2026-09-24 自己打的勾不算確認。

## 安裝相關的程式放在哪

| 內容 | 位置 |
|---|---|
| 規則：已測的 backend 版本範圍、怎麼判斷已登入、git 最低版本、產生的 launchd／systemd unit 文字 | `agend_core::setup`（資料 + 純函式，無 I/O，符合 no_std） |
| 執行：跑指令、寫檔、註冊服務 | `agend` crate 的 `setup` 模組 |

## 里程碑（使用者可見）

| 完成到 | 使用者看到 |
|---|---|
| 第 1–9 施工關 | 兩個 codex agent 互傳訊息；重啟 daemon 時不中斷、不遺失、不重複 |
| 第 10–11 施工關 | 本機 repo 從派工走到 merge，TUI 可看可操作 |
| 第 12 施工關 | 三個 backend + GitHub + 手機（Telegram） |
| 第 13 施工關 | 其他人可以自己安裝 |

之後：與 v1 並行一週（另一個 Telegram bot、另一份 repo clone、另一個 home），一週內日常工作不需回 v1；learnability 複測（規劃 §6 第 5 階段）。

## 每個施工關的完成定義

- [ ] `cargo test -p <crate>` 單獨通過
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `cargo xtask check-deps` 通過（不能是 SKIPPED）
- [ ] `cargo xtask accept <施工關>` 存在、會跑測試並印出人看得懂的 demo
- [ ] 該 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻
- [ ] 使用者完成該施工關「你親自驗收」清單並填寫驗收紀錄
- [ ] 使用者確認後才開下一個施工關

需要改 agend-core 時：先改 core，重過第 1 施工關的測試。crate 之間不得有私下耦合。

第 1 施工關的 demo 由 `agend-core` example 呼叫 protocol、policy、assignment 與 pipeline API。verifier 回饋修正後，workspace tests（156 個，含 105 core unit tests、11 個可完成性測試、兩個狀態機探索器、7 protocol compatibility tests、2 workflow TOML golden tests）、clippy、check-deps 與 acceptance 已通過；仍待 fresh-context verifier 重跑與使用者手動驗收。

## 第 0 階段（spike）

已完成；8 個問題的結論在 [BACKEND-BEHAVIORS.md](BACKEND-BEHAVIORS.md#第-0-階段的-8-個問題)。

## 下一步

```bash
cat docs/gates/gate-01-core.md
~/.cargo/bin/cargo xtask accept core
```

## 進度紀錄

每完成一件事加一行（日期 + 一行 + commit／PR），新的在上面。

- 2026-09-25 第 1 施工關 verifier r3 推翻（3e8b3a3）後修正：沒有 merge 的 workflow 也不允許最後的 branch work 之後再有 work；pick fanout 重跑要重新挑（4b05a60）。
- 2026-09-25 第 1 施工關 verifier r2 推翻（843a235）後改成結構性解法：存檔檢查以 `step` 做可完成證明、pick fanout 與 branch work 文法收斂、隨機 workflow 產生器成為常駐測試（ba30886）。
- 2026-09-25 第 1 施工關 verifier r1 推翻（832a4dc）後修正：merge／command／綁 head 的 approval 前面必須有產出 branch 的 work 且需要 repo；merge 送出後的 head 變更等 forge 結果（`MergeFailed`）（f458545、ebdac60）。
- 2026-09-25 rebase 到 v2（#104），第 1 施工關文件改用名詞表的詞（832a4dc）。
- 2026-09-25 使用者決定 D34–D37（`planned` workflow、對話式請示、請示排序、context recap），並核准 D32 擴充到 workflow 定義型別（以 golden TOML 測試鎖格式）；實作（7468ba0、d91865a）。
- 2026-09-25 第 1 施工關 fresh-context verifier 推翻幾個窄點，已修：merge 須為最後關卡、綁 head 的關卡須在最後的 branch work 之後、`on_fail` 只能指向 work、merge 送出後不可取消、`PipelineState` 不可偽造、第二個探索器（13c0dbc、6ae6364）。
- 2026-09-25 使用者決定第 1 施工關 Q1，記為 D33（一個 agent 一個 task、返工回 task 持有者）；`policy::assign` 照此改寫（ba1fe59）。
- 2026-09-25 使用者確認第 1 施工關提案 P1–P7，記為 D26–D32。
- 2026-09-25 第 1 施工關第 2 輪 review 修正：head 變更不跳過關卡、返工不遺失、要求修改退回 task 持有者、merge 門檻逐個關卡檢查、取消獨立狀態、佔位符存檔檢查、reviewer 同 backend fallback；新增狀態機探索器（2a6e29b、4f78b31、e867d52）。
- 2026-09-25 草稿原樣匯入 `feat/gate-01-core`（80c4de9）；草稿作者未經確認的 P1–P7 勾選先更正為未確認（b480311）。
- 2026-09-25 （草稿作者）review 修正 pipeline、assignment、protocol 相容與 check-deps 自我檢查；workspace tests（66 core tests、5 protocol compatibility tests）、clippy、check-deps、accept core 通過；待 fresh-context verifier 與使用者親自驗收（工作樹，尚未提交）。
- 2026-09-25 新增名詞表 docs/GLOSSARY.md；施工階段統一稱「施工關」、workflow 步驟稱「關卡」（#104）
- 2026-09-24 `agend-core` 草稿初次自動驗收通過：workspace fmt/clippy、49 core tests、no-std check 與 demo；草稿作者自行對照 P1–P7（不是使用者確認），P6 由第 5 施工關 Store 落地（工作樹，尚未提交）。
- 2026-09-24 在 P1–P7 確認前開始 `agend-core` 實作草稿（codex/gate-01-core 工作樹，尚未提交）。
- 2026-09-24 AGENTS.md 加入必守的 Git 工作流程：branch + worktree、只經 PR 合併（#103）
- 2026-09-24 第 1 施工關提案中（#102）
- 2026-09-24 README 系統圖改為 SVG（#101, e893877）
- 2026-09-24 CI 首次通過（ubuntu + macOS，8a5b0fd，[run 35979128418](https://github.com/suzuke/AgEnD/actions/runs/35979128418)）
- 2026-09-24 骨架與文件 push 到 v2（8a5b0fd）
- 2026-09-24 spike 完成（codex／claude／opencode + claude 追加；紀錄在 [research/](research/README.md)：[spike-codex](research/spike-codex.md)、[spike-claude](research/spike-claude.md)、[spike-claude-f](research/spike-claude-f.md)、[spike-opencode](research/spike-opencode.md)、[runtime-spike](research/runtime-spike.md)）
