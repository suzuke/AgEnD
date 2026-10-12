# 第 13 關 doctor 故障與修復矩陣

> **TL;DR**
> - 以下是原始碼與既有證據的覆蓋盤點，不是整關驗收通過表。
> - Codex 帳戶設定、OpenCode session／供應者歷史與 GitHub 工作流程診斷已接原生 producer；unknown 不算通過。
> - 真服務、真 backend、操作員人工驗收須各自取得證據。

2026-10-12 更新：本機修復回歸及唯讀 runtime 診斷。執行紀錄及固定版本見 [目前狀態](gate-13-status.md)。

| doctor 項目 | 故障／恢復路徑 | 現有驗證與缺口 |
|---|---|---|
| home | 不存在→init；0500／0755→0700；v1／非法位置→另選 home | `install_home` 與 `cli::init_and_doctor`；一般使用者實際執行 doctor 的 chmod 提示，包含 UTF-8 空白／單引號／shell 特殊字元路徑，修復後 ok、配置不變 |
| daemon | 未啟動→啟動；版本不合→更新並重連 | CLI fixture 驗不可達與連線成功；真 service restart 證據分平台，不能以 hello 成功代替整個服務功能 |
| git | 舊於 2.38→支援版本 | `install_home` 補 2.37.9 fail→2.38.0 ok，`cli::init_and_doctor` 以原生 fixture 執行檔產生版本回覆；並非真的安裝／降級操作者 Git |
| claude／codex／opencode | PATH 缺失或版本命令失敗→恢復 executable | `install_home`；僅證明版本 probe，不證明登入或 driver 相容 |
| backend/&lt;instance&gt; | 配置程式缺失／受管副本遭改→恢復正確程式 | `install_home` 核設定程式與變更拒絕；受管歷史 canary 與當前 executable 完整性分開報告 |
| observation/&lt;instance&gt; | boot／配置不符→重新取得匹配快照 | `doctor/observations.rs` 反例與 backend observation 原生整合；匹配快照仍不證明 running image／登入 |
| policy／capability | 分開政策與目前 driver 條件 | policy 僅表示讀取到規則；capability 核 Codex 連線及人工輸入 gate、OpenCode endpoint 健康／版本／session 及 permission 版本 gate。前後核 instance／holder／配置，失配回 unknown |
| authentication／provider | 缺少帳戶設定→恢復設定；供應者拒絕→同 session 後續完成 | Codex 原生 account/read 固定 refreshToken=false；doctor_native 經真 daemon＋假 app-server 驗 missing→configured、0 登入、無真模型。Configured 不等於 token 有效。OpenCode 讀最近 64 筆原生歷史，按 provider／model 留最新完成結果；401／403／429／成功分開，無結果維持 unknown，舊成功不證明目前認證有效 |
| github | 使用 GitHub forge 且 auth status 失敗→恢復 | 只檢查實際 GitHub workflow；local-only 不執行 gh。daemon 的原生 gh auth status --active --hostname github.com 有界執行，fixture 驗 fail→ok／未知 exit。失敗可能是認證或連線，不輸出原始帳戶資料 |
| holders | 孤兒 holder→daemon boot sweep | `cli::doctor_orphan_warning_recovers_by_starting_the_real_daemon`：離線真 holder→doctor 提示啟動 daemon→正式 boot sweep Shutdown→子程序退出、lock 釋放→doctor ok。另有 FakeDaemon 孤兒提示案例；離線修復不冒充在線 restart 驗收 |
| disk | 可用空間小於 1 GB→釋放空間；home 大於 20 GB→清自有資料 | 原生小 volume 留存 fail→ok 證據；`install_home` 用 sparse fixture 驗大 home warn→執行引用路徑的 du 提示→移除自有 fixture→ok，不填滿主機磁碟 |
| sandbox | 缺工具→恢復 sandbox 工具 | `install_home` 先指定不存在工具，再以平台原生 sandbox readiness 恢復；不移除主機工具 |
| telegram | 非法設定／空 allowlist／token 權限不符→修正專用設定 | `install_home` 及 doctor 原生設定測試；僅本機設定，未向 Telegram 發送。真配對與通知另驗 |
| service | 定義缺失／遭改／manager 停止→修復自有安裝 | `service::tests::diagnostic_observes_recovery_without_mutating_the_installation_or_manager` 為 lifecycle 模型；Linux 正式 archive 生命周期與 macOS 固定 debug binary 及正式 ARM64 archive／C 等待程序的原生 launchd 生命周期均通過；服務成功不代替每一個 doctor 修復提示的人工驗收 |

## 人工驗收邊界

不在日常 HOME 製造故障、不降級共用 Git、不停外來 holder、不變更共用認證。各項測試須先提供專用 home、固定 binary、預算及清理範圍。正式 service 安裝與真模型執行另按已列計畫取得授權。

`cargo xtask accept install` 的成功不能自動勾選本表所有真環境項目，也不代表使用者已照 doctor 的修復提示操作過。

## 下一步

自動修復回歸與 runtime 診斷納入最終固定提交驗收；使用者逐步驗收仍獨立記錄。Claude 真測依使用者指示暫緩。不可寫 home 測試須一般使用者；非 UTF-8 目錄修復已補原始 bytes 編碼與回歸；Linux 檔案系統案例等待本批 CI。Linux ARM64 低磁碟已以固定 797a7b9 正式 archive 驗同一 home 的 tmpfs 64 MiB→2 GiB，disk fail→ok、設定不變；只證明未改動的 disk／statvfs 路徑，不代表全部 doctor 檢查變成 ok。
