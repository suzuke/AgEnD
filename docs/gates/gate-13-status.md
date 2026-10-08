# 第 13 施工關：目前證據與完成缺口

> **TL;DR**
> - 第 13 關仍未完成、未核准合併；PR #158 是 draft。
> - 功能提交、局部測試、整關驗收與真環境驗收分別記錄，不能互相替代。
> - 下一步：以已通過的自動驗收為基線，完成以下缺口，再交付 final fresh verifier 與逐步人工驗收。

此頁整理 2026-10-08 的當前狀態；歷史過程保留在 [施工關紀錄](gate-13-install.md)。

| 範圍 | 已實作／已有證據 | 尚未證明完成 |
|---|---|---|
| 13A home／設定 | 預設 home、init 權限、doctor 的配置程式解析、沙箱與服務診斷；install_home 回歸 | doctor 已分開執行檔／歷史 canary 範圍／unknown 登入；daemon 唯讀配置／觀測快照已接入並核 boot；四條版本敏感能力政策已列出；登入有效性、能力真環境證據與故障→修復人工矩陣仍待完成 |
| 13B 服務 | launchd／systemd plan、安裝所有權、對帳、解除安裝、資料刪除互斥；d78f530 的 Linux ARM64 正式 archive 在隔離 systemd／一般使用者下通過五階段生命周期 | macOS 真 launchd capture 尚待核准；macOS daemon／holder 生命周期、登入登出與最終版本驗收 |
| 13C 受管版本 | 匯入、canary 收據、版本切換／回退、身份綁定與 pending 恢復；原生假 backend 測試 | 三個 backend 的專用認證／真模型 canary、切換與回退完整驗收及所有故障切點 |
| 13C 版本發現 | 固定 npm 公開 metadata、每日持久排程、外部 CLI 磁碟版本探測、待辦與 Telegram 精確確認 | 真實系統 CLI 的版本觀測驗收；雙 monitor 同時 active 的正式 daemon 停機兩種完成順序已通過 |
| 13C 重啟通知 | 真 daemon 三次啟動測試：worker 發現換版、通知重建、操作員確認持久化，agent 與錯誤 action 拒絕 | 此測試使用本機假 CLI，不能代替真模型或 service-manager 重啟驗收 |
| 13D Telegram | daemon 配對、token reference、allowlist、設定套用；loopback HTTP producer 測試 | 專用 bot 的 /start→confirm→apply→重啟→通知／操作真驗收 |
| 13E 打包 | d78f530 的 release-artifacts 37783051296 通過四平台 archive、formula 與雙平台 Brew 七個 jobs | 最終提交的打包驗證、全新使用者真 backend 首任務、版本／發布交付計畫；尚未公開發布 |
| 整關 | 持續執行局部原生測試與獨立 source review | 當前提交的 accept install、完整雙平台 CI、最終 fresh-context verifier、使用者逐步驗收與合併確認 |

doctor 逐項覆蓋與缺口見 [故障／修復矩陣](gate-13-doctor-matrix.md)。

## 當前自動驗收

- 固定程式提交 `f08fe43` 的 `cargo xtask accept install` 以 exit 0 完成，耗時 2,200.61 秒；起訖 HEAD 相同且工作樹乾淨。包含 core／daemon／CLI checks、安裝 demo、三種原生假 backend canary、服務 lifecycle 模型與配對隔離測試。
- 該版全新 HOME 首任務 `t-1` 完成，恰好一次 merge，耗時 5,597 ms；使用 fake workers，0 模型呼叫。自有 HOME／repo 已清理。
- [CI 37778088087](https://github.com/suzuke/AgEnD/actions/runs/37778088087) 的 `f08fe43` 在 macOS／Ubuntu 成功；不能把結果套用到後續修改。
- `d78f530` 的 [release-artifacts 37783051296](https://github.com/suzuke/AgEnD/actions/runs/37783051296) 七個 jobs 全部成功。正式 Linux ARM64 archive（binary SHA-256 `caaeee85b06c830c6d70822c1edf965981fd295d0ab90a5dd9967709ae32873f`）以原生 busctl、非 root 使用者驗安裝、重啟接回相同 holder／agent 世代、解除安裝保留資料與外來 drop-in、拒絕外來 MainPID、完整清理。隔離 container／image／暫存已刪除；未碰主機服務、未用真 backend，未驗登入登出。成功案例未記錄 readiness false→true，該分支另由捕獲的原生 fixture 與前置實驗支持。
- `d78f530` 的 macOS 26 PR CI 在 self-setsid fixture 準備階段收到 EPERM。`64c35eb` 僅為 fixture 加入限定 EPERM／父程序群組的有界重試，必須建立真 session，清理前仍存活且最終由 SIGKILL 終止；production 期限與清理邏輯未改。本機 binary 40 項、fmt／clippy／check-deps 通過，新 CI 尚待結果。
- 歷史 `9a8bcf7` CI 的 pipeline／GitHub pipeline WIP 與返工失敗，經 `e64e629` 修正 fake-worker --version 誤入 inbox 後，本機完整 pipeline 15/15、GitHub pipeline 5/5 與上述雙平台 CI 通過。
- 前輪 accept install 的 Codex 假 CLI 版本 probe 曾逾時；單例、並行全組及本輪 accept／demo 均通過，但原偶發逾時未穩定重現。`f762f47` 只增加 elapsed_ms／budget_ms／stdout_bytes，不記原始輸出、不重試、不延長期限。
- 上述不代表真 backend、真 Telegram 或主機 service-manager 驗收；最終 fresh-context verifier 必須重跑並嘗試推翻，目前逐批 read-only review 不能代替。

## 本批進度

- 2026-10-08：doctor 分開執行檔完整性、歷史 canary 範圍與 unknown 登入；有效歷史收據不再提示重跑 canary，明示尚未驗證目前 daemon／登入／其他能力。正式假 backend producer 收據與錯 build 回歸 7/7、install_home 9/9、CLI init_and_doctor、workspace clippy、fmt、check-deps 通過，獨立唯讀覆核無 blocker；自有測試目錄／程序未見殘留。此改動未補足 live-auth producer 或整關真環境驗收（feat/g13-install／PR #158）。

- 2026-10-08：13A 接入 operator-only BackendDiagnostic 1.9；同一 SQLite transaction 讀配置與匹配觀測／受管預約摘要，排除 args／session／環境。doctor 一次性 RPC 核 fleet 的 daemon boot 與配置，跨 boot 或 scope 不符回 unknown；外部樣本不冒充執行映像、預約不冒充存活。原生三次啟動 RPC／doctor、1.8 與 agent 拒絕、store 4 項、managed 2 項、boot／scope 反例、install_home 9 項與 CLI 回歸通過；client 全套 52 項通過，workspace clippy／fmt／check-deps 通過。獨立覆核指出的通用 client 重連問題已改為 exchange_once 並複核；重啟競態仍是組合證據，非單一中途重啟案例。自有測試程序／目錄未見殘留，登入 producer／能力矩陣與真環境驗收仍待完成（feat/g13-install／PR #158）。

- 2026-10-08：13A doctor 增加四條 daemon 能力政策：Codex 人工輸入、Claude 完整啟動畫面、OpenCode endpoint 與獨立 permission gate。讀實際 policy／共用常數，不放寬准入；verification override 限定 instance，boot／配置失配不展示政策，runtime eligibility 仍 unknown。core 149 通過／2 既有 ignored、三 backend 原生 canary 7 項、OpenCode driver 20 項、原生三次重啟與診斷、權限／scope 回歸、workspace clippy／fmt／check-deps 通過；獨立唯讀覆核無 blocker。本批自有程序／測試目錄未見殘留，移除重複 build log；真認證／能力驗收仍待完成（feat/g13-install／PR #158）。

- 2026-10-08：`40df658` 補雙 monitor 正式 daemon 停機驗收；兩種完成順序通過，兩個丟棄 worker handle 的 mutation 均被抓出。daemon lib 223 通過／2 子程序入口 ignored、workspace clippy／fmt／check-deps 通過；自有程序與暫存不存在。上述程式已包含於 f08fe43 的完整自動驗收與雙平台 CI；後續提交仍需自己的驗證。

## 授權與清理邊界

- 可繼續：feature branch 實作、隔離測試、push／draft PR／CI。
- 待具體核准：實際使用者常駐服務變更、公開發布、三 backend 真模型與專用認證計畫，以及本關 merge。
- 已準備的 nonce launchd capture 計畫尚未取得回答；不能把 goal 自動續行當成批准。
- 保留 `~/.claude.json` trust entries、外來程序／worktree、AlphaCR runs。
- 完成每批後清自有臨時程序與目錄；未合併 Gate 13 worktree 與 target 保留供驗收，合併後再清。

## 下一步

自動驗收基線已取得；接著在固定提交重跑整套自動驗收與四平台打包，補登入有效性與真環境驗收。需要使用者操作時，提供固定版本、命令、預算、影響範圍與清理方式，一次帶一個步驟。
