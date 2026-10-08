# 第 13 施工關：目前證據與完成缺口

> **TL;DR**
> - 第 13 關仍未完成、未核准合併；PR #158 是 draft。
> - 功能提交、局部測試、整關驗收與真環境驗收分別記錄，不能互相替代。
> - 下一步：處理 accept install 與 CI 的失敗，完成以下缺口，再交付 final fresh verifier 與逐步人工驗收。

此頁整理 2026-10-08 的當前狀態；歷史過程保留在 [施工關紀錄](gate-13-install.md)。

| 範圍 | 已實作／已有證據 | 尚未證明完成 |
|---|---|---|
| 13A home／設定 | 預設 home、init 權限、doctor 的配置程式解析、沙箱與服務診斷；install_home 回歸 | backend 登入有效性與完整版本相容矩陣；doctor 故障→修復人工矩陣 |
| 13B 服務 | launchd／systemd plan、安裝所有權、對帳、解除安裝、資料刪除互斥；Linux 隔離服務紀錄 | macOS 真 launchd capture 尚待核准；兩平台服務重啟後 holder 存活、解除安裝清理的最終版本證據 |
| 13C 受管版本 | 匯入、canary 收據、版本切換／回退、身份綁定與 pending 恢復；原生假 backend 測試 | 三個 backend 的專用認證／真模型 canary、切換與回退完整驗收及所有故障切點 |
| 13C 版本發現 | 固定 npm 公開 metadata、每日持久排程、外部 CLI 磁碟版本探測、待辦與 Telegram 精確確認 | 真實系統 CLI 的版本觀測驗收；正式 daemon 同時兩種 monitor active 的停機整合證據 |
| 13C 重啟通知 | 真 daemon 三次啟動測試：worker 發現換版、通知重建、操作員確認持久化，agent 與錯誤 action 拒絕 | 此測試使用本機假 CLI，不能代替真模型或 service-manager 重啟驗收 |
| 13D Telegram | daemon 配對、token reference、allowlist、設定套用；loopback HTTP producer 測試 | 專用 bot 的 /start→confirm→apply→重啟→通知／操作真驗收 |
| 13E 打包 | 歷史 release-artifacts 37753100820 在 3f52828 通過四平台 archive／首任務與雙平台 Brew 驗證 | 最終提交的打包驗證、全新使用者真 backend 首任務、版本／發布交付計畫；尚未公開發布 |
| 整關 | 持續執行局部原生測試與獨立 source review | 當前提交的 accept install、完整雙平台 CI、最終 fresh-context verifier、使用者逐步驗收與合併確認 |

## 當前自動驗收

- `cargo xtask accept install` 在 `21ecf59` 啟動，已以 exit 1 結束：`backend_canary` 16 通過、1 失敗，Codex 假 CLI 的版本探測逾時；尚未到 install demo。執行期間另提交 fake-worker 修正，故本輪也不是固定 head 的全套證據。
- `9a8bcf7` 的 macOS CI 在 pipeline 有兩項失敗（WIP 未封存、預期 rework 卻首次完成）。`e64e629` 修正 fake-worker 的版本查詢誤入 inbox；獨立 source review 確認此路徑與兩項症狀相符。修正後已重建 fake-worker，固定程式 head `e64e629` 的完整 pipeline 15 項重驗通過；Ubuntu 同一輪 CI 的 GitHub pipeline WIP 案例亦失敗；本機修正後 GitHub pipeline 全 5 項通過（25.64 秒，真 daemon／holder、離線 GitHub producer）。新 head CI 尚待完成。
- Codex canary 原失敗案例在 `e64e629` 單獨執行通過（14.81 秒，版本探測仍限 5 秒）。這只證明單例可通過，尚未確認原逾時根因；原預設並行方式的 backend_canary 17 項也通過（77.83 秒），仍不把重跑成功當作問題已修好。補逾時的 elapsed_ms、budget_ms、stdout_bytes 診斷，不保存輸出內容、不增加重試或延長期限。
- `backend_version_monitor` 已納入 install demo；它不註冊主機服務、不啟動真 backend、不讀真認證。
- 新測試若在 CI 之前提交，舊 head 的綠燈不能代表新 head 通過；仍在跑的 CI 不因觀察逾時重啟。
- 最終 fresh verifier 必須重跑並嘗試推翻；目前逐批 read-only source review 不能冒充最終驗證。

## 授權與清理邊界

- 可繼續：feature branch 實作、隔離測試、push／draft PR／CI。
- 待具體核准：實際使用者常駐服務變更、公開發布、三 backend 真模型與專用認證計畫，以及本關 merge。
- 已準備的 nonce launchd capture 計畫尚未取得回答；不能把 goal 自動續行當成批准。
- 保留 `~/.claude.json` trust entries、外來程序／worktree、AlphaCR runs。
- 完成每批後清自有臨時程序與目錄；未合併 Gate 13 worktree 與 target 保留供驗收，合併後再清。

## 下一步

先取得當前自動驗收結果；其間補可隔離完成的缺口。需要使用者操作時，提供固定版本、命令、預算、影響範圍與清理方式，一次帶一個步驟。
