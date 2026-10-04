# 第 12 施工關：其餘 adapter（`adapters`）

> **TL;DR**
> - A claude、B opencode、C GitHub forge、D Telegram；A 段已開始實作，接入尚未完成。
> - 第 1–11 施工關已完成並合併；A 段設計 D40 已於 #138 合併（`4390633`）。
> - 下一步：單次 client 請求基礎 #147 已 merge；先完成持久化，再串接 channel／Stop 與明確 ACK。

## 狀態

**實作中**（2026-10-04）。使用者「merge後開工」已執行：[PR #138](https://github.com/suzuke/AgEnD/pull/138) 設計文件合併為 `4390633`，client 基礎已於 #147 合併為 `8dfccf8`，持久化在 `feat/gate-12a-claude-store` 的獨立 worktree 進行。設計見 [D40](../decisions/d40.md)，來源見[確認紀錄](gate-12a-confirmations.md)。首批不重送的 client 請求基礎已合併，新增 [store 基礎](gate-12a-store.md)；Claude driver、channel／Stop／ACK 與 A 段驗收尚待完成。implementation merge 仍等使用者確認。

## 四段範圍

| 段 | 範圍 | 目前進度 |
|---|---|---|
| A claude | channel、hooks、啟動設定、權限、忙碌策略、送達、清掃 | 設計已 merge；client 基礎 #147 已 merge；持久化實作中，Claude 接入與驗收未完成 |
| B opencode | `opencode serve`、session、送達與權限；三個 backend 互傳訊息 | A 段完成後另寫細案，尚未確認 |
| C GitHub forge | push、PR、checks、head 對帳、merge 與收尾 | 第 10 關依賴已完成，細案尚未確認 |
| D Telegram | notifier、allowlist、token、手機處理需要你、G4 已讀狀態 | 第 10 關依賴已完成，細案尚未確認 |

B／C／D 仍未實作；四段原提案的歷史參考保留在 `74ced40`，不視為已確認設計。

## 本次對齊的現況

| 舊提案的假設 | 現在的 baseline | A 段要處理 |
|---|---|---|
| 第 9、10 關尚未實作，11 尚未完成 | 第 1–11 關已完成 | 使用現有 CLI、pipeline、完整終端契約 |
| Claude 請求預定放 protocol 1.4 | 1.4 已是完整終端 capability，一般請求仍只需 1.3 | P1 已採下一 minor 1.5；保留 1.3／1.4 相容性，精確 schema 待實作稿 |
| 下一個 migration 可能是 0005 | 已有 0005 pipeline 與 0006 Codex 人工輸入 thread | P7 已新增 0007 的事件／投遞表與 store API；runtime 尚未串接 |
| ingest 的 spool 已經寫好 | `ingest.rs` 與 `driver/claude.rs` 都只有模組說明 | bridge、hooks、spool 與 Claude Driver 接入仍須實作 |
| 手動逐步驗收大部分 fake 行為 | 使用者要求可自動化的驗證由 agent 執行，完成後清理 | fresh verifier 重跑，提供可重驗指令；真 CLI／模型回合另需授權 |

此表對齊目前程式與已確認設計；0007 與 store API 已實作，尚待本批驗證與 merge；1.5／runtime 尚未接入，不能視為 A 段功能驗收通過。

## P1–P10 決策入口

| 題目 | 內容 | 狀態 |
|---|---|---|
| [P1](gate-12a-proposal-bridge.md#p1claude-的訊息與-hook-怎麼到-daemon) | channel／hook 如何連 daemon | 已確認；D40 |
| [P2](gate-12a-proposal-bridge.md#p2claude-讀哪些設定我們的檔放哪) | 設定檔、既有檔案與 fake-worker | 已確認；D40 |
| [P3](gate-12a-proposal-permissions.md#p3claude-的權限模式安全決定請選一個) | bypass／deny／auto | 已選 A；D40 |
| [P4](gate-12a-proposal-permissions.md#p4agent-可以用你的-gh-登入直接-merge安全決定請選一個) | agent 的 gh 防護 | 已選 A；D40 |
| [P5](gate-12a-proposal-permissions.md#p5claude-的啟動對話框) | 信任與 development channels 提示 | 已選 A；D40 |
| [P6](gate-12a-proposal-delivery.md#p6claude-的三級忙碌與忙閒) | queue／steer／interrupt | 已確認；D40 |
| [P7](gate-12a-proposal-delivery.md#p7claude-的送達確認與事件) | sent／confirmed、spool 與事件 | 已確認；D40 |
| [P8](gate-12a-proposal-delivery.md#p8claude-不需要-zdotdir) | Bash PATH 與 shim | 已確認；D40 |
| [P9](gate-12a-proposal-delivery.md#p9holder-死掉後清掃-claude) | holder 死後的孤兒清掃 | 已確認；D40 |
| [P10](gate-12a-validation.md#p10什麼是假的什麼是真的) | fake／真 CLI 驗證範圍 | 已確認；D40 |

確認 checklist 已依使用者回覆勾選；P3／P4／P5 保留原比較選項，採 A。P7 舊 hook 即 confirmed 被明確 agend_ack 取代。D16、送達冪等、busy effective_level、holder 重啟與 resume、環境白名單、shim 防手滑等已確認決定沿用。

A 段原限制：不加新的 core 事件或未知提示／忙閒判斷機制。P1 是 core 協定請求，P5 是現有螢幕規則資料改動；兩處實作後都須依 D22 重驗 core。B／C／D 不由此次整理推定核准。

## 證據與驗收

- [2026-09-28 F1–F10 歷史實測](../research/gate-12a-claude-2026-09-28.md)：Claude 2.1.283，5 個短回合、8 次不送 prompt 的啟動。錄製檔是 2.1.282；不能宣稱目前安裝版本已通過。
- [P10、U1–U4、限制與驗收計畫](gate-12a-validation.md)：真測授權、測試接點、未完成的案例與清理。
- G4 已讀狀態留 D 段，與 TUI／Telegram 共用；見[第 8 關](gate-08-client.md)。

## 你親自驗收

Claude 接入／A 段功能驗收尚未完成，目前沒有可執行的功能驗收指令。[驗收計畫](gate-12a-validation.md#真測與使用者可重驗)保留互傳訊息與權限等情境；A 段實作及自動驗證完成後，再提供逐步指令。真 CLI／模型回合須另行取得版本、範圍與預算授權。

## 驗收紀錄

| 日期 | 範圍與結果 | 備註 |
|---|---|---|
| 2026-10-03 | 文件整理；不是 A 段實作驗收 | fresh verifier／固定 head CI 結果以 #138 為準；P1–P10 待決定 |

## 進度紀錄

- 2026-10-04：`6193ea4` fresh verifier REFUTED：event-only DB 的每日快照未納入新表而被跳過；保留原失敗與反例，修正 snapshot empty 判斷，另派全新 verifier 重驗（#148；未 merge）。

- 2026-10-04：持久化批次建立 schema v7／投遞／ACK／人工終結與 retention API；首輪真 SQLite、舊 fixtures 與 accept core 通過。完整 workspace／fresh verifier／CI 另核，完整 Claude 接入未完成、此批未 merge（`6193ea4`／#148；[範圍](gate-12a-store.md)）。

- 2026-10-04：#147 經使用者確認合併為 `8dfccf8`，最新全新 verifier CONFIRMED，push／PR 雙平台 CI 通過；第一次 PR macOS TUI 時序超時原 log 保留，重跑通過且未改 300 ms 門檻。舊 worktree 清理；下一批 [Claude 持久化](gate-12a-store.md) 開工，完整 A 段尚未完成。

- 2026-10-04：首批 `7b1baeb` 獨立覆核 REFUTED，發現預編碼大輸入超出 deadline 與狀態入口殘留；修正後另驗，原反例保留（draft PR #147）。
- 2026-10-04：首批 CI Ubuntu 通過、macOS 共用期限測試失敗；調整測試握手排程餘裕，修正版 CI 另核（#147）。
- 2026-10-04：`e068e59` 第二位 fresh verifier 仍 REFUTED：上限內字串編碼與回覆解析越過 CPU deadline；補分段編碼／解析檢查及原反例回歸，待全新 verifier 另驗（#147）。
- 2026-10-04：`2d754bb` 第三位 fresh verifier REFUTED：serde 中間樹轉換 CPU 尾段在 native API 仍超過 100 ms 餘裕；改成 RawValue envelope 分派正式 core 資料型別，保留反例及各版結果，待全新 verifier（#147）。
- 2026-10-04：`b5629be` 的覆核 REFUTED：巢狀 AskEntry options 六次超過原 100 ms 餘裕；ask／entry／reply 也改分段解析，補完整問答型別及大 options 真 producer 回歸，修正版另驗。另一次平台中斷記未完成（#147）。
- 2026-10-04：`8594cba` fresh verifier CONFIRMED，使用者 46 tests 通過並清理暫存；PR CI 雙平台通過，push macOS 的 socket／解析逾時先後造成測試型別斷言失敗。加直接解析的 CPU 期限斷言，native 路徑接受 I/O 逾時，原期限／餘裕不變，修正版另驗；產品邏輯未改（#147，未 merge）。

- 2026-10-04：使用者確認 merge 後開工，#138 合併為 `4390633`；在 `feat/gate-12a-claude` 建立單次 client 請求基礎，Claude 接入與驗收未完成。
- 2026-10-04 P1–P10 設計確認寫定為 [D40](../decisions/d40.md)，保留 P3／P4／P5＝A，P9／P10 及剩餘參數採建議；#138 本批只改文件，fresh verifier／固定 head CI 另核，尚未 merge 或實作。以下保留各批次當時狀態。

- 2026-10-03 使用者授權先整理 #138；對齊第 1–11 關已完成的 v2、拆頁並保留 F1–F10 原範圍。P1–P10 仍待決定，不包含提案 merge 或實作授權。
- 2026-09-28 A 段 P1–P10 提案寫定（#138、`05b0621`），仍未確認；四段原版在 `74ced40`。前兩輪 review 曾 REFUTED，歷史結果不改成通過。
- 2026-09-25 使用者決定真 CLI 一致性檢查列為完成條件；fixture 必須與驗收版本一致。

## 下一步

依已合併 [D40](../decisions/d40.md) 實作 Claude 接入；client 單次請求基礎已合併；目前建立持久化 API，再串接 channel／Stop 與明確 ACK。完成後提供全新 verifier 與可重驗指令，implementation merge 等使用者確認。
