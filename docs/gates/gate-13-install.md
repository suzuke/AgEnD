# 第 13 施工關：安裝與發布（`install`）

> **TL;DR**
> - 服務註冊、uninstall、telegram setup、打包與發布。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：完成 13C 版本切換／回退，補 13B 真服務驗收，再接 Telegram 配對與發布。

## 狀態

**施工中：13E 打包；13B／13C／13D 驗收待補**（2026-10-08）。使用者指示自行安排優先順序並建立 goal；第 12 關已合併清理。尚未完成整關驗收。

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

- 2026-10-08：13C Codex activity 追蹤涵蓋建立中的連線與 bounded close 逾時後的舊 worker；disconnect 返回不當成停止證據。原生 Unix socket 握手阻塞與 fake app-server Gone callback 超時反例，連同既有 Codex 共 18 項通過；fmt／clippy／check-deps 通過。初版測試誤用 duplex=false（仍可完成握手）已改成原生無回覆 socket，保留失敗證據；supervisor 切換編排仍未接入（feat/g13-install，未合併）。

- 2026-10-08：13C client 1.8 接入操作員 prepare／status／cancel，RPC 不重送；原生 daemon／CLI 驗 Prepared 查詢、精確取消、重啟保留、agent／舊 ID 拒絕，以及 once decoder 的有紀錄／null。Client 52 項、core 149 項（2 項既有 ignored）、daemon 單元 185 項（1 項既有 ignored）與 switch 7 項、fmt／clippy／check-deps 通過；自有 switch Lab 無殘留。成功 prepare 准入整合、實際換版／回滾仍待完成。遠端 3e3867f CI 兩平台重啟連線逾時，已保存失敗 log，未宣稱整體通過（feat/g13-install，未合併）。

- 2026-10-08：重啟逾時追查補 executable 驗證耗時日誌；固定 binary 的本機空 fleet 重測，debug sha2 最佳化使重啟指紋核對 7296→520 ms、CLI 全程 11.518→1.114 秒，所有驗證與 10 秒重連期限保留。CLI 原 18 項通過，協定預期更新 1.8 後完整表通過；client protocol 9 項、pinned launcher 1 項、雜湊保護 4 項及 fmt／clippy／check-deps 通過。自有 g8／g9／pin／timing 暫存無殘留；兩平台遠端 CI 尚待本次 head 驗證，不宣稱逾時已全面修復（feat/g13-install，未合併）。

- 2026-10-08：13C 新增 stop_reserved，停止前核 holder PID，並在同一連線核持久 UUID／instance／agent PID 後 Shutdown；錯身分、legacy 與替代 holder 保留。三項原生反例含 binding 回覆時替換 socket 路徑，核另一 holder 存活；holder runtime 全 10 項、fmt／clippy／check-deps 通過，g6／bound-stop 測試暫存無殘留。回合結束與在途排空、supervisor 換版編排仍待接入（feat/g13-install，未合併）。

- 2026-10-08：13C prepare 在持久暫停投遞後，等待先前 Claude／inbox 回覆完成 socket 寫入；逾時保留 Prepared，後來的輪詢不延長排空範圍。原生背壓完整送出／斷線／逾時測試、daemon 185 項（1 項既有 ignored）、switch／channel／Stop 回歸通過。這只證明本機回覆結束，尚不代表 backend 回合完成或換版可安全啟動。遠端 c263492 兩平台失敗均為測試仍預期協定 1.7；更新目前 daemon 的 1.8 斷言後，Claude 控制權測試與 terminal hub 8 項通過，保留舊版相容案例。client protocol 全 10 項及 fmt／clippy／check-deps 通過，自有 g8／g12b／g11h／g11stop／switch-rpc 暫存均不存在（feat/g13-install，未合併）。

- 2026-10-08：13C 將改 program 與啟動完成分開：Committed／Restoring 跨重開維持 push／inbox 暫停，精確 Running／PID／session／新 managed launch 與 artifact 快照才可完成為 Activated／RolledBack；進行中不可被新 prepare 覆蓋。三 backend × 啟用／回滾 Store 反例及既有切換共 8 項、core 149 項（2 項既有 ignored）、daemon 單元 185 項（1 項既有 ignored）、CLI 回歸與 fmt／clippy／check-deps 通過。這是持久狀態契約，真 native readiness 與 supervisor 切換編排仍待接入（feat/g13-install，未合併）。

- 2026-10-08：13C 正式終端 acquire／resize／input 與 legacy input 接入持久暫停及在途排空，唯讀／release 保留；原生 PTY 背壓核完整 bytes，啟動前 Prepared 經正式 cancel RPC 恢復輸入。terminal hub 序列 10 項通過；並行跑有 holder 5 秒未建 socket 的啟動失敗，已加失敗日誌並保存證據，尚未解決，不以序列通過宣稱整體穩定。初版測試另開已鎖 DB 被拒，已改成啟動前建狀態。daemon 單元 185 項（1 項既有 ignored）、fmt／clippy／check-deps 通過。另發現兩個逾時後才啟動的自有 holder 重建已刪 home，保存日誌並移除自有 home 觸發 watchdog；啟動生命週期缺口仍待修正。native 回合完成與 supervisor 啟用／恢復仍待接入（feat/g13-install，未合併）。

- 2026-10-08：holder 啟動改為只在既有 AGEND_HOME 建立 run／holders 子目錄，避免延遲啟動重建已清理 home。原生子程序在 exec 前停住、刪 home 後放行，核拒絕且無目錄復活；holder 65 項、holder_process 8 項、holder_runtime 10 項及 clippy／check-deps 通過。兩個已發現的自有晚啟動 holder 與 home 已確認消失；此修正不宣稱解決並行啟動 5 秒逾時（feat/g13-install，未合併）。

- 2026-10-08：遠端 b54099a 的 Ubuntu／macOS CI 均停在 testkit 的舊 Hello 版本清單斷言；同步為目前 1.3–1.8，保留精確錯誤內容與 EOF 檢查。agend-testkit 全套 116 項、clippy／check-deps 通過；新 head 完整 CI 尚待執行（feat/g13-install，未合併）。

- 2026-10-08：13C Codex 新增 thread_idle，透過既有連線查完整回合，核 session／連線物件／generation／instance 未變，只認 completed／failed／interrupted；分頁缺 data 或 nextCursor 拒絕，不當空閒。原生 fake app-server 驗空 thread／busy／完成／session 變更／斷線與 producer 變異反例，Codex driver 19 項通過；daemon 單元 185 項通過（1 項既有 ignored）。這是閒置觀察，尚須 supervisor 暫停／排空、受管 holder 身分與完整換版／恢復接入（feat/g13-install，未合併）。

- 2026-10-08：13C OpenCode session_idle 讀 REST 狀態，前後核 instance／holder PID／session handoff／endpoint／憑證，錯 session 或 holder 消失拒絕。原生 daemon／holder／wrapper＋fake REST 兩條重啟路徑驗 idle／busy／abort 後 idle，共 3 項 native 與 20 項 OpenCode 回歸、fmt／clippy／check-deps 通過。初輪測試缺 Tokio runtime 已修正並保留失敗 log；g12open 程序與暫存無殘留。尚未接 supervisor 換版編排，不代表已驗證完整停止／啟動／回復（feat/g13-install，未合併）。

- 2026-10-08：13C 將 Claude startup 按鍵納入暫停／本機排空：Prepared 在 reservation 同交易拒絕新鍵，原操作在 server tracker 追蹤至返回；Committed／Restoring 允許新 launch 走啟動選單。SQLite 9 項及原生 Prepared 無鍵／正式 cancel 後三鍵測試通過；完整 startup 並行 5 過 6 失敗（holder 5 秒未啟動），序列 11 項通過，保留兩份證據，不宣稱並行穩定。Claude 閒置證明與 supervisor 完整換版仍待接入（feat/g13-install，未合併）。

- 2026-10-08：定位私有 executable 首次執行延遲：8 份新複本並行 --version 最慢 5.734 秒，暖啟動 7–13 ms，皆 exit 0。daemon 現在於準備階段對已驗 binding 的私有 launcher 執行 --version（30 秒等待、清空環境、前後核身分），不延長 holder 5 秒連線期限。先前失敗的 startup 預設並行 11 項及 terminal hub 預設並行 10 項全過；launcher 原生成功／失敗／逾時／替換拒絕、fmt／clippy／check-deps 通過，暫存已清。初版單元成功案例 100 ms 太短，改 5 秒，故意逾時案例仍 100 ms；保留原失敗證據。完整最新 head CI 尚未完成（feat/g13-install，未合併）。

- 2026-10-08：13C Claude 閒置觀察綁定 live hook 的 session 與原 holder connection；重連、工具活動、session 結束撤銷舊候選，初始 Ready 另核完整畫面與 generation。原生 hook／holder 反例通過，startup 回歸 11 項通過，最終 clippy／check-deps 通過；尚未接 supervisor 換版編排。21cd574 的兩平台 CI 均停在 xtask 兩個舊 Hello 清單斷言，修正後 xtask 42 項本機通過，完整新 head CI 待驗（feat/g13-install，未合併）。

- 2026-10-08：13C 一般 boot start／death restart／延遲 restart／operator retry 遇到持久 pending switch 時保留現況，讀取失敗也不停止 holder；由換版恢復流程決定後續啟停。原生 daemon 驗 Prepared／Committed／Restoring 跨 boot 精確保留 instance、managed launch、switch，連同既有 switch RPC 共 2 項通過；fmt／clippy／前後 check-deps 通過。這批未完成換版專用恢復／啟用／回滾（feat/g13-install，未合併）。

- 2026-10-08：13C Codex 閒置查詢改在同 worker 串行查原生 queue 與完整 turns；queue 非空、缺 data／nextCursor 或 continuation 不可認閒置。真 fake app-server 驗第二筆待執行訊息與消化後空 queue，producer 變異反例、Codex driver 全 19 項及 fmt／clippy／前後 check-deps 通過。仍是換版閒置前置條件，啟用／回滾編排未完成（feat/g13-install，未合併）。

- 2026-10-08：13C 接上明確 Activate／Rollback RPC 與 CLI，目的准入、reply fence、native idle、worker 結束及 exact holder stop 後提交版本並啟動；定期核新 holder binding／readiness 後釋放投遞。Activated 回退先持久 RollbackPrepared，boot 對 Committed／Restoring 缺 holder 的合格目的可重啟。Codex 雙原生假版本經正式 canary 完整啟用／回滾，核 session 保留與兩次 holder 更換通過；Store 9 項、holder runtime 10 項、switch RPC 2 項、client 52 項、core 149 項（2 ignored）、clippy／check-deps 通過。前兩輪新測試錯用外部 SQLite 查 live daemon，被獨占鎖拒絕；已改正式 RPC＋停機後核 DB。新 store 測試誤把 inbox 暫停當 None，改為正式拒絕後通過；保留失敗證據。尚未認證三後端完整往返、全部 crash 切點與自動失敗回滾（feat/g13-install，未合併）。

- 2026-10-08：本批換版回歸補 daemon 單元 186 項（1 ignored）、terminal hub 10 項、Claude startup cancel 1 項。前一批 boot 保護使兩個啟動前種 Prepared 的 shell fixture 不再啟動，改為先啟動原 holder 再停 daemon 種 pause，重連時僅保留 pause 測試所需資料，不宣稱這些 shell 具 managed admission；取消操作保留有效 terminal，僅恢復缺少的 driver。保留原回歸失敗與編譯錯誤證據，最新 fmt／clippy／check-deps 通過；自有測試目錄檢查無殘留，移除被最終證據取代的成功 logs（feat/g13-install，未合併）。

- 2026-10-08：Claude 雙假版本 canary／啟用／回滾與 session 保留通過；修正身分查詢另開 holder socket 導致原 Ready 連線被取代，改在既有連線查詢並核連線及 PID／binding。holder runtime 11 項通過（含錯身分拒絕且原連線保留）；首輪新增測試發現無 Tokio reactor 呼叫不相容，已改 blocking 等待並保留失敗證據。Codex driver 19 項通過、check-deps 通過。OpenCode 新版准入、完整 crash matrix／自動回滾及全關驗收未完成；本輪自有 switch／binding 測試目錄無殘留，保留施工 worktree／target 與必要證據。

- 2026-10-08：13C OpenCode 新版 canary 與 fleet 版本核對改依精確匯入 artifact／私有 CanaryScope；scope 綁 home inode/device、instance、workspace、program 與來源雜湊，不是 fleet 成功報告。正式 import＋scope producer 的跨 home／錯 instance／額外 argv／修改 artifact 反例通過；OpenCode 雙假版本完整 canary／啟用／回滾及 session 保留通過（66.22 秒）。未執行真模型，三 backend 全組回歸及全關驗收尚待完成（feat/g13-install，未合併）。

- 2026-10-08：三 backend 的 canary 全組 8 項並行通過（75.27 秒），包括各雙版本正式 runner、啟用／回滾及 session 保留；fmt、clippy、check-deps 通過。仍僅為原生假 backend 證據，不代表真新版模型相容或完整 crash 驗收。

- 2026-10-08：13C 三 backend 新增 Committed／新 holder 未就緒切點：fixture 在自有 workspace 等待，硬殺自有 daemon 後重開，核 holder PID 未變再繼續啟用／回滾與 session 保留；三項並行通過（75.97 秒）。fmt／clippy／check-deps 通過。尚未涵蓋 Ready 已出現、還原途中再中斷與自動失敗回滾；沒有執行真模型或改主機服務（feat/g13-install，未合併）。

- 2026-10-08：b4f9032 macOS CI 揭露 probe SIGKILL 後、waitable exit 前群組 EPERM 的競態；改先限時等待未回收 Child 退出（PID 仍固定），再清理群組／回收／核群組不存在。4 項原生 probe 通過；setsid 測試首輪未及寫 marker，改採正式 5 秒 probe 預算，仍核 marker。另驗 Codex Committed 下 daemon 硬中斷、目的 holder 由 Lab 停止後重開，核新 holder／正確版本／原 session／完整回滾通過；不代表所有 backend 或所有 crash 切點已完成（feat/g13-install，未合併）。

- 2026-10-08：13C 目前代 Committed 目的 holder 確認消失且 launch 精確吻合時自動回退；Committed／Activated 先持久 RollbackPrepared，server 就緒後續行重啟前意圖。三 backend 原生假 canary／往返／holder 消失全組 12 項通過；其後補 Codex 正式 Store 建立回退切點、重啟恢復舊版本與 session 的 1 項通過。Store 9 項、daemon 單元 186 項（1 ignored）、fmt／clippy／check-deps 通過；自有 switch／managed／canary 暫存無殘留。存活但未 ready 的 backend、Ready 已觀察後中斷、Restoring 再失敗與通知政策仍待完成；未宣稱全 crash matrix 或真模型驗收（feat/g13-install，未合併）。

- 2026-10-08：13C 將經啟動身分驗證的目前代 AgentExited 與 driver 斷線分開；目的 backend 原生退出後先持久回退意圖、排空回覆與 worker，再核精確 holder 停止及恢復舊版本／session。canary 全 14 項、daemon 186 項（1 ignored）通過；補設定快照保護後重驗 Codex 退出案例。初輪 fixture 只讓 app-server 退出而外層包裝仍活著，保留失敗 log；改成完成交接後 TUI 自行退出。app-server 單獨失敗、啟動掛住與完整 crash／通知政策仍待處理（feat/g13-install，未合併）。

- 2026-10-08：13C 換版問題持久保存原因／首次等待時間，正式 status 與「需要你」顯示，重啟恢復且不提供一般 Retry；成功完成／取消／移除時清除。原生 Codex app-server 退出但 wrapper／holder 存活測試核通知、重啟同一等待時間與 holder 通過；Store 10、core 125、daemon 186（1 ignored）、client 52 項及 fmt／clippy／check-deps 通過。測試找出 pipeline 同步誤刪通知並修復；另修正測試對省略空 actions 的錯誤假設，保留失敗證據。自有 switch／Store／canary 暫存無殘留；存活 backend 啟動逾時與完整故障矩陣仍未完成（feat/g13-install，未合併）。

- 2026-10-08：13C Committed／Restoring 同交易保存 300 秒 activation deadline；到期未就緒保存問題／通知，保留 holder 與投遞暫停，重啟不重置。正式 Store 的時鐘邊界、重開與還原新期限共 11 項、原生兩次 boot／RPC 共 2 項、core 125、daemon 186（1 ignored）及 fmt／clippy／check-deps 通過，自有 recovery／rpc／Store 暫存無殘留。這是觀察逾時，不把逾時當停止授權；真版本隔離、完整故障矩陣與整關驗收仍未完成，接續 13D 配對（feat/g13-install，未合併）。

- 2026-10-08：13D 配對驗證層接既有 notifier HTTP：10 分鐘 nonce、fresh direct human /start、bot 身分重查、精確操作員確認後才產生 token reference／allowlist／topic 設定。原生本機 HTTP 配對反例及 notifier 全 23 項、core 125、daemon 188（1 ignored）、fmt／clippy／check-deps 通過；無真 bot／訊息／模型操作，自有 Telegram 暫存無殘留。尚未接持久 pairing／cursor、CLI／RPC、設定套用或真 Telegram 驗收；13C 未完成項目仍保留（feat/g13-install，未合併）。

- 2026-10-08：13D schema 20 保存單一配對收據，候選／cursor 原子發布，精確快照拒絕舊操作，確認與取消可重啟查詢；配對 Store 4 項、既有 Store 42 項、core 125、daemon 188（1 ignored）及 fmt／clippy／check-deps 通過。自有配對測試暫存無殘留。CLI／RPC、設定套用與真 Telegram 尚未接入，整關仍施工中（feat/g13-install，未合併）。

- 2026-10-08：13D PairingService 串行銜接 notifier HTTP 與持久收據，caller 取消後仍持鎖至發布；配置中的 notifier 拒絕配對讀取。真本機 HTTP 驗取消 caller、單次 getUpdates、錯目的地／重複 Begin 零 HTTP；配對 4 項、core 125、daemon 190（1 ignored）、fmt／clippy／check-deps 通過。配對自有暫存無殘留；服務尚待 daemon 啟停、CLI／RPC 接線，未做真 Telegram（feat/g13-install，未合併）。

- 2026-10-08：13D 接 daemon 啟停、protocol 1.9 操作員配對 RPC 與 `telegram setup begin/status/poll/confirm/cancel` CLI；token 由 daemon 解析，操作不重送。真 daemon／client／CLI 測試驗 nullable 收據、重啟、agent／舊協定／過期／舊 ID 拒絕及 config 原文保留；client 52、core 125、daemon 190（1 ignored）、CLI 單元 25、fake daemon 14 與 fmt／clippy／check-deps 通過。自有配對 labs 無殘留；設定套用、真 Telegram 及整關驗收仍未完成（feat/g13-install，未合併）。

- 2026-10-08：13D `telegram setup apply --id` 由操作員 CLI 讀已確認收據，保留原文與原檔備份、0600 完整發布；不同既有設定、symlink、備份撞名及未確認收據拒絕，相同設定冪等。CLI 單元 30、原生配對／套用 2、core 125、daemon 190（1 ignored）、fmt／clippy／check-deps 通過；自有配對／apply labs 無殘留。daemon 未改寫設定或自動重啟；真 Telegram、其餘 13B/C/E 及整關驗收仍未完成（feat/g13-install，未合併）。

- 2026-10-08：13E 新增 `xtask release --out` native locked release 打包，要求乾淨提交／新輸出目錄，核版本並附 source commit、binary／archive SHA-256；不建立 tag 或公開發布。xtask 45 項、fmt／clippy／check-deps 通過；整組測試找出並修正 1.9 offer 的兩處舊快照，保留初輪失敗 log。正式 release binary 打包、brew／workflow、全新 HOME 驗收仍待驗證（feat/g13-install，未合併）。

- 2026-10-08：13E 首次正式 macOS release build 成功，但獨立解壓檢查拒絕 AppleDouble `._` metadata；打包停用 COPYFILE metadata 並新增精確 archive 清單測試。另依雙平台 CI 修正剩餘 1.9 協商斷言，Claude busy-control 1、CLI 19、terminal hub 10、xtask 45 項與 clippy／check-deps 通過；正式修正版安裝包待重產驗證，未公開發布（feat/g13-install，未合併）。

- 2026-10-08：13E 修正版 macOS ARM64 release `869a664` 已核 archive 精確清單、雙雜湊與解壓版本；新增手動四平台 native Actions 打包（只保留 artifacts、不公開發布）及共用 verifier。真安裝包 round-trip 與 7 種竄改拒絕通過，actionlint 通過；初版 YAML／runner context 錯誤已修正。四平台遠端建置、Brew、全新 HOME 與整關驗收仍待完成（feat/g13-install，未合併）。

- 2026-10-08：13E 新增 Brew formula 產生器，必須提供四份同版本／同提交且 archive 內容與雜湊通過的 native 產物；既有輸出拒絕覆寫。workflow 接四平台完成後的 formula job，施工分支相關檔案 push 觸發，只有 artifacts、不發布或改 tap。本機 native verifier 回歸及缺平台拒絕通過、actionlint 通過；完整矩陣／formula 成功路徑待遠端執行（feat/g13-install，未合併）。

- 2026-10-08：13E 安裝 smoke 使用已驗 `869a664` macOS ARM64 archive，解壓後全新 HOME init、0700／0600 權限、正式 daemon／holder 與 fake worker 完成 checks／review／核准／唯一 merge，archive 到完成含清理 9.81 秒；設定原文保留、worktree 消失。新增 pipeline_probe install 與 release_install_smoke.py，接四平台 workflow；fmt、範例 clippy、actionlint、check-deps 通過，自有目錄與匹配 holder／worker 無殘留。此證據使用 fake worker／fixture 設定，不宣稱真 backend 首任務、Brew install 或整關完成（feat/g13-install，未合併）。

- 2026-10-08：13E 四平台 native release 及 formula job 在 run 37738455556（0825e23）全部通過，保留生成公式；該 run 尚未含後續首任務 smoke。macOS ARM64 以 `cargo install --locked --path crates/agend --root <自有暫存>` 安裝 c085ddf，真 installed binary + fake worker 首任務通過，含建置共 100.699 秒，prefix 已刪。補安裝文件並明列 crates.io 未發布、真 backend／Brew 安裝與完整驗收待補（feat/g13-install，未合併）。

- 2026-10-08：13E doctor 補真 CLI 故障／恢復案例：缺 HOME、0755→0700、Telegram 語法損壞且不覆寫、sandbox 工具缺失→真 probe 恢復、三 backend 缺 PATH→僅 --version fixture 恢復。install_home 共 7 項通過、focused clippy 通過；未執行真模型。磁碟容量故障、服務／登入／版本診斷與整關矩陣仍未完成（feat/g13-install，未合併）。

- 2026-10-08：13E doctor 新增唯讀 service 診斷：無安裝可前景執行；核收據歸屬／artifact／manager 狀態與查詢後收據一致，Prepared／Removing／未執行提示修正，缺失或變造拒絕。診斷不取得或建立 install.lock，不啟停／reload。持正式 lifecycle 鎖的 model 故障／恢復反例、service 18、install_home 7、CLI doctor 1 項與 clippy 通過；初輪測試缺 Path import 已修正並保留 log。未驗主機服務，登入／版本與整關故障矩陣仍待完成（feat/g13-install，未合併）。

- 2026-10-08：CI 加入 workflow＋ref concurrency，同分支／PR 的較新提交取代舊 run，其他 workflow／分支分開。已核身分後取消本施工分支五批被取代 CI，保留 869a664 基準與獨立 release 驗收；取消不視為通過。actionlint 通過（feat/g13-install，未合併）。

- 2026-10-08：13E 磁碟診斷補真 CLI sparse file 邏輯用量 >20 GB 警告／刪 fixture 後恢復，install_home 全 8 項通過。另以自有 64 MB HFS+ sparse image 實測 62 MB free→disk fail，卸載擴容 2 GB 再掛載→ok；映像已卸載刪除並核 hdiutil 無自有掛載。未填滿主機磁碟，未啟動模型。c085ddf 四平台 release 首任務及 formula 全部通過；全關 CI／真服務及模型驗收仍待完成（feat/g13-install，未合併）。

## 第 13 關原生 demo

`cargo xtask demo install` 建置正式 agend、fake-worker 與 pipeline_probe，從 Cargo 的 artifact 訊息取得執行檔路徑。依序執行 home／doctor、服務定義、backend import／switch／canary、Telegram 配對與服務 lifecycle 模型案例，再以全新 HOME 執行 init、首任務、唯一 merge 及清理。`accept 13` 在一般 crate checks 與 check-deps 後呼叫同一入口。

測試只用隔離目錄、假 backend 與本機 Telegram producer，不註冊主機服務或呼叫真模型。archive 四平台驗證另由 release workflow 執行；Brew install、真服務與真 backend／Telegram 驗收不可由此 demo 的成功取代。

- 2026-10-08：新增 `xtask demo install` 並接 `accept 13`，建置三 backend fixtures、共 52 項原生 home／service／import／switch／canary／配對案例通過。初輪首任務遇 macOS 長 socket 路徑而失敗，改 TMPDIR=/tmp 後首任務 10.292 秒完成且唯一 merge／清理通過；保留失敗 log，未重跑已通過案例。xtask tests、clippy、fmt、check-deps 通過；這是離線原生 demo，Brew／真服務／真模型與外部 Telegram 驗收仍另列（feat/g13-install，未合併）。

Brew 原生安裝驗證已接入 release workflow 的 macOS ARM64／Linux x86_64 jobs，僅在一次性 Actions runner 執行。`release_brew_smoke.py` 重新產生並逐字核對四平台 formula，僅將下載 URL 換成同輪 archive 的 file URL，保留 SHA 與安裝邏輯；建立唯一 tap，拒絕既有 agend 安裝，跑 install／formula test／全新 HOME init／uninstall 並檢查清理。這不證明公開 Release URL 已可下載；實跑結果另記。

- 2026-10-08：13E 覆核找出 doctor 版本 probe 無界 reader join 與非零退出誤判，改非阻塞／64 KiB 上限／5 秒總 probe 期限及 2 秒限時清理，核尚未回收的直接 PID 與原群組。原生錯誤／超量／繼承 pipes／自行換群組反例通過；整組測試另抓安裝鎖 close 遇 fork 繼承的競態，改明確 unlock guard 加副本反例後並行 binary 35、install_home 8 項通過。保留初輪失敗，暫存已清；doctor 非惡意程序沙箱，登入／版本相容／漂移與真測仍待補（feat/g13-install，未合併）。

- 2026-10-08：release run 37747622359 的四平台 archive／首任務、formula 與 macOS ARM64 Brew install／test／init／uninstall 全部通過。Linux Brew 在 tap-new 的 Git commit 因 runner 無作者身分而失敗，tap 已 untap 清理；測試子程序補專用 Git author／committer 環境值，不改全域設定，待重驗（feat/g13-install，未合併）。

- 2026-10-08：13C canary 新增明確 `--model`，OpenCode 使用 provider/model；模型參數綁定私有 scope 並寫入報告（要求值，不冒充 provider 解析結果）。非法名稱建 home 前拒絕、替換／移除 scope args 拒絕，以及原生 OpenCode fake 三次成功 outcome／清理通過。真帳戶認證與真模型驗收仍待完成（feat/g13-install，未合併）。

- 2026-10-08：release 37749079834 四平台 archive／首任務、formula 與 macOS Brew 通過，Linux Brew 再次因 tap-new 作者身分失敗；Brew 過濾 Git 身分環境值，改一次性 tap-new --no-git，避免更動共享 Git 設定。Python 語法檢查通過，原生 install／test／uninstall 留待下一輪 CI（feat/g13-install，未合併）。

- 2026-10-08：CI 37749079770 Ubuntu 通過、macOS version probe 斷言失敗；舊斷言未印實際錯誤，根因尚未證實。probe 清理失敗時保留原始錯誤、斷言印錯誤，時間斷言涵蓋 250 ms probe 加 2 秒清理預算（未延長實際期限）。本機 binary 37 項及 workspace clippy 通過，macOS CI 待重驗（feat/g13-install，未合併）。

- 2026-10-08：13C 新增專用 --auth-file 私有副本與 canary HOME／Codex／Claude 環境隔離；啟動前核完整 scope 的 program／args，正常 fleet 不新增憑證白名單。Codex 模型用 app-server 支援的 -c model=…；三個真 native fake consumers 核實收到模型測試對應的認證與私有路徑，缺副本不得跳過。最後 7 項 canary 案例、binary 37 項、workspace clippy 與補強後 testkit clippy／fmt／check-deps 通過；來源不變且自有 lab／程序清理。未讀真憑證、未呼叫模型；真測與最終 fresh verifier 仍待完成（feat/g13-install，未合併）。

- 2026-10-08：3f52828 的 macOS CI（37753100715）通過前輪 doctor timeout 案例，但 canary session-move 測試失敗；未印實際錯誤，根因仍待 CI 蒐證。補 probe 退出狀態與原生 fixture 的 setpgid／setsid 階段診斷，期限及必須真正 detached／回收的斷言不變。本機四個 process 案例通過，不據此宣稱已修復 CI（feat/g13-install，未合併）。

- 2026-10-08：doctor 改診斷 fleet 明示的設定 program，保管 bytes 變動 fail 且不執行，無當前 CLI build canary／外部程式／無 daemon PATH 證據則 warn；不冒充登入或漂移通知完成。install_home 9 項及相對程式補驗、真／fake CLP（含新增 program producer 斷言）、CLI doctor、舊協定相容 16 項、workspace clippy／fmt／check-deps 通過。首輪 socket fixture 名過長及遺漏 xtask 初始化編譯失敗已修正、保留 log；自有 lab／程序已清（feat/g13-install，未合併）。

- 2026-10-08：release-artifacts 37753100820（3f52828）全部成功：Linux x86_64／ARM64、macOS Intel／ARM64 archive 與首任務、formula、macOS／Linux Brew install／test／fresh HOME init／uninstall。這是 Actions 一次性環境驗證，尚未公開發布，亦未取代主機服務與真 backend／Telegram 驗收（feat/g13-install，未合併）。

- 2026-10-08：13C 新增 backend latest，固定 npm 公開來源、核套件身分，拒絕轉址／超量／錯誤，5 秒期限涵蓋標頭與 body。真 metadata 三套查詢及無 HOME／agent 拒絕驗證通過；四項 HTTP fixture 反例、core 全組（兩個 deep ignored）、daemon 194 項（1 child ignored）、CLI 37 項及 workspace clippy／fmt／check-deps 通過。未呼叫模型或切換版本；每日排程、持久漂移通知及整關真測仍待完成（feat/g13-install，未合併）。

- 2026-10-08：13C migration 0021 保存每日 registry 嘗試與結果，重啟／時鐘倒退不提早重查，失敗保留上次成功值，舊 attempt／重複完成及舊 revision 確認拒絕。真 SQLite 兩項重開反例、Store 42 項（含全部歷史 schema 升級／retention）、workspace clippy／fmt／check-deps 通過；clippy 初輪多餘 unit expression 已修正。測試暫存無殘留；此批是持久 Store API，定時 worker／attention 與被動漂移仍待接線（feat/g13-install，未合併）。
