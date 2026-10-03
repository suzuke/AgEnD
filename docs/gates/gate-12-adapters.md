# 第 12 施工關：其餘 adapter（`adapters`）

> **TL;DR**
> - A claude、B opencode、C GitHub forge、D Telegram；目前只有 A 段開工前提案 P1–P10。
> - 第 1–11 施工關已完成並合併；#138 整理自目前 v2，P1–P10 全部仍待使用者決定。
> - 下一步：從 P1 開始，一次解釋一題並等使用者決定；提案 merge 與 A 段實作另待確認。

## 狀態

**提案整理中**（2026-10-03，[draft PR #138](https://github.com/suzuke/AgEnD/pull/138)）。使用者只授權整理文件、push 與 CI；P1–P10 尚未確認，Claude driver 尚未實作。第 11C #145 已 merge，本次 baseline 是文件 #146 合併後的 `d21d344`。

## 四段範圍

| 段 | 範圍 | 目前進度 |
|---|---|---|
| A claude | channel、hooks、啟動設定、權限、忙碌策略、送達、清掃 | P1–P10 待確認 |
| B opencode | `opencode serve`、session、送達與權限；三個 backend 互傳訊息 | A 段提案確認後再寫 |
| C GitHub forge | push、PR、checks、head 對帳、merge 與收尾 | 第 10 關依賴已完成，細案尚未確認 |
| D Telegram | notifier、allowlist、token、手機處理需要你、G4 已讀狀態 | 第 10 關依賴已完成，細案尚未確認 |

B／C／D 仍未實作；四段原提案的歷史參考保留在 `74ced40`，不視為已確認設計。

## 本次對齊的現況

| 舊提案的假設 | 現在的 baseline | A 段要處理 |
|---|---|---|
| 第 9、10 關尚未實作，11 尚未完成 | 第 1–11 關已完成 | 使用現有 CLI、pipeline、完整終端契約 |
| Claude 請求預定放 protocol 1.4 | 1.4 已是完整終端 capability，一般請求仍只需 1.3 | P1 提議下一 minor 1.5；保留 1.3／1.4 相容性，仍待確認 |
| 下一個 migration 可能是 0005 | 已有 0005 pipeline 與 0006 Codex 人工輸入 thread | P7 開工時取下一空號；目前為 0007，尚未新增 |
| ingest 的 spool 已經寫好 | `ingest.rs` 與 `driver/claude.rs` 都只有模組說明 | bridge、hooks、spool 與 Claude Driver 接入仍須實作 |
| 手動逐步驗收大部分 fake 行為 | 使用者要求可自動化的驗證由 agent 執行，完成後清理 | fresh verifier 重跑，提供可重驗指令；真 CLI／模型回合另需授權 |

此表是文件與程式碼對齊，不代表核准 1.5、0007 或新增實作。

## P1–P10 決策入口

| 題目 | 內容 | 狀態 |
|---|---|---|
| [P1](gate-12a-proposal-bridge.md#p1claude-的訊息與-hook-怎麼到-daemon) | channel／hook 如何連 daemon | 待確認 |
| [P2](gate-12a-proposal-bridge.md#p2claude-讀哪些設定我們的檔放哪) | 設定檔、既有檔案與 fake-worker | 待確認 |
| [P3](gate-12a-proposal-permissions.md#p3claude-的權限模式安全決定請選一個) | bypass／deny／auto | 待選 A／B／C |
| [P4](gate-12a-proposal-permissions.md#p4agent-可以用你的-gh-登入直接-merge安全決定請選一個) | agent 的 gh 防護 | 待選 A／B／C |
| [P5](gate-12a-proposal-permissions.md#p5claude-的啟動對話框) | 信任與 development channels 提示 | 待選 A／B／C |
| [P6](gate-12a-proposal-delivery.md#p6claude-的三級忙碌與忙閒) | queue／steer／interrupt | 待確認 |
| [P7](gate-12a-proposal-delivery.md#p7claude-的送達確認與事件) | sent／confirmed、spool 與事件 | 待確認 |
| [P8](gate-12a-proposal-delivery.md#p8claude-不需要-zdotdir) | Bash PATH 與 shim | 待確認 |
| [P9](gate-12a-proposal-delivery.md#p9holder-死掉後清掃-claude) | holder 死後的孤兒清掃 | 待確認 |
| [P10](gate-12a-validation.md#p10什麼是假的什麼是真的) | fake／真 CLI 驗證範圍 | 待確認 |

每題的原建議都保留；checklist 沒有代使用者勾選。D16、送達冪等、busy effective_level、holder 重啟與 resume、環境白名單、shim 防手滑等已確認決定沿用。

A 段原限制：不加新的 core 事件或未知提示／忙閒判斷機制。P1 是 core 協定請求，P5 是現有螢幕規則資料改動；兩處都須確認並依 D22 重驗 core。B／C／D 不由此次整理推定核准。

## 證據與驗收

- [2026-09-28 F1–F10 歷史實測](../research/gate-12a-claude-2026-09-28.md)：Claude 2.1.283，5 個短回合、8 次不送 prompt 的啟動。錄製檔是 2.1.282；不能宣稱目前安裝版本已通過。
- [P10、U1–U4、限制與驗收計畫](gate-12a-validation.md)：真測授權、測試接點、未完成的案例與清理。
- G4 已讀狀態留 D 段，與 TUI／Telegram 共用；見[第 8 關](gate-08-client.md)。

## 驗收紀錄

| 日期 | 範圍與結果 | 備註 |
|---|---|---|
| 2026-10-03 | 文件整理；不是 A 段實作驗收 | fresh verifier／固定 head CI 結果以 #138 為準；P1–P10 待決定 |

## 進度紀錄

- 2026-10-03 使用者授權先整理 #138；對齊第 1–11 關已完成的 v2、拆頁並保留 F1–F10 原範圍。P1–P10 仍待決定，不包含提案 merge 或實作授權。
- 2026-09-28 A 段 P1–P10 提案寫定（#138、`05b0621`），仍未確認；四段原版在 `74ced40`。前兩輪 review 曾 REFUTED，歷史結果不改成通過。
- 2026-09-25 使用者決定真 CLI 一致性檢查列為完成條件；fixture 必須與驗收版本一致。

## 下一步

先說明 [P1：訊息與 hook 到 daemon](gate-12a-proposal-bridge.md#p1claude-的訊息與-hook-怎麼到-daemon)，等使用者決定後才說下一題。
