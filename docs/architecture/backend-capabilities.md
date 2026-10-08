# Backend 能力與版本政策診斷

> **TL;DR**
> - `agend doctor` 分開列出 daemon 的能力政策、磁碟版本觀測與歷史 canary。
> - 政策列不是准入結果；目前 holder／driver／認證條件未核實時，runtime eligibility 保持 unknown。
> - 下一步：依需要的能力核對下表；受管版本升級仍走明確 canary／switch 流程。

## 目前四條版本敏感政策

| 能力 | daemon 採用的規則 | 仍需核對 |
|---|---|---|
| Codex 人工終端輸入 | production policy 精確接受完整 `codex-cli 0.159.3` 輸出；停用 policy 不開放版本，verification override 只限明示 instance | holder PID／production launch record、connected link、持久 own-clientId 歸屬、terminal control |
| Claude 啟動畫面辨識 | 依 Claude 2.1.284 錄製的完整 frame；100／140×24；workspace 與其餘內容必須匹配，Ready 僅允許已批准的單行 Try 建議變動 | frame recognition 不等於可送按鍵；仍需 lifecycle、session／holder 與穩定畫面條件；不是登入證據 |
| OpenCode driver endpoint | expected version 優先取 canary scope，其次 managed artifact，否則 unmanaged baseline 1.18.34；endpoint 與 health 的版本須一致 | holder／session 身分、healthy；診斷只描述優先序，不解析 canary scope 或認證 artifact |
| OpenCode permission reply | 另行精確限定 1.18.34，與 endpoint baseline 分開維護 | 當前 holder／endpoint、pending permission／session 與持久決策條件 |

新版 OpenCode 即使通過 managed canary，也不會因此放寬 permission reply 的固定版本限制。Codex 人工輸入的限定同樣不是整個 Codex driver 的版本範圍。Claude 的版本標籤代表錄製來源，完整 frame parser 才是判定方式。

## 證據與作用範圍

- `BackendDiagnosticReply` 的 boot ID 必須與 Fleet 所屬 daemon 一致，instance 配置也必須一致；否則 doctor 不展示該回覆的觀測或政策。
- `capability/<instance>/<id>` 來自該 daemon 的政策，不採用操作員 PATH 上的 CLI 或 doctor 本身的版本政策冒充。顯示 Warn 與 `runtime eligibility unknown`；不授予輸入或啟用能力。
- `observation/<instance>` 是 SQLite 的配置與匹配紀錄。外部 probe 失敗仍可能保留較早成功值；managed reservation 是啟動前意圖，不是存活證明。
- `compatibility/<instance>` 的歷史 canary 綁 artifact、AgEnD build、平台、時間與指定模型；三則短回合不涵蓋所有工具、權限或模型。
- provider 登入有效性、目前執行映像與完整能力驗收仍不可從上述資料推論。daemon binary digest 尚未由此 API 提供。

診斷 RPC 是操作員限定的 1.9 唯讀請求，使用一次性連線與三秒總期限；不啟動 backend、不讀憑證、不自動執行 canary、不改版本。

## 實作與驗證位置

- 政策來源：`CodexInputPolicy`、`screen::claude_startup`、OpenCode endpoint／permission 的兩個獨立常數。
- daemon 回覆：`handlers/backend_capabilities.rs`；consumer：`agend/src/doctor/observations.rs`。
- 原生三 backend canary／managed fleet 的 doctor 測試核對正式 daemon producer；政策單例檢查 verification override 不外溢；boot／配置反例確認政策不跨身分邊界。
- 這些測試使用本機假 backend。真模型、權限與登入驗收仍須依各能力取得證據，不能由表格或 fake 通過取代。

## 下一步

第 13 關的未完成項目以[目前證據與缺口](../gates/gate-13-status.md)為準；不因政策清單已實作而標記整關完成。
