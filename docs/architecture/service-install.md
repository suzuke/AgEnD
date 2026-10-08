# 使用者服務與解除安裝（第 13B，施工中）

> **TL;DR**
> - `service plan` 唯讀；`service install` 安裝 user launchd／systemd 服務。
> - `uninstall` 預設保留資料；刪資料必須另加旗標並確認完整 home 路徑。
> - Linux 生命週期已有隔離真測；macOS 真服務與整關驗收仍未完成。

## 操作

| 指令 | 行為 |
|---|---|
| `agend service plan --json` | 顯示定義、受管 executable 與 service 路徑，不寫檔 |
| `agend service install --no-start` | 發布定義、executable 與 installation receipt，不註冊服務 |
| `agend service install` | 對帳自有資源後註冊服務；再用 doctor 確認 readiness |
| `agend service status` | 核對 installation receipt、檔案與服務管理器 |
| `agend uninstall` | 停止自有服務／holders，移除自有定義、executable 與 shim，保留資料 |
| `agend uninstall --delete-data` | 尚未提供路徑確認時拒絕操作，印出需要確認的 canonical home |
| `agend uninstall --delete-data --confirm-home /完整/home` | 路徑逐字符合才可刪除該 home 中的設定、DB、workspace、archive 與其他資料；無法復原 |

資料刪除必須在自有 installation receipt 仍存在時一起執行。已完成預設解除安裝、沒有 receipt 的目錄不會被當成自有安裝刪除。agent 身分不可使用安裝／解除安裝操作。

## 所有權與並行

- 收據先持久化，再發布檔案／註冊；中斷後以同一收據對帳，不收編未知或被修改的 service／executable。
- systemd 核對 D-Bus 實際有效設定與 live MainPID 身分。停止時不隱含 reload 未檢查的 drop-in。
- macOS 執行中 PID 加核映射 executable inode、精確 argv／home、UID 與開始時間；已用自有 native 程序驗同路徑替換拒絕。loaded launchd 定義解析與真 Rust daemon 尚待原生驗收。
- daemon 重新啟動保留 holder；解除安裝才停止通過 home／lock／socket 檢查的自有 holder。
- 移除階段持有 installation lock、home maintenance flock 與既有 DB 的 SQLite 排他鎖。當前版本的 daemon 在建立／開啟 DB 前必須取得共享 home lock。
- 明確刪資料仍留下 home、`service/` 及兩個空鎖檔，維持相同 inode，避免等待者取得兩套鎖；不留下設定、工作資料或 receipt。
- 已在執行的舊版 DB owner 會被 SQLite 鎖拒絕。未實作 home flock 的舊版程式若在 DB unlink 後另行啟動，不在並行排他保證內；清理期間不可另啟動舊 binary。

## 刪除前的拒絕條件

- 路徑確認不符、缺少自有收據、service／executable 被改動、仍有 DB owner。
- home 內還有 `.git`：先完成／取消任務，或把要保留的 repository 移出去，避免留下外部 worktree 登記與遺失工作。
- 外來 UID、可由他人寫入的目錄、跨掛載點或過深的目錄。Linux 比對開啟目錄的 mount ID，包含同一磁碟的 bind mount；macOS 比對 `fstatfs` mountpoint。
- symlink 只刪 link，不走訪其目標。此防護不宣稱能對抗清理同時由另一個特權程序變更掛載或惡意替換路徑。

刪除不是跨整個目錄的原子交易；I/O 失敗可能留下部分資料，但 receipt 保留，可用同一明確確認重試。開始前自行備份要保留的資料。

## 下一步

完整驗收狀態與人工步驟見 [第 13 施工關](../gates/gate-13-install.md)。
