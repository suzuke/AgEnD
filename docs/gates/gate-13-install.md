# 第 13 施工關：安裝與發布（`install`）

> **TL;DR**
> - 服務註冊、uninstall、telegram setup、打包與發布。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：完成 13C 版本切換／回退，補 13B 真服務驗收，再接 Telegram 配對與發布。

## 狀態

**施工中：13B 服務安裝／13C 版本管理**（2026-10-08）。使用者指示自行安排優先順序並建立 goal；第 12 關已合併清理。尚未完成整關驗收。

## 施工順序（2026-10-08）

1. **13A home／設定與文件收尾**：操作員優先使用明確 AGEND_HOME，未設時使用 $HOME/.agend；空白／相對路徑拒絕。agent 必須保留明確 AGEND_HOME；config.toml 不再重複保存 home。init 建 0700 home 與 0600 初始設定，保留既有檔案並拒絕 v1／設定 symlink。
2. **13B 服務與解除安裝**：產生 user launchd／systemd 設定、安裝所有權紀錄與對帳；驗證停止／重啟 daemon 保留 holder，解除安裝只處理自有資源，資料刪除另行明確確認。
3. **13C backend 版本管理**：隔離保管版本、偵測漂移、canary、明確升級／回退；不修改使用者共用 CLI 或 trust entries。
4. **13D Telegram 配對**：經 daemon notifier 完成 token reference、/start 與 allowlist，CLI 不新增 Telegram client。
5. **13E 打包與驗收**：release artifacts／brew／cargo install 路徑、全新 HOME 五分鐘首任務與 doctor 故障矩陣、獨立覆核／CI。公開發布與使用者既有常駐服務變更先完成可審閱成果。

每批驗證後清理自有程序及暫存；合併後刪除 worktree／分支／target。新工作樹前先檢查上一批已完成資源不存在，保留必要證據。此頁的「已實作」不等於整關完成。

## 範圍

- launchd／systemd 服務註冊
- `agend uninstall`（移除服務與 shim；刪資料前先問）
- `agend telegram setup`（由 daemon 的 notifier 配對）
- `xtask release`、brew formula、GitHub release workflow、`cargo install`
- systemd unit 必須 `KillMode=process`（預設 `control-group` 會在重啟 daemon 時殺掉所有 holder，D3 失效）；實測 launchd 重啟 daemon 時 holder 存活（第 4 施工關風險，使用者 2026-09-25 決定）

### 從其他施工關帶來的筆記（開工時處理）

- **`AGEND_HOME` 預設位置與 `config.toml` 要不要列 home**：第 9 施工關 P3 本關（CLI）沒有預設值，13A 已選定操作員預設 `$HOME/.agend`，`config.toml` 不重複列 home 路徑；原先延後的問題為預設位置與 home 設定歸屬（D8／[tui-and-setup](../architecture/tui-and-setup.md#設定與目錄d8) 的寫法）（見 [gate-09-cli P3](gate-09-cli.md#p3agendhome-怎麼找)）。
- **backend CLI 自動更新（版本漂移）**：codex 會自己更新（2026-09-28 兩天內 0.156.1 → 0.158.0，一次改了 driver 依賴的 K14、K17、K18 與錄音沙箱要寫的檔），claude、opencode 也會。設計上有「偵測到 backend 版本變了，先在暫存 workspace 起 canary instance 確認能進 ready，再讓整個 fleet 重啟；不行就提前通知」（[delivery.md](../architecture/delivery.md#啟動與授權提示四層越上面越優先)），第 7 施工關把 canary 交給本關（[gate-07-codex P9](gate-07-codex.md#p9依賴規則這關不做的事)），但 daemon 目前跑著時完全不知道版本換了。本關開工時要決定：（1）canary 怎麼做；（2）AgEnD 要不要替 agent 關掉 backend 的自動更新（claude `DISABLE_AUTOUPDATER=1`、opencode `OPENCODE_DISABLE_AUTOUPDATE=1`；codex 的方法未查證），改由使用者手動升級。建議提前的一小步（不必等本關）：daemon 每次啟動 agent 前記下 backend `--version`，跟上次不同就 log 一行並在「需要你」出一項提醒（只提醒、不擋；第 7 施工關[已知風險](gate-07-codex.md#已知風險開工時處理)的建議 3），由第 10 或第 12 施工關順手做，做了就在這裡註明。
  - **使用者提出的方向（2026-09-29 討論，開工時再定）**：
    - **為什麼不能只關自動更新**：agent 與使用者自己開的 claude／codex 是同一個執行檔；只在 agent 環境關自動更新，使用者自己用時它還是會更新，agent 下次重起就跑新版。更新後舊版通常不會留下，所以「退回舊版」不能靠 CLI 自己。
    - **AgEnD 保管 agent 用的版本**：把能用的版本複製到 `$AGEND_HOME/backends/<backend>/<version>/`，agent 一律從那裡啟動（`--program`）；使用者自己的 CLI 照常更新，不影響 agent。
    - **兩種偵測**：（a）被動：定期或每次啟動 agent 前對系統上的 CLI 跑 `--version`，變了就知道；（b）主動：每天查一次官方最新版（例如 npm registry，或 CLI 自己的檢查更新指令），補上「使用者很少開那個 CLI、它一直沒更新，agent 就一直落後」的漏洞。落後時在「需要你」提醒，由使用者決定何時升。
    - **同一套升級流程**：複製新版 → 在暫存 workspace 用新版跑 canary（短劇本：啟動 → ready → 送一則 → confirmed → 結束，約 3 個短回合，類似第 7 施工關的 `codex_live`）→ 通過就換（自動換或先問，看設定），舊版那份留著可以退回；失敗就留在舊版、fleet 照常，「需要你」出一項，並可自動開 task 請 agent 修 driver（附失敗的 canary 紀錄，merge 仍由使用者核准）。
    - **canary 的極限**：抓得到啟動卡住、送達壞掉這類明顯問題，抓不到細微的行為改變（0.158.0 一次改三件事，短劇本可能只踩到一件）。
    - **待查證**：（1）三個 CLI 能不能整份複製到別處照常跑（claude 新版是單一執行檔應該可以；codex standalone 是套件目錄大概可以；npm 裝的要連依賴一起）；（2）CLI 會不會檢查自己的安裝位置，或啟動時又自己更新（要搭配各自關自動更新的設定；codex 的方法未查證）；（3）各家有沒有「查最新版」的指令或穩定的查詢來源。
- **`sandbox-exec` 被 Apple 標為 deprecated**：第 10 施工關 checks 用的沙箱工具在 macOS 上是 `sandbox-exec`，Apple 已標為 deprecated；目前的 macOS 仍能用，未來版本拿掉時要另找工具，checks 會 fail closed 出現 `sandbox-missing`（見 [gate-10-pipeline P6](gate-10-pipeline.md#p6command-關卡runner)）。本關打包發布時留意目標 macOS 版本是否仍支援它。

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-core`、`~/.cargo/bin/cargo test -p agend-daemon`、`~/.cargo/bin/cargo test -p agend` 單獨通過
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept install` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

1. 全新 HOME 從 release 安裝到第一個 task 完成。

   操作：開工時細化：下載 release 產物 → `agend init` → 建立 task

   應該看到：印出總耗時，小於 5 分鐘。

   - [ ] 通過

2. 故意弄壞：每個 doctor 檢查。

   ```bash
   ~/.cargo/bin/cargo xtask accept install
   ```

   應該看到：每個檢查一段：「弄壞 → doctor 顯示修正指令 → 照做後恢復」。

   - [ ] 通過

3. 解除安裝（這時的 `agend` 是第 1 步裝好的正式版；v2 服務標籤暫定 `dev.agend.daemon`，確切值開工時細化）。

   ```bash
   agend uninstall
   # macOS
   launchctl list | grep -F dev.agend.daemon; echo "service matches: $?"
   # Linux
   systemctl --user list-units --all | grep -F agend-daemon.service; echo "service matches: $?"
   ```

   應該看到：刪資料前先問；之後服務檢查印 `service matches: 1`（grep 找不到東西），agent PATH 的 shim 不見了。只比對 v2 的標籤，所以 v1 的 `com.agend-terminal.daemon` 不會被算進來。

   - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-10-08：13C runtime 保存 executable 指紋，並核實際 running image：macOS executable mapping／Linux procfs，比對 capture 前後與後續檔案身分；macOS 兩項 native binding、4 項 canary 與 5 項匯入回歸通過。覆核確認捕獲前替換缺口修補；驗證到 exec 的路徑替換仍待修，因此即使 canary 正確也維持拒絕受管啟動。Linux 新 binding 尚待原生執行（feat/g13-install，未合併）。

- 2026-10-08：13C 完整 fake canary 已涵蓋三 backend：4 項共用原生程序測試通過，Claude 走真 Ready parser、MCP ACK helper、PostToolUse／Stop，再核三個不同 execution ID；缺失／重用 execution ID 的報告拒絕。歷史 fake Claude 3 項與 conformance 6 項通過，聚焦覆核未發現 blocker。未執行新真模型、未驗證認證隔離、未准入或切換版本（feat/g13-install，未合併）。

- 2026-10-08：13C OpenCode 接入獨立回合證據，核對 native parentID／literal input／成功回覆及查詢前後 session／holder／endpoint；真 1.18.34 capture 反例與原生 fake CLI canary 驗證，保留不准入與不切換 fleet 的邊界。Claude outcome 與整體版本切換仍待完成（feat/g13-install，未合併）。
- 2026-10-08：13C canary 新增獨立 message_outcome；Codex 以正式 thread history 核對單一輸入、訊息身分、成功回合與非空白回覆，排除 confirmed／idle 誤認成功；原生流程與異常證據覆核持續驗證。Claude／OpenCode outcome、版本准入與切換尚未完成（feat/g13-install，未合併）。
- 2026-10-08：13C 受控 canary 的 fake Codex 三訊息流程與操作員 sender 命名空間通過；版本探測限制子程序建立、self-setsid 回收及路徑轉向拒絕通過 macOS／Linux ARM64 原生證據與聚焦覆核。這不是三 backend 真模型或整關認證；13B 真 launchd、13C 切換／回退、13D–E 尚未完成（feat/g13-install，未合併）。

- 2026-10-08 13A home／初始設定已通過核心驗收與 fresh-context 六組對抗測試，含 32 個並行 init；13A 的 agend 回歸累計 290 項通過。13B 加入唯讀 service plan；macOS plutil 與隔離 Ubuntu 24.04／systemd 255 parser 通過，修正 WorkingDirectory 的原始路徑格式及含引號 executable 的 env exec 路徑。服務註冊、解除安裝和整關驗收仍未完成（feat/g13-install）。
- 2026-09-29 補「從其他施工關帶來的筆記」：backend CLI 自動更新（版本漂移）與 canary——第 7 施工關交給本關，原本頁面漏記；加上使用者討論出的方向（AgEnD 保管 agent 用的版本、被動＋主動兩種偵測、canary 失敗留在舊版）與待查證事項。

### 13B 施工覆核（Linux 已真測，整關尚未驗收）

- 安裝／解除安裝 receipt 對帳已加入；真 CLI 的五項隔離檔案測試通過，不註冊使用者服務。
- fresh-context source review 找出 systemd effective overrides、holder 路徑轉向、daemon 啟動前 socket 空窗三個阻擋項。
- holder 路徑加入逐層目錄與 socket 檢查，兩個反例及八項 lifecycle 模型測試通過；仍待重新獨立覆核。
- daemon 排他生命週期加入共享／排他維護鎖與原生 SQLite 鎖；startup 空窗反例與 42 項 store、5 項跨程序回歸通過；fresh source review 確認新 daemon 的移除排他。註冊前的探測會在呼叫 manager start 前釋鎖，僅能拒絕當時存在的 owner；競爭啟動仍由 SQLite 排他仲裁，不能宣稱原子移交。systemd 已核對實際載入的 D-Bus 屬性，原生 systemd 255 baseline／drop-in fixture 測試通過；覆核追加的「停止時隱含 reload」及「恢復設定但 MainPID 仍為外來程序」已補防護，Linux 真執行反例已通過：未載入的 ExecStop 未執行、外來 MainPID 拒絕且存活；停止／重啟 daemon 保留同一 holder，解除安裝停止自有 holder、保留資料／外來 drop-in。另以全新 home 完成直接安裝、執行中狀態及解除安裝。已加入明確刪資料路徑確認與保留鎖 inode 的清理，獨立覆核後補實際 mount identity 檢查；Linux 真 bind mount 反例已通過，外部資料保持不變；macOS 真服務驗收仍待完成。以上結果不代表 13B 或第 13 關通過。

操作與刪除邊界見[服務安裝](../architecture/service-install.md)。

- 2026-10-08：macOS live PID 的映射 inode／argv／home／UID／世代檢查與 native C／SDK ABI／同路徑替換反例已通過，整組服務 16 tests 通過。尚未註冊本機服務；唯一暫時 label 的 launchd 設定捕獲已備妥，待授權後執行，不代替完整 Rust daemon 驗收。

13C 已加入原生副本匯入／inspect、agent 更新環境隔離及啟動前受管路徑核對；另補 1.7 唯讀 delivery 收據 API 供 canary 對帳。[介面與限制](../architecture/backend-versions.md)。套件、漂移、canary、切換／回退尚未完成。

- 2026-10-08：13C 加入 Claude 原生 ACK／PostToolUse／Stop 的唯讀完成判定，以 prompt ID 綁定單次執行；SessionStart／SessionEnd 即使 replay 或延遲也會阻斷跨生命週期拼接。保存的原生證據反例與 Store 重開測試通過，獨立覆核關閉此缺口；尚未執行完整 Claude canary 或真模型。自有測試暫存／程序已核對無殘留，未合併工作區與必要證據保留。

- 2026-10-08：正常 daemon 已使用固定啟動副本；原始 binary 替換後的新 holder／shim、停止 daemon 保留 holder，以及自有 Lab 清理的原生測試通過。覆核發現服務 shim ownership 需接受安裝紀錄 digest 對應的副本，已補處理與反例測試；完整回歸、跨版本重連與准入仍未完成。

- 2026-10-08：固定產物下三 backend canary 4 項、服務 6 項、shim ownership 與 daemon 回歸已通過，fmt／clippy／check-deps 通過；清理確認無本批自有程序與暫存。准入仍缺重連時存活 backend 的持久啟動綁定，保留 D3／D5 既有跨版本契約，並非要求所有 holder 同 build。

- 2026-10-08：holder 1.3 加入不可補認的原生啟動 UUID 回報；65 項 holder、149 項 core（另 2 項既有 ignored 未執行）、workspace clippy／check-deps 通過。覆核發現停止時第二次 Spawn 競態，已以永久 spawned 與 stopping guard 修正；原生 HUP 重疊測試通過，移除防護的 mutant 實際產生第二個 PID 並失敗。daemon 持久綁定仍待接入、准入關閉，本批 holder 暫存已清理（feat/g13-install，未合併）。

## 下一步

```bash
cat docs/gates/gate-13-install.md
~/.cargo/bin/cargo xtask accept install
```

- 2026-10-08：holder 啟動 UUID 與關閉競態修補提交 `715bdf0`；65 項 holder、149 項 core（另 2 項既有 ignored）、runtime 回歸與 clippy／check-deps 通過。Ubuntu canary 三回合完成後超出 60 秒測試預算，整合測試改採正式 180 秒預設；本機三 backend 原生 fake canary 4 項於 87.23 秒通過，包含程序／暫存清理。遠端 CI 與 daemon 持久啟動紀錄仍待完成，不宣稱整關或真模型通過。

- 2026-10-08：修正固定 launcher 重啟時的重複雜湊，重用快照驗證產生的檔案身分 binding；保留 running image／摘要／ownership 檢查與 10 秒 CLI 等待期限。原失敗 CLI table 原生重跑通過（38.92 秒），snapshot 2 項與 pinned launcher 1 項、clippy、check-deps 通過，聚焦獨立覆核未發現 blocker。同時更新 client 1.7 協商斷言；整關 CI／持久啟動紀錄仍待完成（feat/g13-install，未合併）。

- 2026-10-08：13C migration 0018 新增受管啟動意圖，UUID、artifact 與啟動參數在同一 SQLite transaction 以 instance 快照及舊 binding CAS 保存；明確移除 instance 時 cascade，重建不繼承。原生 SQLite 新測試 2 項、既有 store 42 項及 core 149 項通過（2 項既有 ignored 未執行），fmt／clippy／check-deps 通過；聚焦覆核無 blocker，測試暫存已清理。supervisor／runtime 串接與准入仍未完成（feat/g13-install，未合併）。

- 2026-10-08：13C reserved runtime 接入 SpawnBound／GetLaunchBinding，持久 UUID／PID 核對後才發布 writer；重連不送 Spawn，不符時保留 holder。未驗證 Exited 暫存，intentional close 不誤報失敗且核對可取消。原生 holder 7 項、runtime 19 項、終端 hub 8 項與 fmt／clippy／check-deps 通過，聚焦覆核缺口已修正；清除重複驗證 logs。supervisor 的受管啟動／版本准入決策尚未串接（feat/g13-install，未合併）。

- 2026-10-08：13C supervisor 接入受管啟動准入與持久 UUID；canonical 匯入程式進 launch argv，先核舊 holder／orphan 已離開再保存意圖。重連核原 artifact／設定／UUID，不重跑新版 canary；首次 Codex／OpenCode 原生 session 發現與啟動時指定 session 分開。三 backend 原生 canary＋fleet 啟動／重連／錯 UUID 保留程序共 4 項通過（102.21 秒），supervisor 8 項、匯入拒絕回歸 5 項、fmt／clippy／check-deps 通過；聚焦覆核兩項缺口已修正。版本切換／回退與整關驗收仍待完成（feat/g13-install，未合併）。

- 2026-10-08：補受管 canary build 身分失配反例，正式報告改成不匹配 digest 後，新啟動准入拒絕，三 backend 的原 holder 仍以持久 UUID 重連；原生 4 項通過（99.98 秒），fmt／clippy／check-deps 通過，自有程序／lab 清理完成。更新版本管理文件移除已失效的「全部拒絕准入」敘述；不宣稱已驗證所有跨版本 driver 相容性（feat/g13-install）。

- 2026-10-08：13C migration 0019 保存每 instance 最近一次 BackendSwitch，prepare 不改 program，commit／rollback 與 phase 同交易；設定、舊 switch 或來源啟動意圖變更拒絕提交。原生 SQLite 3 項、store 42 項、core 149 項通過（2 項既有 ignored），fmt／clippy／check-deps 通過；聚焦儲存契約覆核無 blocker，測試暫存與重複 clippy log 已清理。尚未接操作入口、idle 排空、停止／啟動與 Prepared 取消／恢復，不宣稱完整版本切換完成（feat/g13-install）。

- 2026-10-08：13C Prepared 切換可取消，保留原 program 與 agent PID，取消後可建立新請求；Committed 拒絕取消，須走 rollback。原生 SQLite 4 項、fmt／clippy／前後 check-deps 通過，測試暫存與重複 log 已清理。這批只完成儲存契約，CLI、投遞排空及 supervisor 恢復仍未接入（feat/g13-install，未合併）。

- 2026-10-08：13C OpenCode runtime 保留尚未退出的取消／舊代 worker，以實際 thread completion 提供停止查詢；重連及重複取消不會丟失舊代。新增原生 worker 生命週期 1 項、既有 OpenCode 20 項通過，fmt／clippy／check-deps 通過；對應測試暫存皆不存在。尚未接入 supervisor 版本切換，worker 停止不代表 backend 回合結束（feat/g13-install，未合併）。

- 2026-10-08：13C Prepared 在正式 Claude channel／Stop、Codex、OpenCode reservation 暫停新的 push attempt，取消後恢復，原回執可確認；6 項切換測試、16 項 Codex 回歸、daemon 單元 185 項通過（1 項既有 ignored）。修正舊 store 測試以精確保留永久 maintenance lock 並核 inode／權限；fmt／clippy／check-deps 通過，85 種相關測試目錄無殘留。inbox、在途寫入排空、idle 與 supervisor 切換編排仍未完成（feat/g13-install，未合併）。

- 2026-10-08：13C agent inbox 讀取與 Prepared 檢查在同一 DB 工作執行，準備中明確拒絕送出內容，操作員歷史與其他 instance 不受影響；取消恢復最後筆數／after 游標。7 項切換測試、3 項正式 pipeline context、既有 handler 回歸及 fmt／clippy／check-deps 通過。supervisor 的在途回覆排空、idle 與版本切換編排仍待接入（feat/g13-install，未合併）。
