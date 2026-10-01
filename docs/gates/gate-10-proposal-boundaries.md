# Gate 10 提案：決定、範圍與開工風險

> **TL;DR**
> - 此頁保存開工時的已決定事項、範圍與風險；當時的前置施工關狀態屬歷史紀錄。
> - 狀態與驗證證據見 [Gate 10 入口](gate-10-pipeline.md)。
> - 下一步：回 [Gate 10 入口](gate-10-pipeline.md) 看目前狀態與自動驗收。

### 已決定（原本的待你決定）

- 操作者取消 task：要，正式命令 `agend task cancel <task>`（使用者 2026-09-26 決定）；語法歸第 9 施工關，daemon 端 `task_cancel` 在本關。驗收用 `agend task cancel`；只有開工時第 9 施工關還沒做好，才暫時用 `pipeline_probe cancel`。操作者開 task：可以（P10）。
- 與決策或架構頁不同、使用者 2026-09-26 都決定照本頁做的：D18（P8 `no-role`）、D33 第 3 點（P3 拒絕刪除持有者）、pipeline.md 不在使用者目錄 `git merge`（P7 的 `--ff-only`）、delivery.md 推送為主（P11 的 `delivery = inbox`）、pipeline.md 不從 git 推論 done（P7 的手動 merge 記成完成）。

### 本關不做（明確列出）

- fanout／`epic`（子 task）、`depends_on`、supersede、reopen：用到的 workflow 或操作在本關回錯誤並指出還不支援。
- 改派（持有者被刪、額度用盡、逾時動作 `reassign`）、臨時 instance、角色範本（P3）。
- forge github、GitHub CI（第 12 施工關）。
- 卡住偵測、usage limit、依緊急程度自動選忙碌等級（第 7 施工關 P9 說要移來本關：本關一律用 `Queue`，其餘建議移到第 12 施工關，有真 backend 資料時再做）。
- codex approval 轉給人回答（第 7 施工關 P9 說移到第 10、11 施工關：建議整個給第 11 施工關）。
- 派工時的檔案衝突警告（`policy::conflict`）：正確性由 P7 的 merge 前 rebase 保證，警告之後再加。
- `agend workflow` 的 `new --from`、`edit`、`history`、`rollback`、`delete`（P10）。
- 逾時動作「通知」只記 log 與 task 事件；Telegram 通知在第 12 施工關。
- checks 的 IP 網路限制（沙箱只管寫入與 unix socket，P6）；需要 unix socket 的 checks（docker、ssh-agent、gpg-agent）；Windows（沒有沙箱工具，照 fail closed 不跑 checks）。

### 已知風險（開工時處理）

- 依賴三個還沒完成的施工關。第 7 施工關的 `messages` 表或第 9 施工關的 ticket／`inbox --after`／`operator` 請求跟這裡的理解不同時，改這頁的「你親自驗收」，不改它們。
- **`sandbox-exec` 被 Apple 標為 deprecated**（P6）：目前的 macOS 仍能用，未來版本可能拿掉；拿掉時照 fail closed，checks 不跑、出現 `sandbox-missing`，要另找工具。
- **`bwrap` 需要 unprivileged user namespace**：Ubuntu 24.04 起 AppArmor 預設限制它，CI 的 `ubuntu-latest` 也是。CI 先 `sudo apt-get install -y bubblewrap`（runner 預設沒有），再試；不行就在 CI 加一步放寬（`sysctl kernel.apparmor_restrict_unprivileged_userns=0`）或給 `bwrap` AppArmor profile。沙箱測試**不能**因為工具不能用就 SKIP 當通過。
- 沙箱裡 home 唯讀：會寫 `~` 的工具（例如 rustup 的鎖檔、某些語言的全域快取）可能失敗；開工時用 `cargo test` 與 `npm test` 各試一次，缺的路徑改用 `XDG_CACHE_HOME`（在每次的暫存目錄裡）。macOS 上不帶 `-t` 的 `mktemp` 會失敗（見 P6），測試腳本要改用 `mktemp -t`。
- checks 很慢（P6）：沒有共用快取，每次都重新下載依賴、從頭編譯。這是為了不讓一次 check 讓下一次假綠（跟 P6 拒絕重複用 worktree 是同一個理由）。之後要加速，只能加「沙箱外的受信任步驟準備、沙箱裡唯讀」的快取，不能再開放可寫的共用快取。
- 沒有 `AGEND_INSTANCE` 的程式連上 daemon socket 就被當成操作者（第 8 施工關 P2、D6 已接受的限制）：agent 寫的任何程式碼都一樣。所以 checks 不能連任何 unix socket（P6）；agent 自己在它的 holder 裡跑的程式有 `AGEND_INSTANCE`，不受這條影響，但故意清掉環境變數的仍能假冒，同 D6。
- macOS 沒有 pid namespace：check 用 `setsid` 另開 process group 的子程序，停掉 process group 時停不到，會在 check 之後繼續跑；它仍在沙箱裡，寫不了外面、連不了任何 unix socket（只剩 DNS），但能用 IP 網路。開工時量，必要時改成追蹤沙箱裡的所有子程序。
- **Linux 只藏了三個放 socket 的目錄**（P6）：`/tmp`、`/run/user/<uid>`、`$AGEND_HOME/run`。其他在 `--ro-bind / /` 下看得到的 socket（`/var/run/docker.sock`、`/run/systemd/…`、abstract namespace）check 仍連得上；docker socket 等於 root，機器上有它時要知道這一點。要更嚴只能用 seccomp 擋 `AF_UNIX` 的 `connect`，本關不做。
- `-c core.hooksPath=/dev/null` 也跳過專案自己的 `post-checkout` 等 hook（P5）：依賴 hook 準備環境的專案，checks worktree 裡少了那一步；要的話寫進 checks 指令。
- 在持有者的 worktree 裡 rebase（P7）：這時 task 不在 work 關卡，但 agent 仍可能剛好在跑 git。worktree 不乾淨就當成衝突退回 work，不硬做。
- trailer 搜尋最多看 main 的 1000 個 first-parent commit（P7）：merge 之後 main 又前進超過 1000 個 commit、而且 DB 同時丟了 `merge_intent`，才會找不到；開工時量搜尋的耗時。
- core 要改四處（serde、`TaskStatus` 兩個新值、`outstanding_actions`、`Store` trait 的新方法）加上 binding 快照型別搬家：第 1 施工關的測試與探索器全部重跑；第 3 施工關的 shim 測試重跑。
- `tasks.status` 的 CHECK 改動在 SQLite 要重建整張表（migration 裡的標準 12 步）；附舊版 fixture。

## 下一步

回 [Gate 10 入口](gate-10-pipeline.md) 看目前狀態與自動驗收。
