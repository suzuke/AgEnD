# AgEnD v2

> **先看這三行**
> - 讓 Claude、Codex、OpenCode 組隊完成任務：派工 → 開發 → 檢查 → 審查 → 合併。
> - **目前可試用，尚未正式發布。** 第 1–12 施工關已完成；第 13 關實作已合併，Claude 版本管理真測與部分驗收仍待完成。
> - 第一次來？從下方「先看介面」開始，不需要模型帳號。

[先看介面](#先看介面) · [連上真正的-agent](#連上真正的-agent) · [操作速查](#操作速查) · [目前進度](#目前進度) · [開發文件](#開發文件)

## 它幫你做什麼

AgEnD 是常駐在本機的 Agent Engineering Daemon。你指定任務、團隊與工作流程；它分配 agent、建立工作目錄、執行檢查，依流程取得核准後合併。需要你決定的權限、問題與審查，集中在「需要你」。

終端介面可以查看各 agent 的原生畫面並取得輸入控制。也能用 Telegram 處理通知與核准。支援 macOS、Linux。

## 先看介面

這條路只用示範資料，**不啟動模型、不需要登入**。需要 Git 與 rustup；專案會依 `rust-toolchain.toml` 使用 Rust 1.96.0，首次編譯需要下載依賴。

### 1. 取得 v2 原始碼

```sh
git clone --branch v2 https://github.com/suzuke/AgEnD.git agend-v2
cd agend-v2
```

**確認：** 目前目錄是 `agend-v2`，裡面有 `Cargo.toml`。

### 2. 開啟示範介面

```sh
cargo run --locked -p agend-tui --example tui_fake -- --lang zh-TW
```

**確認：** 出現團隊、agent 與示範待辦。視窗至少 110 欄、20 列時會顯示側欄；較小時使用單欄。

### 3. 試三個操作

- 按 `!` 看「需要你」，用 `[`、`]` 查看上一項／下一項。
- 按大寫 `P` 預覽配色；方向鍵選擇、Enter 儲存、Esc 取消。
- 按 `q` 離開示範。

示範中儲存的配色會套用到正式介面；模型的原生終端保留自己的配色。

## 連上真正的 agent

先完成上面的原始碼下載。以下以 **OpenCode** 為例，需要先安裝 OpenCode 並確認它能在你自己的終端正常啟動、登入及回覆。實際傳送訊息可能產生模型費用。

### 1. 安裝 AgEnD

在 `agend-v2` 目錄執行：

```sh
cargo install --locked --path crates/agend --root "$HOME/.local"
export PATH="$HOME/.local/bin:$PATH"
agend --version
```

**確認：** 印出 AgEnD 版本。目前驗收版本號仍為 `0.0.0`。若已有其他來源的安裝而 Cargo 拒絕覆寫，先核對既有安裝；[安裝文件](docs/install-release.md) 有更新方式。

### 2. 建立試用資料目錄，啟動 daemon

在同一個終端（**A**）執行：

```sh
export AGEND_HOME="$HOME/agend-trial"
agend init
```

**先確認：** daemon 尚未啟動的警告是正常的；其他 `fail` 請先依 `fix:` 處理。資料保存在 `$HOME/agend-trial`；已有資料會沿用，請勿當作空目錄刪除。

沒有 `fail` 後，再執行：

```sh
agend daemon
```

**確認：** 出現 `agend daemon ready`。保持終端 A 開啟。

### 3. 新增 OpenCode，開啟介面

另開終端（**B**），先設定相同的路徑：

```sh
export PATH="$HOME/.local/bin:$PATH"
export AGEND_HOME="$HOME/agend-trial"
agend instance add try-opencode opencode
agend app --lang zh-TW
```

**確認：** 介面開啟後，按 `/`，輸入 `try-opencode`，按 Enter 查看它的終端，再按 `i` 取得控制；可輸入「請只回覆試用成功，不要使用工具或修改檔案」。

模型回覆後，按 `Ctrl-]` 退出輸入模式，再按 Esc 返回。這一步只驗證能與 agent 對話；團隊派工還需要設定 repo、角色與 workflow，接著看 [任務流水線操作](docs/architecture/pipeline-runtime.md#命令與協定)。

## 操作速查

| 想做什麼 | 按鍵 |
|---|---|
| 首頁／需要你 | `h`／`!` |
| 移動、開啟 | 方向鍵、Enter |
| 側欄與內容切換焦點 | Tab（寬畫面） |
| 收合／展開團隊 | 側欄中按 `←`／`→` |
| 上一項／下一項待辦 | `[`／`]` |
| 查看終端、取得控制 | `t`、`i` |
| 結束終端輸入、返回 | `Ctrl-]`、Esc |
| 切換配色／語言 | 大寫 `P`／`L`（配色在管理畫面操作） |
| 離開管理介面 | `q` |

**結束試用：** 離開介面不會停止 daemon 或 agent。

1. 返回管理畫面，按 `q` 離開。
2. 在終端 B 執行 `agend instance remove try-opencode`，確認名稱後回答 `y`。若已有任務，先處理或取消任務再移除。
3. 移除成功後，到終端 A 按 Ctrl-C 結束 daemon。資料目錄保留，方便下次繼續。

配色有深色、終端預設、淺色、高對比四種；儲存於 `$XDG_CONFIG_HOME/agend/tui-theme`（未設時為 `~/.config/agend/tui-theme`）。更多按鍵與設定見 [TUI 說明](crates/agend-tui/README.md)。

## 遇到問題

先在設定了同一個 `AGEND_HOME` 的終端執行 `agend doctor`，看警告下方的 `fix:`。

| 畫面或訊息 | 下一步 |
|---|---|
| daemon not reachable | 確認終端 A 還在執行，兩個終端的 `AGEND_HOME` 相同 |
| 找不到 opencode | 先確認 `opencode --version`；自訂位置可用 `instance add … --program /絕對路徑/opencode` |
| authentication unknown | doctor 沒有足夠的目前認證證據；到該 backend 的終端確認登入與回覆 |
| 等待授權／需要你 | 按 `!` 讀取完整目錄、問題與選項，再決定是否允許 |
| 終端只能看、不能輸入 | 按 `i` 取得控制；版本不支援或已有控制者時，依提示處理 |

## 目前進度

- 第 1–12 施工關已完成：任務流水線、三種 backend 通訊、GitHub 與 Telegram 已接入。
- 第 13 關安裝、服務、版本管理與發布產物實作已於 [#158](https://github.com/suzuke/AgEnD/pull/158) 合併；新 TUI 與配色於 [#159](https://github.com/suzuke/AgEnD/pull/159) 合併。
- **尚未完成：** Claude 受管版本的真模型 canary／跨版往返、部分人工及環境驗收、正式版本與公開發布。不能把第 12A 的 Claude 通訊驗收當作第 13 關版本管理驗收。

現有安裝路徑為原始碼與 Actions 驗收產物；尚無 v2 公開 Release、Brew tap 安裝或 crates.io 安裝。完整範圍見 [Roadmap](docs/ROADMAP.md) 與 [目前缺口](docs/gates/gate-13-status.md)。

## 開發文件

| 目的 | 入口 |
|---|---|
| 修改程式、執行測試 | [AGENTS.md](AGENTS.md) |
| 安裝、更新與產物驗證 | [安裝文件](docs/install-release.md) |
| 架構、crate 分工 | [系統圖](docs/images/system.svg) · [ARCHITECTURE](docs/ARCHITECTURE.md) |
| 詞彙與設計決策 | [GLOSSARY](docs/GLOSSARY.md) · [DECISIONS](docs/DECISIONS.md) |
| 施工進度與驗收證據 | [ROADMAP](docs/ROADMAP.md) |

每個 crate 都有自己的 `README.md` 與 `TESTING.md`。授權為 Apache-2.0，見 [LICENSE](LICENSE) 與 [NOTICE](NOTICE)。

## 下一步

先跑一次[介面示範](#先看介面)，再依需要啟動真正的 agent。
