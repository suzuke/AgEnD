# 施工路線圖：13 關

> **TL;DR**
> - 依 crate 由下往上分 13 關；每關單獨驗收，使用者確認後才開下一關（D22）。
> - 目前狀態：**第 0 階段（spike）完成；第 1 關尚未開始**。骨架與文件已就位。
> - 下一步：開第 1 關（agend-core），驗收指令 `cargo xtask accept core`。

## 13 關

| 關 | 範圍 | 驗收（可觀察） |
|---|---|---|
| 1 `core` | agend-core：型別、兩套協定（client + holder）、trait、流水線狀態機（6 種關卡）、busy policy、去抖動、衝突偵測、merge 門檻（patch-id）、螢幕分類器 | `cargo xtask accept core` 跑測試並印出一個模擬 task 走完 `code` workflow（純邏輯，無 daemon） |
| 2 `testkit` | 每個 trait 的假實作、契約測試套件、假 daemon、假 agent 程式 | 契約測試通過；假 agent 可單獨啟動並回應 |
| 3 `shim` | git／kill 防護、導向 worktree、protected-ref、快照與還原 | 在暫存 repo 以 `git` 名稱執行 shim：導向、拒絕、快照後還原 |
| 4 `holder` | PTY、畫面、附屬程序、holder 協定 | `agend holder` 包 bash + 小型探測 client：讀畫面、送鍵、中途斷線重連，bash 存活 |
| 5 `store` | daemon store：SQLite schema、migration、保留期限、每日快照 | in-memory 測試；xtask 命令印出資料表 |
| 6 `daemon-holder` | 整合關：runtime adapter | 真 daemon + 真 holder；重啟 daemon，agent 與畫面存活 |
| 7 `codex` | codex driver + 送達模型、三級忙碌策略 | 對假 app-server；可選的真 codex smoke test |
| 8 `client` | 整合關：agend-client + protocol server | CLI 連得上；daemon 重啟時會重試 |
| 9 `cli` | agend CLI：agent 命令、操作者命令、status；安裝相關只做 `doctor`、`init`（讓前面各關能在本機跑；`init` 的服務註冊步驟在第 13 關補上） | 對假 daemon 驗每個命令的輸出與錯誤；再對真 daemon |
| 10 `pipeline` | daemon：pipeline、git、runner、forge local、supervisor、reconcile | 假 driver + 暫存 repo：task 從派工走到 merge |
| 11 `tui` | attention-first TUI（沿用 DEMO-01 原型的教訓：`github.com/suzuke/agend-attention-tui-demo`，private） | 先餵假事件，再接真 daemon |
| 12 `adapters` | claude + opencode driver、forge github、telegram | 先對假實作，再做真 backend smoke test |
| 13 `install` | 安裝與發布（最後一關，需要其他全部）：launchd／systemd 服務註冊、`agend uninstall`（移除服務與 shim；刪資料前先問）、`agend telegram setup`（CLI 請 daemon 配對：貼 token → 使用者對 bot 傳 `/start` → chat id 加入 allowlist；配對由 daemon 的 notifier 做，CLI 沒有 Telegram client）、`xtask release` 打包、brew formula、GitHub release workflow、`cargo install` | CI 用全新 HOME + 假 agent，從安裝到第一個 task 完成 < 5 分鐘；每個 `doctor` 檢查都有「故意弄壞 → 看到修正指令」的測試 |

## 安裝相關的程式放在哪

| 內容 | 位置 |
|---|---|
| 規則：已測的 backend 版本範圍、怎麼判斷已登入、git 最低版本、產生的 launchd／systemd unit 文字 | `agend_core::setup`（資料 + 純函式，無 I/O，符合 no_std） |
| 執行：跑指令、寫檔、註冊服務 | `agend` crate 的 `setup` 模組 |

## 里程碑（使用者可見）

| 完成到 | 使用者看到 |
|---|---|
| 第 1–9 關 | 兩個 codex agent 互傳訊息；重啟 daemon 時不中斷、不遺失、不重複 |
| 第 10–11 關 | 本機 repo 從派工走到 merge，TUI 可看可操作 |
| 第 12 關 | 三個 backend + GitHub + 手機（Telegram） |
| 第 13 關 | 其他人可以自己安裝 |

之後：與 v1 並行一週（另一個 Telegram bot、另一份 repo clone、另一個 home），一週內日常工作不需回 v1；learnability 複測（規劃 §6 第 5 階段）。

## 每關的完成定義

- [ ] `cargo test -p <crate>` 單獨通過
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `cargo xtask check-deps` 通過
- [ ] `cargo xtask accept <關>` 存在、會跑測試並印出人看得懂的 demo
- [ ] 該 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻
- [ ] 使用者確認後才開下一關

需要改 agend-core 時：先改 core，重過第 1 關的測試。crate 之間不得有私下耦合。

`cargo xtask accept <關>` 目前是骨架：對該關的 crate 跑 fmt、clippy、test 與 check-deps，並明說 demo 尚未實作。demo 隨各關加入。

## 第 0 階段（spike）結果

全部完成，結論在 [BACKEND-BEHAVIORS.md](BACKEND-BEHAVIORS.md)。

| # | 問題 | 結論 |
|---|---|---|
| 1 | 附屬程序存活時 daemon 能否重連並補回事件 | codex、opencode 可以（以獨立程序測，非 holder 內） |
| 2 | codex 是否通知 TUI 手動發起的 turn | 可以，但要先 `thread/resume` |
| 3 | claude `Esc` 後 channel 訊息是否立即處理 | 有 CLAUDE.md 來源說明 3/3；程式化 send-now 只在 headless 驗證 |
| 4 | opencode 插入與中斷 | 無插入；abort 可用 |
| 5 | 以明確 id resume | 三個都可以 |
| 6 | codex sandbox 內 CLI 能否連 unix socket | 預設被擋；需 approval 或把 socket 放進 workspace |
| 7 | claude 以 allowlist 免除 `agend` 權限提示 | allow 規則有效；反例與 `agend` 本身未直接驗證 |
| 8 | 啟動提示能否全部避免；授權是否有結構化管道 | codex 可預寫 trust；opencode 無提示；claude 預寫設定 BLOCKED。授權：codex、opencode 可用，claude 未驗證 |

## 下一步

```bash
cargo xtask accept core    # 目前只跑檢查；demo 屬於第 1 關的工作
cat crates/agend-core/README.md
```
