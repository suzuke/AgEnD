# 第 12A：被動保存啟動畫面

> **TL;DR**
> - 開發用 `claude_startup_capture` 經實際 daemon／holder 保存指定寬度的啟動畫面；不送 prompt、按鍵或團隊訊息。
> - 原生替身測試驗證工具本身；真 Claude 版本、P5 自動處理提示與 P6 啟動完成仍未驗收。
> - 下一步：另外取得選定真 CLI 的版本查詢與啟動蒐證授權，再使用本工具。

## 範圍

工具建立自己的 `AGEND_HOME`、workspace、Claude push instance 與 session，沿用正式 supervisor 的啟動旗標、三個設定檔和登入環境。它訂閱真 holder frame，取得自己新建 instance 的控制權只為設定 PTY 尺寸，不發送 `Input` 或 daemon key。沒有人為製造 busy／idle 或 ACK。

初始尺寸設定完成後保存 1–60 秒、最多 512 個有變化的 frame。畫面來自 holder 的字元 cell，保存文字、尺寸、cursor、revision、generation 與 alternate screen 狀態；這份 JSONL 是蒐證格式，尚非 classifier／conformance fixture。寬度範圍 20–200，列數 5–100。

## 產物與拒絕條件

| 項目 | 行為 |
|---|---|
| executable | 必須是絕對路徑；啟動前及結束後比對指定 SHA-256，不執行 `--version` |
| version label | 由呼叫者提供；`version_was_queried = false`，不認證版本一致性 |
| `screens.jsonl` | 首行是啟動 metadata，其餘是真 frame 的文字；識別碼、路徑與帳號內容經既有 redactor／secret scan。已核對的 executable SHA-256 保留原值 |
| `result.json` | 保存成功與否和 frame 數；`startup = not_assessed`，不把空畫面或已捕捉畫面當成啟動完成 |
| output | 只建立全新目錄（0700），檔案 0600；拒絕覆寫既有證據 |
| 拒絕 | hash 不合、secret scan、失去終端／控制、generation／尺寸改變等均回失敗；部分檔案保留供核對，不宣稱成功 |
| 清理 | 成功或畫面拒絕後，經 production instance remove 停 holder，再停 daemon；fixture 移除自己的 home／workspace，保留指定 output |

secret scan 通過後的真畫面仍需人工檢視再入 Git。只有 native 測試替身的版本標籤不能當成真 CLI fixture；本工具不按 Down／Enter，也不更新 `SCREEN_RULES`。

## 原生重驗（不執行 Claude）

```bash
cd /Users/suzuke/AlphaCR-worktrees/AgEnD-g12a-claude-driver
export CARGO_TARGET_DIR=/private/tmp/agend-g12a-driver-target
~/.cargo/bin/cargo build -p agend --bins
~/.cargo/bin/cargo test -p agend-daemon --test claude_startup_capture
~/.cargo/bin/cargo run -p agend-daemon --example claude_startup_capture -- --help
```

測試用自己的 shell producer 經正式 daemon／holder，核對 100／140 欄的實際 PTY 尺寸、繁中／é、正式 argv、未收到輸入、成功與畫面拒絕後的程序／workspace 清理；另核 hash 不合與既有 output 都不啟動 producer。測試產物隨 fixture 結束清理。

## 真 CLI 蒐證（待授權，尚未執行）

完整啟動須指定 `AGEND_REAL_CLAUDE_STARTUP=1`。以下只描述命令格式，不代表已取得執行授權：

```text
AGEND_REAL_CLAUDE_STARTUP=1 AGEND_BIN=<本批 agend 絕對路徑>   cargo run -p agend-daemon --example claude_startup_capture --   --program <核准的 CLI 絕對路徑> --sha256 <核准的 SHA-256>   --version-label <另行查詢的版本> --columns 100 --rows 24   --seconds 20 --out <全新證據目錄的絕對路徑>
```

另以 140 欄與另一個 output 蒐證；兩次各用新 session／workspace。仍須先核准版本／完整命令／回合預算。工具不送模型 prompt，卻會真正啟動指定 CLI，不能從 native 測試授權推定可以啟動真 Claude。

## 下一步

完成真版本查詢及啟動蒐證後，核對兩種寬度的真畫面，再實作 D40 P5 的已知提示自動處理及 P6 的初始 idle gate；未知提示保留人工入口。其他真 ACK／PATH／conformance 驗收見 [12A 驗收計畫](gate-12a-validation.md)。
