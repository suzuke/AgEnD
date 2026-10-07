# 第 13 施工關：安裝與發布（`install`）

> **TL;DR**
> - 服務註冊、uninstall、telegram setup、打包與發布。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：完成 13A home／設定回歸，再推進服務安裝與解除安裝。

## 狀態

**施工中：13A 回歸／13B 服務安裝**（2026-10-08）。使用者指示自行安排優先順序並建立 goal；第 12 關已合併清理。尚未完成整關驗收。

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

- 2026-10-08 13A home／初始設定已通過核心驗收與 fresh-context 六組對抗測試，含 32 個並行 init；13A 的 agend 回歸累計 290 項通過。13B 加入唯讀 service plan；macOS plutil 與隔離 Ubuntu 24.04／systemd 255 parser 通過，修正 WorkingDirectory 的原始路徑格式及含引號 executable 的 env exec 路徑。服務註冊、解除安裝和整關驗收仍未完成（feat/g13-install）。
- 2026-09-29 補「從其他施工關帶來的筆記」：backend CLI 自動更新（版本漂移）與 canary——第 7 施工關交給本關，原本頁面漏記；加上使用者討論出的方向（AgEnD 保管 agent 用的版本、被動＋主動兩種偵測、canary 失敗留在舊版）與待查證事項。

## 下一步

```bash
cat docs/gates/gate-13-install.md
~/.cargo/bin/cargo xtask accept install
```
