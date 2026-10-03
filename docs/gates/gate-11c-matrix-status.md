# 第 11 施工關 C 段：驗收矩陣執行狀態

> **TL;DR**
> - 此頁連結完整驗收矩陣與真 producer／native source；固定 09a205d 已經全新 verifier r3 重跑並反證。
> - 固定 09a205d 的四個 CI jobs 各核 900 passed／2 既有 ignored，首次真 U17 已獨立核實；使用者已有實機紀錄並要求剩餘行為自動驗證。
> - 下一步：核本批提示修正的固定 head checks／全新 verifier／CI 及清理，等使用者確認 #145 merge；[收尾紀錄](gate-11c-closeout.md)。

範圍以 [P1–P6](gate-11c-proposal.md) 及 [驗收計畫](gate-11c-validation-plan.md) 為準。下表是證據索引，不是最終完成勾選；測試名稱或綠燈本身不能代替核對 assertion 與實際 producer。

| 計畫列 | 可重跑 source／證據 | 目前限制 |
|---|---|---|
| core／協商 | [協定相容](../../xtask/tests/protocol_compat.rs)、[holder 能力](../../crates/agend/tests/terminal_capability.rs)、[client 拒絕](../../crates/agend-client/tests/full_terminal.rs)、[frame golden](gate-11c-frame-order-validation.md)；實際 thumb no-std 已通過 | golden 新增後的 head CI 另核 |
| holder renderer | [真 PTY frame suite](../../crates/agend-holder/tests/terminal_frames.rs)：色彩／style／cursor／CJK／combining／wide edge／alt／逐 byte 切片 | 實機字型與外觀須人工核 |
| 畫面／更新 | [holder capture](../../crates/agend-holder/src/screen/frame.rs)、[真 runtime](../../crates/agend/tests/terminal_runtime.rs)、[App 延遲與倒序](gate-11c-frame-order-validation.md)、[最後 dirty 時效](gate-11c-outer-validation.md) | 本機時效不是所有 host throughput 保證 |
| 歷史 | [真 PTY frame suite](../../crates/agend-holder/tests/terminal_frames.rs)、[full App](../../crates/agend-tui/tests/full_app.rs)、[真 PTY App](gate-11c-native-app-validation.md)：1,000 行淘汰、固定／回底、clamp、normal／alt | 輪流觀看同一畫面的 UX 須人工核 |
| daemon／權限 | [共用契約](../../crates/agend-testkit/src/contract/terminal.rs) 與 [fake／真入口](../../crates/agend/tests/full_terminal_contract.rs)：Acquire／Resize／Input／Release 全 forbidden，尺寸／owner／實收 bytes 不變 | 只開放已驗 0.159.3，未知或其他版本仍拒絕 |
| 多視窗 | [真 daemon hub](../../crates/agend/tests/terminal_hub.rs)、[真 PTY／外層 App](gate-11c-outer-validation.md)、[互動 demo](gate-11c-demo.md)：不同尺寸、最後 i、舊 token／EOF、重取 | 互動 demo 的尺寸是 parser size，kernel stty 由原生 App 另核 |
| TUI renderer | [full App](../../crates/agend-tui/tests/full_app.rs)、[原生 renderer](gate-11c-native-validation.md)：一行狀態、attrs／寬格／cursor／極小尺寸／退出導航 | HollowBlock 用 block fallback；圖形／OSC clipboard 等另列支援邊界 |
| 鍵／paste | [full App](../../crates/agend-tui/tests/full_app.rs)、[真 PTY](gate-11c-native-app-validation.md)、[外層 capture](gate-11c-outer-validation.md)：本機退出碼、mode-aware keys、整次 paste／raw／base64／envelope 拒絕 | 非美式鍵盤與實際終端回報須人工核 |
| mouse | 同上：tracking／SGR／UTF-8、press／release／motion、status／區外、Shift／唯讀歷史 | 終端不回報 Shift 時以唯讀捲動替代 |
| 斷線／壓力 | [client bounds](../../crates/agend-client/tests/full_terminal.rs)、[Source pressure](../../crates/agend-tui/tests/full_source.rs)、[真 hub](../../crates/agend/tests/terminal_hub.rs)、[原生外層](gate-11c-outer-validation.md)：有界 queue、慢 peer／EOF、不重送、20 次 thread／fd／程序清理 | 必須以新 head 的完整 checks 核資源，不沿用前 head CI |
| Codex U17 | [完整 fake](gate-11c-u17-validation.md)：真 App／client／daemon／holder，同 thread、人工 busy／idle、正式 Queue／idle Send、重啟／草稿、兩個自己的 receipt／turn | [真 U17](gate-11c-u17-live-validation.md) 0.159.3 首次通過；第四回合只核自己的 receipt；原證據已獨立核實，版本開放獲同意；新實作仍需另驗 |

## 尚待完成的確認

- 本批修正的固定 head 自動檢查、全新無 context verifier 與雙平台 CI；原 09a205d 通過結果不能冒充新 head。
- 依使用者 2026-10-03 指示，以 native／外層 PTY 自動化完成剩餘行為驗收，清理本批 fixture／程序／target／verifier worktree；原實機紀錄分別保留。
- 實際終端 app／非美式鍵盤／Linux 實機未提供，不作跨環境外觀認證。
- 使用者確認 #145 merge；提案 #144 的 merge 授權不代替實作 PR。

## 下一步

核 [收尾紀錄](gate-11c-closeout.md) 與 PR 的最終固定 head 報告，使用者確認後才合併。
