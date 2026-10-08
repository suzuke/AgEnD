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

- 固定程式提交 `8507a05` 的 `cargo xtask accept install` 已以 exit 0 完成；期間工作樹保持乾淨。包含 core／daemon／CLI checks、安裝 demo、三種原生假 backend canary、服務 lifecycle 模型與配對隔離測試。
- 全新 HOME 的首任務 `t-1` 完成，恰好一次 merge，耗時 5,994 ms；使用 fake workers，0 模型呼叫。該 HOME／repo 已不存在，檢查未見本批 target 的 daemon／fake backend 程序或相關 g13 test roots 殘留。
- [CI 37767137610](https://github.com/suzuke/AgEnD/actions/runs/37767137610) 的 head 精確為 `8507a05`，macOS 與 Ubuntu 兩個 job 均 success。不能把此結果套用到後續程式修改。
- 歷史 `9a8bcf7` CI 的 pipeline／GitHub pipeline WIP 與返工失敗，經 `e64e629` 修正 fake-worker --version 誤入 inbox 後，本機完整 pipeline 15/15、GitHub pipeline 5/5 與上述雙平台 CI 通過。
- 前輪 accept install 的 Codex 假 CLI 版本 probe 曾逾時；單例、並行全組及本輪 accept／demo 均通過，但原偶發逾時未穩定重現。`f762f47` 只增加 elapsed_ms／budget_ms／stdout_bytes，不記原始輸出、不重試、不延長期限。
- 上述不代表真 backend、真 Telegram 或主機 service-manager 驗收；最終 fresh-context verifier 必須重跑並嘗試推翻，目前逐批 read-only review 不能代替。

## 授權與清理邊界

- 可繼續：feature branch 實作、隔離測試、push／draft PR／CI。
- 待具體核准：實際使用者常駐服務變更、公開發布、三 backend 真模型與專用認證計畫，以及本關 merge。
- 已準備的 nonce launchd capture 計畫尚未取得回答；不能把 goal 自動續行當成批准。
- 保留 `~/.claude.json` trust entries、外來程序／worktree、AlphaCR runs。
- 完成每批後清自有臨時程序與目錄；未合併 Gate 13 worktree 與 target 保留供驗收，合併後再清。

## 下一步

自動驗收基線已取得；接著補登入／相容性診斷與其餘隔離驗證缺口。需要使用者操作時，提供固定版本、命令、預算、影響範圍與清理方式，一次帶一個步驟。
