# 第 13 關 doctor 故障與修復矩陣

> **TL;DR**
> - 以下是原始碼與既有證據的覆蓋盤點，不是整關驗收通過表。
> - 登入有效性與即時能力仍缺 producer；unknown 不能算修復成功。
> - 真服務、真 backend、操作員人工驗收須各自取得證據。

盤點基線：`3b0bb89`。執行紀錄及固定版本見 [目前狀態](gate-13-status.md)。

| doctor 項目 | 故障／恢復路徑 | 現有驗證與缺口 |
|---|---|---|
| home | 不存在→init；0755→0700；v1／非法位置→另選 home | `install_home` 與 `cli::init_and_doctor`；CLI 不覆寫既有設定 |
| daemon | 未啟動→啟動；版本不合→更新並重連 | CLI fixture 驗不可達與連線成功；真 service restart 證據分平台，不能以 hello 成功代替整個服務功能 |
| git | 舊於 2.38→支援版本 | `cli::init_and_doctor` 以原生 fixture 執行檔產生版本回覆；並非真的安裝／降級操作者 Git |
| claude／codex／opencode | PATH 缺失或版本命令失敗→恢復 executable | `install_home`；僅證明版本 probe，不證明登入或 driver 相容 |
| backend/&lt;instance&gt; | 配置程式缺失／受管副本遭改→恢復正確程式 | `install_home` 核設定程式與變更拒絕；受管歷史 canary 與當前 executable 完整性分開報告 |
| observation/&lt;instance&gt; | boot／配置不符→重新取得匹配快照 | `doctor/observations.rs` 反例與 backend observation 原生整合；匹配快照仍不證明 running image／登入 |
| capability/&lt;instance&gt;/&lt;capability&gt; | 補齊各能力的版本及執行時條件 | 目前只列 daemon 政策、eligibility unknown；尚無真環境「壞→修復」完成證據 |
| authentication | 缺失／失效認證→專用帳戶重新登入 | 目前固定 warn／unknown；缺 authoritative live producer，尚未完成。不得用 Ready 或歷史 canary 改標登入有效 |
| holders | 孤兒 holder→daemon boot sweep | CLI 真 holder＋FakeDaemon 驗 orphan warning；該案例由測試主動 Shutdown，不是照 fix 重啟 daemon 的修復證明。boot sweep 原生測試須在最終矩陣另行連結 |
| disk | 可用空間小於 1 GB→釋放空間；home 大於 20 GB→清自有資料 | 原生小 volume 留存 fail→ok 證據；`install_home` 用 sparse fixture 驗大 home warn→ok，不填滿主機磁碟 |
| sandbox | 缺工具→恢復 sandbox 工具 | `install_home` 先指定不存在工具，再以平台原生 sandbox readiness 恢復；不移除主機工具 |
| telegram | 非法設定／空 allowlist／token 權限不符→修正專用設定 | `install_home` 及 doctor 原生設定測試；僅本機設定，未向 Telegram 發送。真配對與通知另驗 |
| service | 定義缺失／遭改／manager 停止→修復自有安裝 | `service::tests::diagnostic_observes_recovery_without_mutating_the_installation_or_manager` 為 lifecycle 模型；Linux 正式 archive 生命周期另有真測，macOS 待驗 |

## 人工驗收邊界

不在日常 HOME 製造故障、不降級共用 Git、不停外來 holder、不變更共用認證。各項測試須先提供專用 home、固定 binary、預算及清理範圍。正式 service 安裝與真模型執行另按已列計畫取得授權。

`cargo xtask accept install` 的成功不能自動勾選本表所有真環境項目，也不代表使用者已照 doctor 的修復提示操作過。

## 下一步

先補登入 producer／真能力證據及 holders 的建議修復路徑，再將固定提交的自動測試與逐步人工結果逐項綁定。本表目前沒有將任何缺口降為非必要。
