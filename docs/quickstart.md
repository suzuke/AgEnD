# 第一次使用 AgEnD

> **先看這三行**
> - 安裝入口在 [README](../README.md#安裝)；這裡帶你啟動一個 OpenCode。
> - 一次只做一個步驟，確認結果再繼續。
> - 需要兩個終端：A 跑 daemon，B 管理 agent。

## 連上真正的 agent

先完成 [README 的原始碼安裝](../README.md#安裝)。以下以 **OpenCode** 為例，需要先安裝 OpenCode 並確認它能在你自己的終端正常啟動、登入及回覆。實際傳送訊息可能產生模型費用。

### 1. 確认安裝與 PATH

在準備啟動 daemon 的終端執行：

```sh
export PATH="$HOME/.local/bin:$PATH"
agend --version
```

應看到 `agend 0.0.0`（目前驗收版本）。找不到命令時，先完成 README 的安裝步驟。

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

模型回覆後，按 `Ctrl-]` 退出輸入模式，再按 Esc 返回。這一步只驗證能與 agent 對話；團隊派工還需要設定 repo、角色與 workflow，接著看 [第一個任務](first-task.md)。

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

配色有深色、終端預設、淺色、高對比四種；儲存於 `$XDG_CONFIG_HOME/agend/tui-theme`（未設時為 `~/.config/agend/tui-theme`）。更多按鍵與設定見 [TUI 說明](../crates/agend-tui/README.md)。

## 遇到問題

先在設定了同一個 `AGEND_HOME` 的終端執行 `agend doctor`，看警告下方的 `fix:`。

| 畫面或訊息 | 下一步 |
|---|---|
| daemon not reachable | 確認終端 A 還在執行，兩個終端的 `AGEND_HOME` 相同 |
| 找不到 opencode | 先確認 `opencode --version`；自訂位置可用 `instance add … --program /絕對路徑/opencode` |
| authentication unknown | doctor 沒有足夠的目前認證證據；到該 backend 的終端確認登入與回覆 |
| 等待授權／需要你 | 按 `!` 讀取完整目錄、問題與選項，再決定是否允許 |
| 終端只能看、不能輸入 | 按 `i` 取得控制；版本不支援或已有控制者時，依提示處理 |


## 下一步

完成對話後，用 [第一個任務](first-task.md) 試一次建立檔案、檢查與人工確認合併。
