# 第 12A 提案：P3–P5 權限與啟動

> **TL;DR**
> - 權限模式、gh 防護與啟動對話框。
> - 已選 P3＝A、P4＝A、P5＝A；原選項保留供比較。
> - 下一步：依 D40 實作並驗證選定行為；目前只有文件確認。

## P3：claude 的權限模式（安全決定，請選一個）

- 問題：claude 沒有人在旁邊按「允許」。用哪個權限模式？
- 官方參考：[權限模式](https://code.claude.com/docs/en/permission-modes)、[規則](https://code.claude.com/docs/en/permissions)（2026-10-03 查核）。實際 deny／auto 行為仍須在選定 CLI 版本驗證，不能由文件查核直接認證。
- 風險：agent 在你的 uid、你的網路下跑；進來的文字（其他 agent 的 `agend send`、它讀到的檔案與網頁）都可能帶 prompt injection。
- 選項：
  - A. bypass：略過一般權限提示；仍受 CLI 本身的其他限制，不能理解成完全不擋。
  - B. bypass ＋ deny 規則：`--settings` 裡擋 `Bash(gh pr merge*)`、`Bash(gh api*merge*)`、`Bash(gh pr review*--approve*)`，以及讀 `$AGEND_HOME/secrets`、`$AGEND_HOME/run`（daemon 寫成展開後的絕對路徑 `Read(//<絕對路徑>/**)`）。字串比對，擋手滑、不擋故意。
  - C. auto：以分類器審查動作，可能增加延遲與費用；**2026-09-28／2.1.283 的 F5：sonnet 可用；haiku 退回 manual**，其他模型與現行版本仍待查證，manual 下的提示沒人按，agent 就停住。
- 已選：**A，bypassPermissions，不加入原 P3 的 deny 規則**。原建議 B 未採用；不能把既有個人設定或選項 B 的規則默默加回去。一般提示略過仍不代表所有提示消失；實際啟動與管理政策須驗。
- 原建議理由：B 希望減少手滑，但字串規則不作沙箱；選 A 後由 P4 的共用 shim 防常見 gh 誤操作。
- 替代方案：A、C。
- 例子：選 B 時 agent 跑 `gh pr merge 42` → claude 回 `denied by permission rule`。
- 關係：第 7 施工關 P4（codex full-access）是同一類決定。F4 只代表當時那台機器的設定；P2 不讀使用者設定，啟動警告仍要以選定設定實測。
- [x] 使用者確認（**A**；[確認紀錄](gate-12a-confirmations.md)）

## P4：agent 可以用你的 gh 登入直接 merge（安全決定，請選一個）

- 問題：agent 沒有沙箱，PATH 上的 `gh` 用的是你的登入。它可以直接 `gh pr merge 42`，跳過之後流水線的人工核准；shim 只守 git。好意的 agent 很可能手滑打出這行（它以為最後一步就是 merge），照第 3 施工關的判斷方法屬於應該擋的。這件事在 claude 一跑起來就存在，跟 C 段做不做無關。
- 選項（只用 A 段做得到的）：
  - A. shim 也包 `gh`：拒絕 `pr merge`、`pr review --approve`、`api` 打到 merge／reviews 路徑、`auth token`，其餘放行。所有 backend 都有效。
  - B. 只靠 P3 的 deny 規則：只對 claude、只在 P3 選 B 時有效。
  - C. 接受，寫明：本段不處理，依賴目標 repo 的 branch protection；AgEnD 目前整合 branch 是 v2，不能只寫 main。仍須檢查自己的登入是否能繞過保護，不把 branch protection 當成萬用保證。
- 已選：**A**；拒絕規則在 A 段實作，操作者直接使用的 gh 不注入 shim；防誤操作、不防刻意繞過。原 B 在 P3 選 A 後不適用。
- 理由：F6 證實 claude 跑的指令會先找到 shim，包一層就能對每個 backend 擋掉最常見的手滑。
- 替代方案：B、C。
- 例子：選 A 時 agent 跑 `gh pr merge 42` → `agend-shim: refused gh pr merge (merges go through agend)`，exit 1。
- 關係：A **擴大第 3 施工關 shim 的範圍（目前包括 git、kill、pkill、killall 與 git hooks），請明確決定**。
- [x] 使用者確認（**A**；[確認紀錄](gate-12a-confirmations.md)）

## P5：claude 的啟動對話框

- 問題：2026-09-28／2.1.283 的 F1、F3：新目錄第一次有信任對話框；**每次啟動**都有 development channels 警告（包括 holder 死掉重起）。沒人按，claude 就停在那裡，也收不到訊息。選定版本與 P2 設定仍須重驗。
- 選項：
  - A. 用 core 已有的螢幕規則表 `agend_core::screen::SCREEN_RULES`（第 1 施工關；已經有 claude 信任與 MCP 兩條，類別 `StartupMenu`，`suggested_key` 目前是空的；每條最多一個鍵，[delivery](../architecture/delivery.md) 第 3 層）。改成三條，都附 F1 的真畫面 fixture：信任對話框游標在「❯ No, exit」→ `Down`；游標在「❯ Yes, I trust this folder」→ `Enter`；development channels 警告 → `Enter`。daemon 只在 claude 還沒送出 `SessionStart` 之前，對 holder 的畫面跑 `classify`，比對到有 `suggested_key` 的就經 holder 送那一個鍵（現有 holder 單鍵送入；須測第 11C 多視窗 owner、斷線與 resize 互動）。
  - B. development channels 用螢幕規則按 `Enter`；信任由你手動：`agend instance add` 之後印一行「第一次要在 TUI 接受信任」，你 attach 進去按一次（claude 自己記在 `~/.claude.json`，之後不再問）。
  - C. 不用 development channel（沒有 channel，D16 的「閒置經 channel 送」做不到）。
- 已選：**A**；只在啟動期間處理已知畫面，遵守第 11C 的 holder 控制權；未知提示交由人處理。信任的範圍依選定版本／repo 根目錄，不把歷史「每個 instance 都問」當成現行保證。
- 理由：用的是已經存在的規則表與送鍵，每條還是單一按鍵；信任要兩步，就用游標位置分成兩條。啟動遇到已知信任提示時自動處理，避免依賴人工值守；是否再次提示依 CLI 版本及 repo 信任狀態核實。
- 替代方案：B、C；預寫 `~/.claude.json` 的信任狀態（改你的檔案）。
- 例子：新 instance 第一次起 → log `g12-c: dialog "trust" answered (Down, Enter)`、`dialog "dev-channels" answered (Enter)`。
- 關係：delivery.md 第 3 層照做（一條規則一個鍵）。新的部分只有兩樣：`SCREEN_RULES` 的資料改動（core，照 D22 跟 P1 一起重過第 1 施工關），以及 daemon 接通已知啟動畫面的 `classify` 結果與送鍵。只認這三個已知畫面，不做未知提示的偵測。
- [x] 使用者確認（**A**；[確認紀錄](gate-12a-confirmations.md)）

## 開工時核實

P3、P4、P5 已選 A；選項描述只是比較，原 P3 B 不再是接入建議。P5 使用現有分類器增修資料，新樣式須附選定版本真 fixture。不得從歷史 F1–F6 或文件查核宣稱新版已通過。P4 只防手滑，不防同 uid 下刻意繞過 shim。已確認細節與授權範圍以 [D40](../decisions/d40.md)為準。

## 下一步

回到[施工關入口](gate-12-adapters.md)，整理選定行為的正反例及實作步驟。
