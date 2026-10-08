# Canary 認證與模型

> **TL;DR**
> - `backend canary --allow-model --model <id> --auth-file <絕對路徑>` 使用明確的專用認證來源。
> - 不搜尋共享帳戶、不複製設定／plugins、不寫回來源；成功或已確認停止的失敗流程會清理私有 home。
> - 目前只有原生假後端驗證；真 backend 的登入與三次模型回覆仍須另行驗收。

## 輸入與目的地

`--auth-file` 可省略；省略不繼承 daemon 的 API key／token 環境變數。`--allow-model` 仍是任何 canary 執行的必要條件。
檔案必須是本使用者擁有、group／other 無權限、單一 hard link 的一般檔案，最多 1 MiB；拒絕最後一層 symlink、FIFO、讀取中變更與非法內容。錯誤不回傳檔案內容。

| Backend | 專用檔案格式 | 本次私有 home 內位置 |
|---|---|---|
| Codex | backend 產生的 `auth.json` JSON object | `probe-home/.codex/auth.json`，另建只有 `cli_auth_credentials_store = "file"` 的 config |
| OpenCode | backend 產生的 `auth.json` JSON object | `opencode/canary/data/opencode/auth.json`，沿用正式 wrapper 的 XDG data 目錄 |
| Claude | `claude setup-token` 取得的單行 OAuth access token，最多 16 KiB | `canary-auth/claude-oauth-token`；核 scope 後才轉成該 holder 的 `CLAUDE_CODE_OAUTH_TOKEN` |

目的檔案為 0600、建立的目錄為 0700，已有目的檔時拒絕覆寫。AgEnD 不解析或證明 JSON 憑證有效；登入結果仍由 backend 與後續 canary readiness 決定。

私有 scope 核 home inode／device、來源 artifact、instance、backend 與 workspace。canary 的 `HOME` 強制指向私有 `probe-home`；Codex 的 `CODEX_HOME` 與 Claude 的 `CLAUDE_CONFIG_DIR` 也指向該目錄。一般 fleet 的環境白名單不增加憑證。Claude 的共享 trust entries 不讀寫，這個流程也不執行 `/login` 或 `/logout`。

## 模型與證據

`--model` 接受單一模型 ID；Codex app-server 轉成全域 `-c model="…"`（不傳 TUI 的 `--model`）；OpenCode 要求 `provider/model`。模型參數精確綁入 scope，替換／移除或加任意旗標不能沿用該 scope。報告 `model` 是要求值，不是供應者解析後模型的證明，也不表示新版本已啟用。

使用專用測試帳戶或專用 token。複製檔案只隔離本機儲存，不隔離供應者帳戶、額度或 refresh token 輪替；不要把正在使用的共享 OAuth session 當測試專用來源。來源檔案由操作者保留，AgEnD 不同步 refreshed credential 回去。

若程序無法確認已停止，沿用 canary 的保留 home／失敗報告規則；該 home 可能包含認證副本。只有確認自有程序停止後才清理，不以刪檔掩蓋仍在執行的程序。

## 官方依據與驗證邊界

- [Codex 認證](https://learn.chatgpt.com/docs/auth)：file store 位於 CODEX_HOME，文件也說明複製 cache 與 refresh 行為。
- [Claude 環境變數](https://code.claude.com/docs/en/env-vars)：OAuth access token 可供自動化，優先於 Keychain；[帳戶隔離](https://code.claude.com/docs/en/authentication) 說明 CLAUDE_CONFIG_DIR。
- [OpenCode credentials](https://dev.opencode.ai/docs/providers)：auth.json；AgEnD 的正式 XDG 位置見 [12B 帳戶設定](../gates/gate-12b-account-setup.md)。

這些文件支持介面選擇，不取代固定 backend 版本的真測。雲端身份、custom provider、企業管理設定與 Keychain 原生登入不由本入口代辦。

## 下一步

固定版本與模型、專用來源檔、三訊息預算及清理計畫後，再執行授權的真模型 canary。
