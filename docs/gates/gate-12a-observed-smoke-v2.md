# 第 12A：第二次完整 smoke 的啟動失敗

> **TL;DR**
> - 固定 `990bead` 的 observed v2 已另行授權並執行一次；初始 idle 逾時 FAILED，零工作訊息。
> - 24 份原始 frame 核 A Ready／idle、B 的 how-does 提示 None／unknown；補完整已錄製 literal，不放寬未知內容。
> - 下一步：原生回歸與全新 verifier 核對；完整模型通訊尚未通過，任何新真 CLI 計畫須另取授權。

## 固定執行與結果

計畫 SHA-256 `6c23f0e2bc70f45186e107fc5bbd2248de52bca4d2a1fad7bcb3b5c434a4eae7`。
版本、兩個 Haiku instance、七則工作訊息／900 秒／零重跑與六份 bytes 依原固定計畫；
執行前核 namespace freshness，授權紀錄先於首個版本查詢保存。
launcher 將 LANG 設回計畫原定 C.UTF-8，未更改計畫或 executable。

本次一次版本查詢、兩次 instance add、24 次只讀 frame 擷取與 683 次 status；
每個 instance 一個 live SessionStart，正式 TrustNo／TrustYes／Development 三鍵各 written，manual=0。
A startup halted=1，B halted=0；38 次 status 兩者 unknown，645 次 A idle／B unknown。
初始等待 180 秒逾時後停機，沒有 test send、native messages、delivery 或 ACK；未自動重跑。
執行結束碼 1，不能將原生測試或本次 frame 捕捉稱為完整 smoke PASS。

## 畫面與修正

每個 instance 首份 frame 為空白，後續十一份各自相同。
A 為既有 `Try "edit <filepath> to..."`，source990 classifier Ready；
B 為 `Try "how does <filepath> work?"`，原 classifier None。
只替 B 的提示為舊值後完整 token 比對為 Ready；其餘 footer 與既有 `100x24-2` 相同。

新 `claude-2.1.284-main-100x24-4.txt` 逐列匯出 B 第二份真 frame，只遮 canonical workspace；
24 列、空白、NBSP 均保留，[來源 JSON／fixture SHA](../../crates/agend-core/tests/fixtures/screens/README.md)。
只新增完整 Ready literal；不送額外鍵、不改 P5 信任選項、未知內容仍拒絕。
新 native P6 回歸用此 frame 重播真 daemon／holder／PTY；
修正前已重現三鍵完成但首次 poll 仍 0，保留 exit101 反例。
修正後核 SessionStart 先到、三鍵完成後重等五秒、首次 poll 可取訊息及 Ready halted=1。
作者 `accept core`（含 fmt／workspace clippy／core／wire／實際 no-std）與啟動八例通過；
啟動回歸耗時 121.13 秒，只啟動 native shell producer，不啟動真 Claude。
改動前後實際 `check-deps` 亦已通過，新 literal 另交全新 verifier 覆核。

全新無相關 context verifier 獨立核六份 pins、命令／native／frame 身分、固定990 classifier，
11 個語意突變與五個 native 負例均拒絕；結果 CONFIRMED_WITH_LIMITS，非 smoke PASS。
授權紀錄比最早命令完成 timestamp 早 317ms；命令開始時間未保存，
對話授權 provenance 仍為 root 聲明，不能將完成時間當開始時間。

## 清理與保留

自有 daemon／holders／backend 已停止，native helper 回 A／B Gone、owned holders absent。
home、兩個 scratch namespaces、十個 personal session 路徑共 13 精確路徑均 absent；
記錄 PID／PGID 與本次兩個 account trust keys 核 absent。
trust 清理只刪執行前不存在的精確兩鍵，保留當下其他 parsed values；
open-descriptor／byte／inode guard 是當下檢查，非全域鎖或未來 race 保證。

必要 private evidence 保存於 `/Users/suzuke/Documents/Hack/AgEnD-ops/g12a-live-smoke-20261006/`。
`session-evidence` 為空，沒有 transcript／usage，不能推斷精確 API 次數、實際模型呼叫數或費用。
未合併 author worktree 與既有固定計畫 binaries 保留；本批新編譯 target 已清除。

## CI 與下一步

990 的 push 雙平台 CI 通過；PR Ubuntu 通過、macOS FAILED：
`actual_app_with_background_startup_sampling_keeps_final_dirty_output_within_budget`
測得 301.612416ms，超過原 300ms。原失敗 log 保留，未放寬門檻或用重跑掩蓋。
本次 fixture 修正不能宣稱修復該效能失敗。

完成獨立覆核及清理後提供可重驗指令；完整訊息階段尚未驗收。
新真 CLI 計畫依 [D40](../decisions/d40.md) 另取授權，#153 merge 等使用者確認。
