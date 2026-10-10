# 第 13 施工關：目前證據與完成缺口

> **TL;DR**
> - 第 13 關仍未完成、未核准合併；PR #158 是 draft。
> - 功能提交、局部測試、整關驗收與真環境驗收分別記錄，不能互相替代。
> - 下一步：以已通過的自動驗收為基線，完成以下缺口，再交付 final fresh verifier 與逐步人工驗收。

此頁整理 2026-10-09 的當前狀態；歷史過程保留在 [施工關紀錄](gate-13-install.md)。

| 範圍 | 已實作／已有證據 | 尚未證明完成 |
|---|---|---|
| 13A home／設定 | 預設 home、init 權限、doctor 的配置程式解析、沙箱與服務診斷；install_home 回歸 | doctor 已分開執行檔／歷史 canary 範圍／unknown 登入；daemon 唯讀配置／觀測快照已接入並核 boot；四條版本敏感能力政策已列出；登入有效性、能力真環境證據與故障→修復人工矩陣仍待完成 |
| 13B 服務 | launchd／systemd plan、安裝所有權、對帳、解除安裝、資料刪除互斥；63b083b 的 Linux ARM64 正式 archive 在隔離 systemd／一般使用者下通過五階段生命周期；macOS 原生 launchd 安裝／停機／重啟接回同 holder 與子程序／解除安裝保留資料通過，測試資源已清理 | macOS 已補正式 ARM64 archive 的相同流程；使用 C 等待程序，真 backend 與登入登出未測試 |
| 13C 受管版本 | 匯入、canary 收據、版本切換／回退、身份綁定與 pending 恢復；原生假 backend 測試；固定三種真 CLI 的 import／inspect 與搬移後版本探測通過 | Claude／Codex 真模型 canary；三 backend 切換與回退完整驗收及所有故障切點。OpenCode 1.18.34 三訊息 canary 已通過（見下方紀錄） |
| 13C 版本發現 | 固定 npm 公開 metadata、每日持久排程、外部 CLI 磁碟版本探測、待辦與 Telegram 精確確認 | 真實系統 CLI 的版本觀測驗收；雙 monitor 同時 active 的正式 daemon 停機兩種完成順序已通過 |
| 13C 重啟通知 | 真 daemon 三次啟動測試：worker 發現換版、通知重建、操作員確認持久化，agent 與錯誤 action 拒絕 | 此測試使用本機假 CLI，不能代替真模型或 service-manager 重啟驗收 |
| 13D Telegram | daemon 配對、token reference、allowlist、設定套用；專用 bot 真測完成 /start→confirm→apply→同 home 重啟→通知→Mark read→approve，任務 done 且唯一 merge | 本次限私聊人工 approval；不擴稱覆蓋所有 Telegram 動作或真模型 |
| 13E 打包 | 63b083b 的 release-artifacts 37800713214 通過四平台 archive、formula 與雙平台 Brew 七個 jobs | 全新使用者真 backend 首任務、版本／發布交付計畫；尚未公開發布 |
| 整關 | 63b083b 的 fresh-context accept install、雙平台 CI、四平台打包均通過 | 剩餘實作與真測完成後的最終 fresh-context verifier、使用者逐步驗收與合併確認 |

doctor 逐項覆蓋與缺口見 [故障／修復矩陣](gate-13-doctor-matrix.md)。

## 當前自動驗收

- 固定 `63b083be43642e76ea94f20323900d0e98315727`，fresh-context verifier 一次執行 `cargo xtask accept install`，exit 0、1,969.572 秒，起訖 HEAD 與工作樹乾淨。checks 印出 898 passed／0 failed／4 ignored（含 child entry 計數，非 898 個不同契約）；no-std 實際通過。demo 56 passes 重複選定 suites；假 worker 首任務 4,774 ms、一次 merge、0 模型。獨立清理核對移除 41 個該輪新增、無 listener 的假 Codex socket，未碰外來項目。
- 同版 [CI 37798841941](https://github.com/suzuke/AgEnD/actions/runs/37798841941) 雙平台成功；[release-artifacts 37800713214](https://github.com/suzuke/AgEnD/actions/runs/37800713214) 四平台 archive、formula、雙平台 Brew 七 jobs 成功。Linux ARM64 正式 archive 在隔離 systemd／非 root UID 下完成五階段服務驗收，78.917 秒；container、image、暫存皆移除。這些不證明真模型、新使用者認證或公開發布。
- 2026-10-09 私聊 Telegram 真測通過：配對套用後重啟同一 home，確認配對收據保留；Mark read 產生正確 read key 且仍等待 approval，approve 的 accepted 收據綁原通知，任務 done、一次雙親 merge。使用 fake workers、0 模型。自有 daemon／holder／worker 與 home／repo 全部清理。前兩輪 external harness 因 SQLite exclusive lock 與缺少 agend PATH 失敗；分別修正為停 daemon 後讀收據、補 PATH，離線預驗與獨立腳本覆核後才重跑，沒有修改 production 來遷就驗收。
- 2026-10-09 macOS 原生 launchd 流程 11.054 秒通過：使用固定 63b083b debug binary 與隔離 home，正式 service install／bootout／重新安裝使 daemon PID 改變，同一 holder 與 C 等待子程序持續存活且有 reattach log；修改自有 plist 時 uninstall 拒絕，恢復後 uninstall 保留 DB／設定／資料。最後 label、程序與 root 均清理。這不等於真 backend 或 release archive 驗收；修改定義的反例未單獨驗證拒絕期間每個程序持續存活。證據 `launchd-lifecycle-g13-ld-9q9x8rht.json`；前輪複製系統 sleep 被終止、且 sleep 不接受 session 參數的失敗與離線重現保留，改用可接收 backend 參數的專屬 C fixture 後才重跑。
- 2026-10-09：固定 `63b083b` 的 macOS ARM64 正式 archive 通過原生 launchd 安裝、重啟接回同 holder／C 等待子程序、變造定義拒絕與解除安裝保留資料，6.407 秒完成；service label、程序、測試 root 與下載解壓目錄已清理。archive SHA-256 `2771f94734ff99369e0aa33237c9a47b893cde84f59860b4ecfb48f6154d3ad0`，binary SHA-256 `07796bc38a3921542e95d350e0e20b5f6209aaa3a4fe8c563dd0e9191713833b`。證據 `launchd-lifecycle-g13-ld-tc9ywmro.json`、`mac-release-cleanup.json`；未要求模型或認證操作，不代替真 backend 與登入登出驗收。
- 本機原始證據保留於 `AgEnD-ops/g13-install-20261008`：`accept-63b083b-fresh/REPORT.txt`、`linux-release-service-63b083b-thread-scan-result.json`、`launchd-capture-result.json`、`telegram-live-control-g13-tgc-i3q3b1c9.json`。以下舊版紀錄是歷史證據，不取代上述固定提交結果。

- 固定提交 `d46d53b` 的 `cargo xtask accept install` exit 0，耗時 2,039.20 秒，起訖 HEAD 相同且工作樹乾淨；安裝 demo 的 canary 17/17、Telegram pairing 2/2、服務模型 14/14 通過。全新 HOME 首任務 `t-1` 在 4,853 ms 完成且恰好一次 merge（fake workers，0 模型呼叫）；demo root 已移除，程序 argv 核對未見匹配的自有 agend／worker。此清理核對僅涵蓋該 demo，不冒充整套測試所有 lab 的獨立稽核。
- 同一 `d46d53b` 的 [release-artifacts 37791663365](https://github.com/suzuke/AgEnD/actions/runs/37791663365) 七個 jobs 全部成功：四平台 archive、formula、macOS／Linux Brew。產物尚未公開發布；該提交的 [CI 37791469987](https://github.com/suzuke/AgEnD/actions/runs/37791469987) 已在 macOS／Ubuntu 全部成功。前述成功不解釋 `7907e7e` 偶發輸出上限失敗的未知根因。

- 固定程式提交 `f08fe43` 的 `cargo xtask accept install` 以 exit 0 完成，耗時 2,200.61 秒；起訖 HEAD 相同且工作樹乾淨。包含 core／daemon／CLI checks、安裝 demo、三種原生假 backend canary、服務 lifecycle 模型與配對隔離測試。
- 該版全新 HOME 首任務 `t-1` 完成，恰好一次 merge，耗時 5,597 ms；使用 fake workers，0 模型呼叫。自有 HOME／repo 已清理。
- [CI 37778088087](https://github.com/suzuke/AgEnD/actions/runs/37778088087) 的 `f08fe43` 在 macOS／Ubuntu 成功；不能把結果套用到後續修改。
- `d78f530` 的 [release-artifacts 37783051296](https://github.com/suzuke/AgEnD/actions/runs/37783051296) 七個 jobs 全部成功。正式 Linux ARM64 archive（binary SHA-256 `caaeee85b06c830c6d70822c1edf965981fd295d0ab90a5dd9967709ae32873f`）以原生 busctl、非 root 使用者驗安裝、重啟接回相同 holder／agent 世代、解除安裝保留資料與外來 drop-in、拒絕外來 MainPID、完整清理。隔離 container／image／暫存已刪除；未碰主機服務、未用真 backend，未驗登入登出。成功案例未記錄 readiness false→true，該分支另由捕獲的原生 fixture 與前置實驗支持。
- `d78f530` 的 macOS 26 PR CI 在 self-setsid fixture 準備階段收到 EPERM。`64c35eb` 僅為 fixture 加入限定 EPERM／父程序群組的有界重試，必須建立真 session，清理前仍存活且最終由 SIGKILL 終止；production 期限與清理邏輯未改。本機 binary 40 項、fmt／clippy／check-deps 通過；該提交的 [CI 37785631445](https://github.com/suzuke/AgEnD/actions/runs/37785631445) 已在 macOS／Ubuntu 全部成功。
- 歷史 `9a8bcf7` CI 的 pipeline／GitHub pipeline WIP 與返工失敗，經 `e64e629` 修正 fake-worker --version 誤入 inbox 後，本機完整 pipeline 15/15、GitHub pipeline 5/5 與上述雙平台 CI 通過。
- 前輪 accept install 的 Codex 假 CLI 版本 probe 曾逾時；單例、並行全組及本輪 accept／demo 均通過，但原偶發逾時未穩定重現。`f762f47` 只增加 elapsed_ms／budget_ms／stdout_bytes，不記原始輸出、不重試、不延長期限。
- 以上歷史自動驗收不代表真 backend 或主機服務完整驗收；本輪獨立 accept 與 Telegram 真測的範圍見本節開頭。最終整關覆核仍待剩餘工作完成。

## 本批進度

- 2026-10-09：正式 backend import／inspect 匯入固定 Claude 2.1.284、Codex 0.159.3、OpenCode 1.18.34 原生檔，雜湊核對後由 doctor 對搬移副本執行版本探測，三者皆 ok；14.211 秒完成、測試副本與匹配程序無殘留。authentication 維持 warn／unknown，未要求登入、模型或服務操作；真 canary／切換與 live-auth producer 仍待完成（PR #158，未合併）。

- 該輪證據：`AgEnD-ops/g13-install-20261008/backend-native-version-check-result.json`。Claude 舊版原路徑已不存在，從官方固定版本下載並核與歷史驗收相同 SHA-256；專用原始 binary 暫保留供待執行 canary，未改共用安裝。

- 2026-10-08：doctor 分開執行檔完整性、歷史 canary 範圍與 unknown 登入；有效歷史收據不再提示重跑 canary，明示尚未驗證目前 daemon／登入／其他能力。正式假 backend producer 收據與錯 build 回歸 7/7、install_home 9/9、CLI init_and_doctor、workspace clippy、fmt、check-deps 通過，獨立唯讀覆核無 blocker；自有測試目錄／程序未見殘留。此改動未補足 live-auth producer 或整關真環境驗收（feat/g13-install／PR #158）。

- 2026-10-08：13A 接入 operator-only BackendDiagnostic 1.9；同一 SQLite transaction 讀配置與匹配觀測／受管預約摘要，排除 args／session／環境。doctor 一次性 RPC 核 fleet 的 daemon boot 與配置，跨 boot 或 scope 不符回 unknown；外部樣本不冒充執行映像、預約不冒充存活。原生三次啟動 RPC／doctor、1.8 與 agent 拒絕、store 4 項、managed 2 項、boot／scope 反例、install_home 9 項與 CLI 回歸通過；client 全套 52 項通過，workspace clippy／fmt／check-deps 通過。獨立覆核指出的通用 client 重連問題已改為 exchange_once 並複核；重啟競態仍是組合證據，非單一中途重啟案例。自有測試程序／目錄未見殘留，登入 producer／能力矩陣與真環境驗收仍待完成（feat/g13-install／PR #158）。

- 2026-10-08：13A doctor 增加四條 daemon 能力政策：Codex 人工輸入、Claude 完整啟動畫面、OpenCode endpoint 與獨立 permission gate。讀實際 policy／共用常數，不放寬准入；verification override 限定 instance，boot／配置失配不展示政策，runtime eligibility 仍 unknown。core 149 通過／2 既有 ignored、三 backend 原生 canary 7 項、OpenCode driver 20 項、原生三次重啟與診斷、權限／scope 回歸、workspace clippy／fmt／check-deps 通過；獨立唯讀覆核無 blocker。本批自有程序／測試目錄未見殘留，移除重複 build log；真認證／能力驗收仍待完成（feat/g13-install／PR #158）。

- 2026-10-08：`40df658` 補雙 monitor 正式 daemon 停機驗收；兩種完成順序通過，兩個丟棄 worker handle 的 mutation 均被抓出。daemon lib 223 通過／2 子程序入口 ignored、workspace clippy／fmt／check-deps 通過；自有程序與暫存不存在。上述程式已包含於 f08fe43 的完整自動驗收與雙平台 CI；後續提交仍需自己的驗證。


- 2026-10-08：7907e7e 的 accept install 在 daemon version probe 輸出上限斷言失敗（222 通過、1 失敗、2 ignored），當時斷言未印實際錯誤。只補錯誤診斷，未改期限或 production；daemon lib 重跑 223 通過／2 ignored，一次 8 並行、32 案例探測未重現，精確核自有目錄與程序無殘留。根因仍未知，不把重跑視為修復；該次整關驗收維持失敗（PR #158）。

## 授權與清理邊界

- 可繼續：feature branch 實作、隔離測試、push／draft PR／CI。
- 已取得受控真測授權；nonce launchd capture 與專用 Telegram bot 流程已執行。三 backend 真測仍缺專用認證檔案路徑，不使用共享帳戶資料替代。
- 實際使用者常駐服務變更、公開發布與本關 merge，仍需備妥具體結果後取得必要核准。
- 保留 `~/.claude.json` trust entries、外來程序／worktree、AlphaCR runs。
- 完成每批後清自有臨時程序與目錄；未合併 Gate 13 worktree 與 target 保留供驗收，合併後再清。

## 下一步

63b083b 的獨立自動驗收、雙平台 CI 與四平台打包已取得；macOS 原生服務生命周期亦已通過；接著補登入有效性、三 backend 真測及 doctor 故障修復矩陣，不因文件同步重跑已通過的程式測試。需要使用者操作時，提供固定版本、命令、預算、影響範圍與清理方式，一次帶一個步驟。

- 2026-10-10：正式 `backend import`／`backend canary --allow-model` 對 OpenCode 1.18.34、`opencode-go/gpt-6-luna` 完成三則真模型訊息，三個 confirmed receipt／三個 outcome，33,886 ms、passed 與 cleanup_complete 均 true。使用試用環境已隔離的單 provider API 認證，來源雜湊不變；自有 canary home、程序及匯入副本清理核對通過，使用者 trial 保留。證據 `AgEnD-ops/g13-install-20261008/opencode-managed-canary-20261010/{plan,report,cleanup}.json`；build SHA e3a159222c8ba8353bc6a667bdd5221c7778cb5fe2e61b45584db9457aa5ef2b。這不證明受管切換／回退、Claude／Codex canary 或整關完成。
