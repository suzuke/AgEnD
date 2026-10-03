# 第 12A 提案：P1–P2 連線與設定

> **TL;DR**
> - claude channel、hooks 與設定檔；全部待確認。
> - F1–F10 是 2026-09-28／2.1.283 的歷史實測，見[原紀錄](../research/gate-12a-claude-2026-09-28.md)。
> - 下一步：先逐項確認 P1、P2。

## P1：claude 的訊息與 hook 怎麼到 daemon

- 問題：claude 的訊息從 channel（claude 自己起的 MCP server）進去，狀態從 hooks（claude 跑的指令）出來。這兩個程式是什麼？daemon 不在時怎麼辦？
- 建議：兩個都是 `agend` 的內部子命令：`agend channel --instance <id>`（MCP stdio server）與 `agend hook <事件名>`。都走 client 協定的下一個 minor（本次 baseline 為 1.4，建議 **1.5**，仍待 P1 確認）的新請求（`channel_attach`、`channel_message`、`channel_written`、`hook_event`；1.2 是第 9 關、1.3 是第 10 關、1.4 是第 11 關完整終端）。daemon 不在時 hook 寫到 `$AGEND_HOME/spool/hooks/<instance>/<序號>.json`、Stop 回 `{}`（不 block），daemon 開機照序號補送（`ingest` 目前只有模組說明；spool 寫入與補送要在 A 段實作）。hook 的 timeout 設 10 秒（這是本提案的設定；[hooks 文件](https://code.claude.com/docs/en/hooks)的預設依事件而異，2026-10-03 文件查核不代表真 CLI 已重驗）。
- 理由：同一個 binary、同一套有版本的協定（ARCHITECTURE 程序模型第 4 條）；hooks 丟掉就一直被當成 busy（V1-LESSONS #9）。
- 替代方案：bridge 另開私有協定（多一套要管版本）；hook 打 HTTP（daemon 多開 server）；daemon 不在時丟掉。
- 例子：`agend send g12-c "hi"` → bridge 寫 `notifications/claude/channel {content:"From: …\n\nhi", meta:{delivery_id:"m-7"}}` → `sent`。
- 關係：協定型別在 core（D11），所以本提案有兩處 core 改動：P1 的協定請求與 P5 的 `SCREEN_RULES` 資料（不加 pipeline 事件；D26 只加不改）。照 D22 先改 core，並「重過第 1 施工關」：`cargo test -p agend-core`、`cargo xtask check-deps`、`cargo xtask accept core` 全過，可重跑的 fake／native 檢查由 agent 與 fresh-context verifier 自動完成，提供指令讓使用者重驗；D22 的重新驗收與使用者確認仍必須完成。
- [ ] 使用者確認

## P2：claude 讀哪些設定、我們的檔放哪

- 問題：hooks、`.mcp.json`、CLAUDE.md 寫在哪？要不要讀你自己 `~/.claude/settings.json` 裡的 hooks 與 `defaultMode: "auto"`？已完成第 10 施工關、登記成 `claude` 的假 agent 要不要也套？
- 建議：
  - 啟動帶 `--setting-sources project,local --settings $AGEND_HOME/claude/<id>/settings.json`：不讀你的使用者設定（你的 hooks、`defaultMode` 不會跑到 agent 身上），我們的 hooks 與 `enabledMcpjsonServers: ["agend"]` 在 `--settings` 裡（2026-09-28 的 F2 在 2.1.283 證實兩者一起用時 hooks 會跑、MCP 對話框不出現）。
  - `.mcp.json` 與 CLAUDE.md 一定要在工作目錄，寫進 workspace；DB 記它們的 sha256，檔案存在但雜湊不是我們記的 → 不覆蓋，這次啟動失敗；照已完成第 6 施工關的啟動失敗路徑（5 秒後重試，10 分鐘內 3 次仍失敗則 `failed`），原因寫出是哪個檔。
  - 只套在 `delivery = push` 的 claude；`delivery = inbox`（`fake-worker`）不寫檔、不加旗標、不建 driver。
- 理由：agent 的行為不隨你改自己的設定而變；錄製器也是這樣跑（RECORDER.md）。
- 替代方案：讀你的使用者設定（你的 hooks 每一輪都會在每個 agent 上跑；`defaultMode` 會變成 agent 的權限模式）；隔離的 `CLAUDE_CONFIG_DIR`（要重新登入）。
- 例子：`--dir ~/proj` 而 `~/proj/CLAUDE.md` 是你的 → `g12-c failed: CLAUDE.md exists and was not written by agend`。
- 關係：D16（CLAUDE.md 說明來源）照做；第 10 施工關的 `delivery = inbox` 不受影響。
- [ ] 使用者確認

## 與目前實作的接點

- `protocol/client.rs` 的一般協定最低需求仍是 1.3；完整終端另有 1.4 capability。新增 Claude 請求須保留兩者的 negotiation、舊 peer 行為與 schema/golden。
- 現有 `agend hooks install|uninstall` 是 git shim hook 管理，與提案的單數 `agend hook <事件名>` 不同；`agend channel`／Claude `hook` 尚未實作。
- daemon 停著時 Stop 回 `{}`，沒有取出 daemon 的佇列，不能宣稱此刻已送達；恢復後的補送與確認見 P7。

## 下一步

回到[施工關入口](gate-12-adapters.md)，先解釋 P1，等使用者決定。
