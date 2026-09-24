# TUI、設定與安裝

> **TL;DR**
> - TUI 是 attention-first：先看「需要你」，再看各 team。
> - 記住：**唯一真相來源是 DB**；設定檔只剩 daemon 層級的 `config.toml`。
> - 下一步：TUI 模組在 `crates/agend-tui/`；doctor／init 在 `crates/agend/src/`。

來源：規劃 r4 §3.1、§4.7–§4.9、architecture 頁的 TUI 節、D8、D13、D17。

## TUI

- 層級：Fleet → Team → Task 或 Agent。畫面一律用 team 分組；repo 只出現在 Task Detail。
- 首頁：跨 team 的「需要你」在最上面，下面每個 team 一個區塊。
- team 頁：目標、Agents、流水線三個 tab。
- `←`／`→` 在每個畫面都代表上一層／下一層。
- 「已讀」與「已解決」分開：看過只去掉粗體，選了動作才解除。
- 支援英文與繁中，執行中按 `L` 切換。
- 範圍：儀表板 + agent 清單 + 單一 agent attach；分割視窗之後再做。
- 原型：DEMO-01（`github.com/suzuke/agend-attention-tui-demo`，private），五輪完成並驗證。

## agent 介面（CLI，D7、D17）

- 身分與上下文從「呼叫者身分 → DB 裡的 binding」推得，不看 cwd（v1 agent 的 cwd 是 `workspace/<instance>`）。
- agent 只傳意圖：收件者、內容、請求類型或 review 結論、完成條件、附件、回覆對象、期望回覆時間。
- daemon 推得：task_id、branch、repository、PR 編號、reviewed_head／expected_head、correlation_id。
- agent 命令（11 個，D17）：`status`、`done`、`result`、`review approve`、`review changes`、`send`、`inbox`、`ask`、`block`／`unblock`（一個命令、兩個動詞）、`task create`、`remind`。
- 命令總數 < 15、常用命令參數 ≤ 2；`agend status` 顯示所在步驟與可用下一步；錯誤附正確命令；`--help` 範例優先；支援 `--json`。
- daemon 重啟中：CLI 重試最多 10 秒後印出明確訊息。
- 啟動路徑：CLI 與 shim 不建 runtime、不讀設定、不開 DB；argv[0] 分派在 main 最前面。實測啟動 p50 4.1 ms、unix socket 來回 0.014 ms。
- 需要時由同一份定義產生 MCP 轉接層。

## 設定與目錄（D8）

| 項目 | 規則 |
|---|---|
| instance、team、repo、workflow | 存 DB；由 CLI／TUI 管理 |
| instance `lifetime` | `persistent`（daemon 啟動時拉起）或 `ephemeral`（隨 task／team 結束清理） |
| `config.toml` | 人寫、daemon 只讀：home 路徑、Telegram 等連線設定 |
| secret | 以環境變數或檔案路徑引用，不寫進 DB |
| 備份 | `agend export`／`agend import`；每天 `VACUUM INTO` 快照，保留 N 份 |
| workspace | 每個 instance 一個常駐工作目錄；另給 agent 一個會被清掉的暫存目錄 |
| 磁碟 | daemon 監看 home 大小並警告（v1：`evidence/` 108G 是 agent 寫的、`workspace/` 23G） |

Telegram（D13）：一個「需要你」topic + 每個 team 一個 topic；個別 instance topic 可選、非預設。

## 安裝與設定

原則：每個設定錯誤都在第一次使用前被明確指出，並附修正指令。

施工：`doctor`、`init` 在第 9 施工關；服務註冊、`uninstall`、`telegram setup`、打包與發布在第 13 施工關。規則（版本範圍、登入判斷、git 最低版本、unit 文字）在 `agend_core::setup`，執行在 `agend` crate 的 `setup` 模組。

1. `agend doctor`：git 版本（merge-tree 需 ≥ 2.38）、gh 登入（僅 github forge）、各 backend 安裝／版本／登入、服務狀態、磁碟、Telegram（allowlist 為空即報錯）。支援 `--json`。
2. `agend init`：建 home 與 `config.toml`、註冊 launchd／systemd、偵測 backend、建 `general` team 與一個 agent；在 repo 內才詢問是否登記；最後跑 doctor。只在互動終端發問。
3. TUI 空畫面即引導：第一步「開始第一個 agent」，repo 是可選的下一步；不做獨立精靈。
4. `agend telegram setup`：貼 token → 對 bot 傳 `/start` → 自動取得 chat id 並加入 allowlist。CLI 只請 daemon 配對，配對由 daemon 的 notifier 做。
5. 自動處理 trust 設定、shim 安裝、agent PATH；shim 只進 agent 的 PATH，不改使用者自己的 git。
6. `agend uninstall`：移除服務與 shim；資料是否刪除另外詢問。
7. 驗收：安裝到第一個 agent 完成 task < 5 分鐘；CI 以全新 HOME + 假 agent 跑 e2e。
8. 安裝管道：brew、GitHub release 預編譯 binary、`cargo install`。

## 細節：v1 的設定錯誤案例

#2207（空 allowlist 讓 Telegram 雙向訊息靜默丟棄）、#2005（寫入與讀取的 token 變數名不一致）、#3402（codex 專案設定未被信任）、#3499（關終端 daemon 跟著死）、#1351／#2204（quickstart 流程反覆改）。共同點：設定錯了不報錯，只是靜靜不動。

## 不包含（v2.0）

Discord、tray、grok／kiro／agy／shell backend、Windows、deployments、quickstart 精靈、skills 同步、token 成本統計、`repo merge`、v1 治理機制（receipt、claim_verifier、HMAC、operator_mode 簽章、assignment_authority 等約 18 個）。之後要加必須附使用證據。

## 下一步

```bash
cat docs/ROADMAP.md
```
