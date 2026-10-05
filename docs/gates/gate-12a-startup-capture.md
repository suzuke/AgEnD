# 第 12A：被動保存啟動畫面

> **TL;DR**
> - 開發用 `claude_startup_capture` 預設被動保存畫面；額外 opt-in 做受控 trust／development channels 蒐證，不送模型 prompt 或團隊訊息。
> - 真 Claude 2.1.284 已核版本並保存兩寬信任與 development channels 畫面；P5／P6 仍未驗收。
> - 下一步：以真 fixture 補正式啟動處理；後續真確認與模型回合另取授權。

## 範圍

工具建立自己的 `AGEND_HOME`、workspace、Claude push instance 與 session，沿用正式 supervisor 的啟動旗標、三個設定檔和登入環境。它訂閱真 holder frame，取得自己新建 instance 的控制權設定 PTY 尺寸；預設不發送 `Input`。額外 opt-in 的輸入限制見下方模式，不使用 daemon key。沒有人為製造 busy／idle 或 ACK。

初始尺寸設定完成後保存 1–60 秒、最多 512 個有變化的 frame。畫面來自 holder 的字元 cell，依 native cell 的 soft-wrap 標記合併同一邏輯行、跳過寬字元的 leading spacer，再保存遮蔽後文字、soft-wrap 列標記、尺寸、cursor、revision、generation 與 alternate screen 狀態；這份 JSONL 是蒐證格式，尚非 classifier／conformance fixture。寬度範圍 20–200，列數 5–100。

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

secret scan 通過後的真畫面仍需人工檢視再入 Git。只有 native 測試替身的版本標籤不能當成真 CLI fixture；預設模式不按 Down／Enter；蒐證工具本身不更新 `SCREEN_RULES`。

## 原生重驗（不執行 Claude）

```bash
cd /Users/suzuke/AlphaCR-worktrees/AgEnD-g12a-claude-driver
export CARGO_TARGET_DIR=/private/tmp/agend-g12a-driver-target
~/.cargo/bin/cargo build -p agend --bins
~/.cargo/bin/cargo test -p agend-daemon --test claude_startup_capture
~/.cargo/bin/cargo run -p agend-daemon --example claude_startup_capture -- --help
```

測試用自己的 shell producer 經正式 daemon／holder，核對 100／140 欄的實際 PTY 尺寸、繁中／é、正式 argv、未收到輸入、成功與畫面拒絕後的程序／workspace 清理；另核 soft-wrap 電郵仍遮蔽、跨 wrap 的 Bearer 前綴在寫入前拒絕，以及 hash 不合與既有 output 都不啟動 producer。測試產物隨 fixture 結束清理。

## 真 CLI 蒐證（已授權執行，2026-10-05）

完整啟動須指定 `AGEND_REAL_CLAUDE_STARTUP=1`。本次使用者明確「授權」查版本及以下兩種寬度的被動蒐證；命令格式如下：

```text
AGEND_REAL_CLAUDE_STARTUP=1 AGEND_BIN=<本批 agend 絕對路徑>   cargo run -p agend-daemon --example claude_startup_capture --   --program <核准的 CLI 絕對路徑> --sha256 <核准的 SHA-256>   --version-label <另行查詢的版本> --columns 100 --rows 24   --seconds 20 --out <全新證據目錄的絕對路徑>
```

另以 140 欄與另一個 output 蒐證；兩次各用新 session／workspace。本次預算為 0 模型回合、0 按鍵與 0 訊息投遞。工具不送模型 prompt，卻會真正啟動指定 CLI，不能從 native 測試授權推定可以啟動真 Claude。

兩次均成功（100 欄 3 frames、140 欄 2 frames），最後停在預設 `No, exit` 的信任提示；
`startup=not_assessed` 保留原值。固定 executable SHA-256 與另行查詢的版本已核對，
完整命令及遮蔽原始畫面保存於 `AgEnD-ops/g12a-native-checkpoint-20261005/`。
最後完整 frame 的文字已匯出為 [兩個真 fixture](../../crates/agend-core/tests/fixtures/screens/README.md)。
自有 daemon／holder／home／workspace 已清理；未送鍵、未接受信任提示，後續提示與初始 idle 仍缺證據。

## 受控 trust 蒐證（已另獲授權執行，2026-10-05）

`--workspace-trust-control accept` 額外要求 `AGEND_REAL_CLAUDE_STARTUP_TRUST=1`；
預設仍是零輸入。模式只接受另查版本的 2.1.284 標籤、100×24／140×24，
並核 executable SHA-256；標籤本身不是版本查詢或 attestation。

工具持有本次私人 instance 的 operator attach，僅在畫面包含完整信任敘述、確認 footer
及唯一 `Accessing workspace:` 標頭的下一個非空行完整等於本次 canonical workspace 時，
對預設 No 送一次 Down；新畫面選到 Yes 才送一次 Enter。路徑前綴、出現在別處的自有路徑
及重複標頭均拒絕。
只有一個已完成的 Down 才可開始 Enter。初始已選 Yes、未知畫面或錯誤路徑不送鍵；
No 未改變則停在一個 Down，不重送。Enter 後只記畫面，沒有後續按鍵或訊息。

每次送出前將固定 key、frame generation/revision 與 intent 落檔並 sync；完成回條核對
instance／view／generation／attach。失去控制、失去回覆或期限到即失敗，不重送；
`result.json` 記 started／completed，未完成的輸入結果用 null 表示。

這是額外授權的 operator `Input` 蒐證路徑，不是正式 P5 的 `DaemonKey`／revision CAS。
記錄的 revision 是觀測值，operator Input 不帶 expected_revision；不由此認證正式
P5 的原子畫面檢查或人工 owner 拒絕。`production_daemon_key_path_tested=false`、
`startup=not_assessed` 保留此邊界；沒有 development channels 自動確認或初始 idle 推論。

原生替身重播真 No 文字；選到 Yes／後續畫面為 synthetic producer，不能當成真 fixture。
10 個 native 案例覆蓋預設被動、兩寬受控輸入、未知／外來／初始 Yes、No 未切換、
版本／尺寸 preflight，以及原有 hash／遮蔽／清理回歸。使用者另行「授權」後，兩寬受控蒐證均成功：100 欄 8 frames、140 欄 7 frames，
各完成 Down／Enter 一次，共 4 次輸入；未確認後續提示、未送模型 prompt／訊息。
真實選到 Yes 與 development channels 的文字已匯入四個版本化 fixture。
兩次最後均停在 development channels 選單，`startup=not_assessed`，不是初始 idle 證據。
程序與暫存已清理；另只移除本次兩個 canonical workspace 的個人信任設定條目，
其他個人設定值核對不變。執行與清理 manifest 保存於 AgEnD-ops。

## Development channels 蒐證模式（真執行尚未授權）

新增 `--development-channel-control accept`，須同時指定 workspace trust 模式，
另設 `AGEND_REAL_CLAUDE_STARTUP_CHANNELS=1`；既有被動與兩鍵模式不自動增加輸入。
只在本次 generation 的 Down／trust Enter 都已有完整回條後，
對已錄製的完整 development warning、唯一 `Channels: server:agend`、
選到 `1. I am using this for local development` 及確認 footer 送一次 Enter。
其他 server、同名前綴、重複 Channels 標頭、選到 Exit、缺警告或過早出現都不送第三鍵。

模式最多三次 operator Input，不確認後續畫面。即使提示未消失，也不再送 Enter；
`startup=not_assessed`、無 revision CAS、非正式 P5 DaemonKey 的邊界維持。
受控動作結果不明、被拒絕或期限到都失敗並保留證據，不重送。
原生 producer 重播真 development frame；確認後的畫面是 synthetic，未當成真 fixture。
13 個 native cases 包含既有 10 個回歸及三鍵正例、六種拒絕、預設不增加權限／不重送。
13 個 native cases、整個 daemon／fmt／workspace clippy／實際 no-std 通過，
example 缺任一 opt-in 都在 producer／output 建立前拒絕；全新 verifier 待核。
這是下一份待審執行計畫的工具；本次已完成的兩鍵真授權不涵蓋此模式。

## 獨立核對

固定 `356fbef` 獲全新無相關 context verifier 局部 CONFIRMED。原 `4511e21` 的真 holder 跨列 synthetic Bearer 反例會寫入且回成功；修正後同一 producer 重播回失敗並在寫入前拒絕。另核 20／100／140／200 欄與 5／24／100 列的 native 寬字 spacer、多列電郵、零 stdin 與清理；5 個工具回歸、workspace clippy／fmt／實際 no-std 通過。這份結果只驗工具，不證明真 Claude 的版本、提示或 startup complete。

固定 `b0d8074` 的全新 verifier 用真 daemon／holder 找到兩個誤確認反例：外部路徑以自有
workspace 開頭、或錯誤 workspace 畫面在別處提及自有路徑，皆收到 Down／Enter。
原 REFUTED 證據保留；作者改成唯一標頭與完整路徑相等，補兩寬原生拒絕回歸，
修正 `7abb646` 獲同一驗證者局部 CONFIRMED，六個兩寬路徑反例零輸入；
回條故障不重送，整個 daemon／clippy／fmt／實際 no-std 通過並清理。
固定 head 的 push／PR 雙平台 CI 均成功。這份核對不認證正式 P5／P6。

固定 `386d083` 的全新 verifier 局部 CONFIRMED：四個 fixture 逐 byte 對齊來源；
四種 classifier mutation 均被抓到；core／fmt／clippy／實際 no-std、獨立 agend
及 10 個 native capture tests 通過。自有 worktree／branch／target 與程序已清理。
140 欄的 live DB identity 觀察缺口仍保留；不認證正式 P5／P6 或完整 12A。

## 下一步

完成真版本查詢及啟動蒐證後，核對兩種寬度的真畫面，再實作 D40 P5 的已知提示自動處理及 P6 的初始 idle gate；未知提示保留人工入口。其他真 ACK／PATH／conformance 驗收見 [12A 驗收計畫](gate-12a-validation.md)。
