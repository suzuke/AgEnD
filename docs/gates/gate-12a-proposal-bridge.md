# 第 12A 提案：P1–P2 連線與設定

> **TL;DR**
> - claude channel、hooks 與設定檔；P1、P2 已確認，見 D40。
> - F1–F10 是 2026-09-28／2.1.283 的歷史實測，見[原紀錄](../research/gate-12a-claude-2026-09-28.md)。
> - 下一步：依已確認方向寫實作稿；本頁不是功能驗收結果。

## P1：claude 的訊息與 hook 怎麼到 daemon

- 問題：claude 的訊息從 channel（claude 自己起的 MCP server）進去，狀態從 hooks（claude 跑的指令）出來。這兩個程式是什麼？daemon 不在時怎麼辦？
- 已確認：兩個都是 `agend` 的內部子命令：`agend channel --instance <id>`（MCP stdio server）與 `agend hook <事件名>`。helper 經既有版本化 Unix socket client protocol 連 daemon，只有 daemon 開 DB。下一 minor 採 **1.5**，保留 1.3 一般請求與 1.4 完整終端能力；原建議請求 `channel_attach`、`channel_message`、`channel_written`、`hook_event` 與新增 ACK 的精確 schema 在實作稿寫定。
- 離線：hook 先持久化至 `$AGEND_HOME/spool/hooks/`，恢復後補送並等 daemon 確認入庫才刪；來源身分／排序／原子發布細節須寫入實作稿。Stop 回 `{}`，不 block、不取 queue。舊事件不建立當下 idle，恢復後核目前 session／畫面。ACK 的同類待送規則見 P7。
- hook timeout 採 10 秒；這是專用設定，不把官方預設當成實測結果。`ingest` 目前仍只有模組說明。
- 理由：同一個 binary、同一套有版本的協定（ARCHITECTURE 程序模型第 4 條）；hooks 丟掉就一直被當成 busy（V1-LESSONS #9）。
- 替代方案：bridge 另開私有協定（多一套要管版本）；hook 打 HTTP（daemon 多開 server）；daemon 不在時丟掉。
- 例子：`agend send g12-c "hi"` → bridge 寫 `notifications/claude/channel {content:"From: …\n\nhi", meta:{delivery_id:"m-7"}}` → `sent`。
- 關係：協定型別在 core（D11），所以本提案有兩處 core 改動：P1 的協定請求與 P5 的 `SCREEN_RULES` 資料（不加 pipeline 事件；D26 只加不改）。照 D22 先改 core，並「重過第 1 施工關」：`cargo test -p agend-core`、`cargo xtask check-deps`、`cargo xtask accept core` 全過，可重跑的 fake／native 檢查由 agent 與 fresh-context verifier 自動完成，提供指令讓使用者重驗；D22 的重新驗收與使用者確認仍必須完成。
- [x] 使用者確認（逐項及剩餘依建議；[確認紀錄](gate-12a-confirmations.md)、[D40](../decisions/d40.md)）

## P2：claude 讀哪些設定、我們的檔放哪

- 問題：hooks、`.mcp.json`、CLAUDE.md 寫在哪？要不要讀你自己 `~/.claude/settings.json` 裡的 hooks 與 `defaultMode: "auto"`？已完成第 10 施工關、登記成 `claude` 的假 agent 要不要也套？
- 建議：
  - 啟動帶 `--setting-sources project,local --settings $AGEND_HOME/claude/<id>/settings.json`：不讀個人 settings 的 hooks、預設模型與權限，沿用登入，不另設 CLAUDE_CONFIG_DIR。專用 hooks 與 `enabledMcpjsonServers: ["agend"]` 在 `--settings` 裡；管理政策仍適用，local 設定可能共用主 checkout（[CLI](https://code.claude.com/docs/en/cli-reference)、[設定來源](https://code.claude.com/docs/en/settings)）。F2 是 2.1.283 的歷史結果，現行版本須重驗。
  - `.mcp.json` 與 CLAUDE.md 放 workspace；不存在則建立並在 DB 記 sha256，只更新 AgEnD 擁有且雜湊未變的檔案。既有或被修改檔案不覆蓋，停止該 instance 啟動；沿用第 6 關啟動失敗路徑（5 秒後重試，10 分鐘內 3 次仍失敗則 `failed`），原因寫出是哪個檔。CLAUDE.md 同時加入來源說明與收到訊息先呼叫 agend_ack 的規則。
  - 只套在 `delivery = push` 的 claude；`delivery = inbox`（`fake-worker`）不寫檔、不加旗標、不建 driver。
- 理由：agent 的行為不隨你改自己的設定而變；錄製器也是這樣跑（RECORDER.md）。
- 未採用：載入個人 settings；另設 CLAUDE_CONFIG_DIR（會改變設定／登入資料位置，不能假定沿用登入）。
- 例子：`--dir ~/proj` 而 `~/proj/CLAUDE.md` 是你的 → `g12-c failed: CLAUDE.md exists and was not written by agend`。
- 關係：D16（CLAUDE.md 說明來源）照做；第 10 施工關的 `delivery = inbox` 不受影響。
- [x] 使用者確認（三項逐一同意；[確認紀錄](gate-12a-confirmations.md)）

## 與目前實作的接點

- `protocol/client.rs` 的一般協定最低需求仍是 1.3；完整終端另有 1.4 capability。新增 Claude 請求須保留兩者的 negotiation、舊 peer 行為與 schema/golden。
- 現有 `agend hooks install|uninstall` 是 git shim hook 管理，與提案的單數 `agend hook <事件名>` 不同；`agend channel`／Claude `hook` 尚未實作。
- daemon 停著時 Stop 回 `{}`，沒有取出 daemon 的佇列，不能宣稱此刻已送達；恢復後的補送與確認見 P7。

## 下一步

回到[施工關入口](gate-12-adapters.md)，依 [D40](../decisions/d40.md)整理 schema／spool 的實作稿。
