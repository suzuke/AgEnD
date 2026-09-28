# 第 12 施工關：其餘 adapter（`adapters`）

> **TL;DR**
> - claude driver、opencode driver、forge github、Telegram 通知；分四段做，各段自己驗收（P1）。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：開工前提案 P1–P21 等你逐題確認；確認後 merge 這份提案，等第 9–11 施工關完成再開工。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**提案中**（2026-09-28）：開工前提案 P1–P21 寫定，等你確認。依賴：第 7 施工關（送達、`messages` 表、`sh` 包裝、清掃）已驗收；第 9 施工關（CLI、`agend send`、`doctor`、協定 1.2）、第 10 施工關（流水線、forge local、「需要你」的來源、沙箱、協定 1.3）提案已確認、還沒實作；第 11 施工關 B 段進行中。這些施工關做完時跟這裡的理解不同，改這頁，不改它們。

## 範圍

- claude driver（D16）：channel bridge、hooks、啟動設定、三級忙碌、送達確認、事件、resume 與清掃（P2–P6）
- opencode driver：`opencode serve` 放進 holder、密碼、session、送達與事件（P7–P9）
- forge github：`gh`、submit、merge、main 前進、GitHub CI、清理（P10–P13）
- notifier：Telegram 設定、發什麼、收什麼、可靠性（P14–P18）
- 第 8 施工關移來的 G4 已讀狀態（P19）
- 其他施工關記給本關的事，以及 `agend doctor` 延到本關的幾列（P20）
- 什麼是假的、什麼是真的（P21）

### 從其他施工關帶來的筆記（已併進提案）

- **G4 已讀狀態**（[gate-08-client P6](gate-08-client.md#p6真-daemon-本關做哪些請求g4-移走)）→ P19。
- **claude Bash 工具的 PATH 重排**（[gate-03-shim 已知限制](gate-03-shim.md#已知限制)、第 7 施工關 P4、K8）→ P3、U5。
- **使用者提供、未查證：claude 按 ctrl+enter 可以把訊息「強送」給正在工作的 agent**（2026-09-26）→ P4、U2。官方文件有這個鍵，但說的是「你在 TUI 打字排隊的訊息」，對 channel 送來的訊息有沒有用沒寫。
- **holder 被 `kill -9` 後其他 backend 的清掃**（第 7 施工關「已知風險」，問排在本關還是另開）→ P6、P7。
- **第 10 施工關移來**：forge github、GitHub CI、逾時「通知」送 Telegram、卡住偵測／usage limit／自動選忙碌等級（建議移來本關）→ P10–P13、P20。
- **第 9 施工關移來**：`doctor` 的 backend、gh、Telegram 幾列；第一個讀 `config.toml` 的施工關負責建它 → P14、P20。

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子 · 跟既有決定的關係。跟既有決定不同的地方標「**與已確認／已追認的 X 不同，請明確決定**」。

文件已定、這裡不重問的：claude 用互動 TUI＋hooks＋channel、忙碌時用 Stop hook 排隊、中斷＝`Esc` 後立刻送（D16）；opencode 用 `serve` 的 HTTP＋SSE、SSE 沒有重播、授權以 `GET /permission` 為準（[backends/opencode.md](../backends/opencode.md)）；送達狀態四個、只有一套冪等、推送帶完整內容、不支援插入就改中斷（[delivery](../architecture/delivery.md)、`policy::busy::effective_level`）；附屬程序由 holder 持有、用 `sh` 包裝放在同一個 process group（第 7 施工關 P2）；daemon 先建 session、先存 DB 再讓 TUI 起來（第 7 施工關 P3 的做法）；forge 3 個方法、GitHub CI 用 `command` 關卡接（D29）；Telegram 一個「需要你」topic＋每個 team 一個（D13）；「需要你」的項目與 `resolve_attention` 只收操作者（第 8 施工關 P2、P5；第 10 施工關 P8）；daemon 的 git 經 `Runner`、`-c core.hooksPath=/dev/null`（第 10 施工關 P5）；checks 在寫入沙箱裡跑（第 10 施工關 P6）；真 CLI 一致性檢查是必要完成條件（使用者 2026-09-25）。

事實來源：[BACKEND-BEHAVIORS](../BACKEND-BEHAVIORS.md)、[backends/](../backends/claude-code.md)、`crates/agend-testkit/transcripts/{claude,opencode}/` 的錄製檔（claude 2.1.282、opencode 1.18.31）、官方文件（每條附 URL）。標 **未查證** 的集中在「未查證的事實」表（U1–U18）。本 agent 沒有跑任何真的 claude、opencode、gh 或 Telegram。

### P1：本關怎麼分段

- 問題：四樣東西彼此幾乎不相干，全部做完才驗收，一次要審的量太大。要不要分段？
- 建議：同一個施工關、分四段，**每段一個 PR、各自驗收**：A claude（P2–P6）、B opencode（P7–P9）、C forge github（P10–P13）、D Telegram（P14–P19）。P20、P21 跟著各段走。四段都驗收完，第 12 施工關才算完成。順序建議 A → B → C → D（D 要用到 C 的核准按鈕才有東西可按）。
- 理由：前面每關都要審 3–4 輪；分段後每輪只看一塊。第 13 關數目不變。
- 替代方案：拆成 12a–12d 四個施工關（ROADMAP 與 D22、D24 的「13 個施工關」要改）；不分段（一次審四樣）。
- 例子：A 段 PR `feat/gate-12a-claude` 驗收完 merge，頁面進度紀錄寫「A 段完成」，狀態維持「實作中」直到 D 段。
- 關係：D22（每關確認後才開下一關）不變，分段只在關內。
- [ ] 使用者確認

### P2：claude 的訊息與 hook 怎麼到 daemon

- 問題：claude 的訊息從 channel（claude 自己起的 MCP server）進去，狀態從 hooks（claude 跑的指令）出來。這兩個程式是什麼？怎麼連 daemon？daemon 不在時怎麼辦？
- 建議：
  - 兩個都是 `agend` 的內部子命令（不在 `--help`）：`agend channel --instance <id>`（MCP stdio server）、`agend hook <事件名>`（讀 stdin 的 JSON）。環境裡已有 `AGEND_INSTANCE`，照第 8 施工關 P2 當成那個 agent。
  - 都走 client 協定，加兩個 agent 限定的請求，是 **1.4**（1.2 第 9 關、1.3 第 10 關）：`channel_attach`（daemon 之後推 `channel_message {id, content, meta}`，bridge 寫進 claude 後回 `channel_written {id}`）、`hook_event {event, payload}`。Stop hook 另外要一個回覆：daemon 回「要不要 block、reason 是什麼」（P4）。
  - daemon 不在：`agend hook` 把事件寫到 `$AGEND_HOME/spool/hooks/<instance>/<序號>.json` 就結束（Stop hook 回 `{}`、不 block）；daemon 開機時照序號補送（`ingest` 模組開頭寫的做法）。`agend channel` 每秒重連，連上前沒有訊息可送，不影響 claude。
  - hook 的 timeout 在設定裡寫 10 秒（官方預設 600 秒，daemon 卡住時 claude 會等 10 分鐘）。
- 理由：同一個 binary、同一套有版本的協定（ARCHITECTURE 程序模型第 4 條：bridge 升級前跑舊 binary，協定必須向後相容）；hooks 丟掉就一直被當成 busy（V1-LESSONS #9）。
- 替代方案：bridge 另開一條私有 socket 協定（多一套要管版本的協定）；hook 用 `curl` 打 HTTP（daemon 要多開 HTTP server）；daemon 不在時 hook 直接丟（v1 的坑）。
- 例子：`agend send g12-c "hi"` → bridge 收到 `channel_message m-7` → 寫 `notifications/claude/channel {content:"From: …\n\nhi", meta:{delivery_id:"m-7"}}` → 回 `channel_written m-7` → `messages.state = sent`。
- 關係：D7（CLI 為主）不變，MCP 只用在 claude 要求的 channel；協定取 1.4。
- [ ] 使用者確認

### P3：claude 的啟動設定、權限、PATH

- 問題：hooks、channel、「訊息來源說明」寫在哪？三個啟動對話框怎麼過？claude 的權限要開多大？它的 Bash 工具找得到 shim 嗎？
- 建議：
  - **檔案位置**：hooks 放 daemon 的檔案，用 `--settings $AGEND_HOME/claude/<id>/settings.json` 帶進去（官方文件：`--settings` 優先於專案設定）；`.mcp.json` 與 `CLAUDE.md` 一定要在工作目錄（`--mcp-config` 會讓 `server:<name>` 找不到，backends/claude-code 陷阱），寫進 instance 的 workspace。檔案已存在、不是我們寫的（第一行沒有 `agend-managed` 標記）→ 不覆蓋，instance `failed`，原因寫出是哪個檔。
  - **旗標**：`--dangerously-load-development-channels server:agend`、`--permission-mode bypassPermissions`（每次都明確帶，陷阱：權限模式會黏著）、`--session-id`／`--resume`（第 6 施工關）。**用你自己的 `~/.claude` 設定**（不加 `--setting-sources`，跟 codex 用你的 `~/.codex` 一樣）；daemon 不寫 `~/.claude.json` 或 `~/.claude/` 裡任何檔。
  - **三個對話框**：信任資料夾、專案 MCP server、development channels 警告。先試「避免」：`settings.json` 放 `enabledMcpjsonServers: ["agend"]`（**未查證**能不能跳過 MCP 對話框，U3）。其他的用螢幕規則檔（[delivery](../architecture/delivery.md) 第 3 層：比對畫面 → 單一按鍵），每條附真畫面 fixture，fixture 在 `claude_live` 時錄。信任對話框預設游標在「No, exit」，規則必須明確選「Yes」那一項，不能按 Enter（陷阱 6）。
  - **權限**：`bypassPermissions` 對應 codex 的 `never`＋`danger-full-access`。`PermissionRequest` hook 若還是觸發 → 回 `deny`、log `permission denied (no handler yet)`（跟 codex 的 `decline` 一樣；轉給人回答不在本關）。
  - **PATH**：第 7 施工關 K8 的 `ZDOTDIR` **也給 claude**（先做，`claude_live` 驗）。**與已追認的第 7 施工關 K8「`ZDOTDIR` 只給 codex」不同，也再改一次第 6 施工關 H3 白名單，請明確決定**。claude 的 Bash 工具是不是 login shell 未查證（U5）。
- 理由：hooks 放 daemon 自己的目錄，不動你的 repo；MCP 與 CLAUDE.md 沒有別的放法。權限跟 codex 對齊，shim 是 git／pkill／killall 唯一的防護，所以 PATH 要先修再驗。
- 替代方案：隔離的 `CLAUDE_CONFIG_DIR`（spike 時要重新登入，BLOCKED）；預寫 `~/.claude.json` 的信任狀態（改你的檔案）；`default` 權限模式＋每次轉給人（本關沒有回答的人，agent 會卡住）；`--append-system-prompt` 代替 CLAUDE.md（spike F 沒測過這條，效果未查證）；ZDOTDIR 等 U5 查完再決定（先有一段時間 shim 可能被繞過）。
- 例子：`ps` 看到 `claude --dangerously-load-development-channels server:agend --permission-mode bypassPermissions --settings /tmp/g12…/claude/g12-c/settings.json --session-id 3f2a…`；workspace 裡有 `.mcp.json`、`CLAUDE.md`，第一行 `<!-- agend-managed -->`。
- 關係：D16（CLAUDE.md 說明來源）照做；第 7 施工關 K8、第 6 施工關 H3 見上。
- [ ] 使用者確認

### P4：claude 的三級忙碌與忙／閒

- 問題：queue、steer、interrupt 怎麼落到 claude？daemon 怎麼知道 claude 忙不忙？ctrl+enter 要不要用？
- 建議：
  - 忙／閒只看 hooks：`UserPromptSubmit` → 忙；`Stop`（沒被 block）、`SessionStart` → 閒；`Esc` 之後**沒有任何 hook**（錄製檔 `interrupt`），所以 daemon 自己送 `Esc` 時自己記成閒。
  - 閒置：一律經 channel 送。
  - `Queue`（忙碌）：訊息放 daemon 的佇列；下一個 `Stop`（`stop_hook_active: false`）時，daemon 把佇列**全部**取出、合成一個 reason 回 `{"decision":"block","reason":…}`（D16：先取出再回，`stop_hook_active: true` 的那次不再 block，防迴圈）。
  - `Steer`：claude 不支援 → core 改成 `Interrupt`（已寫在 `effective_level`）。
  - `Interrupt`：holder 送單一 `Esc` → 立刻經 channel 送（D16：不等 Stop）。
  - **ctrl+enter 本關不用**：官方文件說它是「立刻送出你排隊的訊息」（`chat:sendNow`），指的是在 TUI 打字排隊的；channel 的訊息算不算沒寫（U2）；有些終端機收不到 ctrl+enter，替代的 `Ctrl+X Ctrl+S` 是兩個鍵，違反「PTY 只送單一控制鍵」。U2 查到有用再提新的 P。
- 理由：官方文件與 2.1.282 錄製都說忙碌時 channel 訊息會自己排隊（U1），但你 2026-09-25 已確認維持 D16，假 claude 也照最壞情況做；Stop hook 是有錄製證據、3/3 的路。
- 替代方案：忙碌時直接經 channel 送、靠 claude 自己排（要推翻 D16）；中斷改用 ctrl+enter（U2 未查證）；每則排隊訊息各一個 Stop（多繞幾輪）。
- 例子：agent 在跑長 turn，`agend send --level queue g12-c "m-q"` → `queued`；turn 結束時 Stop hook 回 `block: From: g12-a …m-q` → 下一個 Stop 帶 `stop_hook_active: true` → `m-q confirmed`。
- 關係：D16 照做；D30 去抖動（轉 idle 穩定 5 秒）只給畫面用，driver 不經去抖動（第 7 施工關 P6 同理）。
- [ ] 使用者確認

### P5：claude 的送達確認、冪等、事件與 cursor

- 問題：`sent`、`confirmed` 各在什麼時候成立？`Driver::events(after_cursor)` 要補回 daemon 不在時的事件（DRV-6），claude 沒有 thread 歷史 API，事件從哪來？
- 建議：
  - `sent`：bridge 回 `channel_written`，或 Stop hook 的 block 回覆已交給 claude。
  - `confirmed`：channel 送的 → `UserPromptSubmit` 的 prompt 裡有 `delivery_id="<訊息 id>"`（錄製檔 `one_turn` 已證實 meta 會變成 `<channel … delivery_id="d1">`）；Stop hook 送的 → 下一個 `stop_hook_active: true` 的 Stop（錄製檔 `busy`：沒有 `UserPromptSubmit`，只有這個證據）。
  - 冪等照第 7 施工關 P5：`messages` 表查 id。daemon 在 `sent` 前後當掉：開機後**不重送**，停在原狀態（「不能確認就誠實標未確認」）；hook 事件補回後自然會確認。
  - 事件：新表 `driver_events`（instance、`seq`、種類、摘要、時間），hook 事件（含 spool 補送的）照到達順序寫入；`events(after)` 就是 `seq > after`；cursor＝`seq`。保留 14 天（跟事件一樣，D31），列進第 5 施工關 P8 的規則表。
  - migration：取開工時的下一個空號（目前是 `0005`，但第 9、10 施工關可能先用掉，取當時下一個空號）；同一個 migration 放本關所有 schema 變動（P8、P16、P18）。
- 理由：hook 是 claude 唯一的結構化事件來源，自己存一份最簡單；讀 claude 的 transcript 檔也行，但綁它的檔案格式（U7）。
- 替代方案：讀 `transcript_path` 的 jsonl 當事件日誌（像 codex 的 thread 歷史，綁格式）；開機時重送 `queued` 以外的訊息（可能重複一個 turn，違反 DRV-9）。
- 例子：daemon 停著時，排隊的訊息在 Stop hook 被 claude 處理（hook 寫進 spool）；daemon 開機補送 → `driver_events` 多兩列 → `m-q confirmed`。
- 關係：第 7 施工關 P5、P7 的規則照用；新表列進保留規則。
- [ ] 使用者確認

### P6：claude 的 session、resume、清掃

- 問題：第 6 施工關 H1 的已知限制：「`Spawn` 被確認」不等於 claude 已經把 session 存檔。沒送過任何訊息就死掉，`--resume <id>` 可能找不到。holder 被 `kill -9` 後 claude 或它的子程序可能留著（第 7 施工關問排在哪）。
- 建議：
  - `--resume <id>` 起來後幾秒就結束、畫面有「找不到 session」類的字（確切字樣未查證，U4），**而且**這個 instance 從沒有 `sent` 以上的訊息 → 同一個 id 改用 `--session-id` 再起一次、log `session <id> not found and never used; started again`。其他情況照第 6 施工關 P6 `failed`。**與已確認的第 6 施工關 P6「絕不自動全新啟動」不同，請明確決定**（跟你 2026-09-26 對 codex 核准的例外是同一個理由：空 session 沒有上下文可丟）。
  - 清掃**排在本關**：照第 7 施工關 P2 的條件，標記是 argv 裡 `--session-id`／`--resume` 後面那個元素完全等於這個 instance 的 session id，或 `agend channel --instance <id>` 的 `<id>` 完全相等。
- 理由：跟 codex 同一套規則，只換標記。
- 替代方案：找不到一律 `failed`（第一則訊息前死掉也要你處理）；清掃另開一個施工關（claude 死掉可能留孤兒）。
- 例子：假 claude 第一次起來就被 `kill -9` → 重起時 `--resume` 找不到 → `started again with --session-id 3f2a…`，不是 `failed`。
- 關係：第 6 施工關 P6、H1；第 7 施工關 P2、P3。
- [ ] 使用者確認

### P7：opencode 的程序、連線與密碼

- 問題：`opencode serve` 跟 TUI 怎麼放進 holder？daemon 怎麼知道 port？loopback 的 TCP port 誰都連得上，要不要擋？
- 建議：
  - 照第 7 施工關 P2 的 `sh` 包裝：背景 `opencode serve --hostname 127.0.0.1 --port 0`（輸出寫 log）→ daemon 從 log 讀出實際 port（錄製器的做法，真 CLI 對 `--port 0` 的行為**未查證**，U9）→ `GET /global/health` 就緒 → 建／接 session（P8）→ 寫 `$GO` → 包裝 `exec opencode attach http://127.0.0.1:<port> --session <id> --dir <workspace>`（旗標見官方 CLI 文件）。
  - **一定要密碼**：每個 instance 一個隨機密碼，存 `$AGEND_HOME/run/holders/<id>.opencode.pw`（0600），包裝讀出來設 `OPENCODE_SERVER_PASSWORD`（官方文件：serve 以 HTTP basic auth 保護、attach 預設讀同一個變數）。理由很具體：第 10 施工關的 checks 沙箱**允許 IP 網路**，沒有密碼的話 agent 寫的測試可以連上 loopback 的 opencode、在沙箱外叫它跑指令。
  - 這個變數會出現在 opencode 跑的指令的環境裡（那個 agent 本來就能用它自己的 server，接受）。**多一個變數是修改第 6 施工關 H3 白名單，請明確決定**。
  - 清掃標記：argv 裡 `attach` 的 URL 或 `--session` 後面那個元素。
- 理由：同一套包裝與清掃，不必新的程序管理；密碼擋住的是一條已知的沙箱繞過。
- 替代方案：unix socket（官方文件沒提到 serve 支援）；不設密碼（checks 可繞過沙箱）；密碼放 argv（`ps` 看得到）。
- 例子：`curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:<port>/session` → `401`；daemon log `g12-o: serve ready on 127.0.0.1:53412 (auth on)`。
- 關係：第 7 施工關 P2 照用；第 6 施工關 H3 見上；第 10 施工關 P6 的沙箱不改。
- [ ] 使用者確認

### P8：opencode 的 session、資料目錄、舊的 instance

- 問題：第 6 施工關說 opencode「沒有 session id，死了就 `failed`」。session 誰建、存哪？用哪個資料目錄？
- 建議：
  - daemon 在 serve 就緒後 `POST /session`，**先存 `instances.session_id` 再寫 `$GO`**（第 7 施工關 P3 的做法）。重起：同一個資料目錄、`attach --session <id>`（spike O4：硬殺後同 id 歷史都在）。
  - 資料目錄用你原本的（`~/.local/share/opencode`，不設 `XDG_*`），登入共用；daemon 不寫你的 opencode 設定。不加 `--pure`（你的 plugin 照常載入，跟 codex 用你的 MCP server 一樣）。
  - 第 12 關之前建的 opencode instance，`session_started = 1` 而 `session_id` 是 NULL → migration 時標 `failed`、`legacy_no_session`（比照第 7 施工關 P3 的 `legacy_no_thread`；可以直接把那一欄改名成通用的 `legacy_no_session`，開工時決定），holder 不動。`new` 的照常第一次啟動。第 8 施工關的重試表那一列跟著改：有 session id 就能 `retry`。
- 理由：跟 codex 完全同一條路，第 6 施工關的缺口一起補上。
- 替代方案：每個 instance 自己的 `XDG_DATA_HOME`＋複製 `auth.json`（登入過期要逐個處理）；讓 TUI 自己建 session 再去找（v1 的猜測）。
- 例子：`g12-o: session ses_4a… created`、`go (attach --session ses_4a…)`；硬殺 holder → `restart 1/3, session ses_4a… resumed`。
- 關係：第 6 施工關 H2；第 8 施工關重試表；第 7 施工關 P3。
- [ ] 使用者確認

### P9：opencode 的送達、忙碌、事件、權限、PATH

- 問題：三級怎麼對到 opencode？怎麼確認？SSE 不重播，事件從哪補？opencode 的權限與 PATH？
- 建議：
  - 閒置與 `Queue`：`prompt_async`（server 自己排，spike O3）。`Steer` → core 改 `Interrupt`。`Interrupt`：`POST /session/:id/abort` → 立刻 `prompt_async`（abort 後會 idle 兩次，錄製檔，第二次不當成新的一輪）。
  - 冪等：送出時帶 `messageID`（官方文件列在 prompt 的欄位裡），值由 daemon 產生並存在 `messages.turn_id`。opencode 對 id 格式有沒有要求**未查證**（U8；它自己的 id 長得像 `msg_…`）。`sent`＝回 204；`confirmed`＝`GET /session/:id/message` 裡出現這個 id 的 user message。
  - 事件：**opencode 的訊息歷史就是事件日誌**（第 7 施工關 P7 的做法）：`events(after)` 讀 `GET /session/:id/message`，每則展開成忙／確認／完成；cursor＝`<message id>:<part 序號>`。SSE 只拿來更新「現在忙不忙」與觸發一次讀取。
  - 權限：`OPENCODE_PERMISSION` 設全部 `allow`（對應 codex 的 never／full-access，**未查證**是否真的不再問，U12）；每 2 秒 `GET /permission`，還有被問的就 `reject`、log（跟 codex `decline` 一樣）。
  - PATH：`ZDOTDIR` 也給 opencode；它的 bash 工具用什麼 shell 未查證（U11），`opencode_live` 驗。
- 理由：跟 codex 同樣「以 backend 自己的歷史為準」，不必再存一份；`messageID` 讓確認不必比內容。
- 替代方案：確認改用「內容完全相同」比對（U8 不成立時的退路）；SSE 事件自己存表（SSE 會漏，spike O5 的 `permission.asked` 就漏過）。
- 例子：`m-9 busy → prompt_async (messageID msg_agend_m9…) → sent → confirmed`；daemon 停著時排隊的那則跑完，開機後從舊 cursor 讀歷史 → `m-9 confirmed`。
- 關係：delivery 三級表照做；第 7 施工關 P5、P7 的規則照用；ZDOTDIR 同 P3 的衝突標記。
- [ ] 使用者確認

### P10：forge github 怎麼接 GitHub、怎麼選

- 問題：用 `gh` 還是直接打 REST API？哪個 team 用 GitHub？
- 建議：
  - 一律用 `gh`（你已登入的那個），經 `Runner`、每次 60 秒 timeout（D28、V1-LESSONS #10）；呼叫都是 `gh api …` 或 `gh pr …`，repo 用 `gh repo view --json nameWithOwner` 從 canonical checkout 的 `origin` 讀一次。
  - 用哪個 forge 由 **workflow 的 submit 關卡**決定（core 已有 `forge = "github"` 欄位），merge 用同一個。本關不給 team 加欄位。
  - `agend doctor` 在有任何 workflow 用 github 時加一列：`gh auth status` 通過、`origin` 指到 GitHub（P20）。
- 理由：不存 token、不另寫 HTTP 與登入；`doctor` 本來就規定查 gh 登入（[tui-and-setup](../architecture/tui-and-setup.md#安裝與設定)）。
- 替代方案：daemon 用 HTTP client 打 REST、token 放 `config.toml`（多一份 secret）；team 設定 `forge`（workflow 已經有，兩份會不一致）。
- 例子：workflow `gh-demo` 的 `submit(forge = "github")` → daemon log `forge github: suzuke/agend-sandbox (gh 2.x, logged in)`。
- 關係：D4、D28、D29。
- [ ] 使用者確認

### P11：submit：push 與開 PR

- 問題：branch 誰 push、用什麼身分？PR 重複開怎麼辦？worktree 從哪個 main 開？
- 建議：
  - daemon 用自己的 git（第 10 施工關 P5：不經 shim、不跑 hook）`git push --force-with-lease=<branch>:<上次推的 SHA> origin agend/<task>/<slug>`；只推這個命名空間。第一次推用 `--force-with-lease=<branch>:`（遠端不能已有這個 branch）。
  - 身分：你自己的 git 憑證。第 10 施工關 P5 的 `env_clear` 會拿掉 `SSH_AUTH_SOCK`，SSH remote 會推不上去：push 這一種呼叫額外保留 `SSH_AUTH_SOCK`（HTTPS remote 走 `gh auth setup-git` 的 credential helper）。**未查證**你的 remote 是哪一種、這樣夠不夠（U14）。
  - 開 PR：先 `gh pr list --head <branch> --state all --json number`，有就沿用；沒有才 `gh pr create --base main --head <branch> --title <標題> --body <內容＋Agend-Task: <task>>`。change id＝PR 編號（`{pr}` 從這裡來）。
  - **worktree 的起點**：github 的 team 綁定時先 `git fetch origin main`，從 `origin/main` 開（不是本機 main）。**與已確認的第 10 施工關 P4 第 2 步「從 main 的 SHA 建」不同，請明確決定**：github 的 merge 發生在遠端，本機 main 不會前進。
- 理由：只推自己命名空間、用 lease，不會蓋掉別人的 push；PR 先查再開，重做（P9 對帳）不會多開一個。
- 替代方案：用 `gh` 的 token 推 HTTPS（不管 remote 設定）；本機 main 也跟著 fast-forward（要處理 checkout 著的 main，第 10 施工關 P7 那一整套）。
- 例子：`t-5 submit: pushed agend/t-5/hello (a1b2c3d); PR #42 opened`；daemon 在開 PR 前被硬殺 → 開機 `PR for agend/t-5/hello not found; opened #42`，不會有 #43。
- 關係：第 10 施工關 P4、P5 見上。
- [ ] 使用者確認

### P12：merge 與 main 前進

- 問題：怎麼保證 GitHub merge 的就是核准的那個 head？main 在 GitHub 上前進了怎麼辦？怎麼知道已經 merge 過？
- 建議：
  - merge：`gh api -X PUT repos/<o>/<r>/pulls/<n>/merge -f sha=<核准的 head> -f merge_method=merge -f commit_message="Agend-Task: <task>"`。官方文件：`sha`＝「PR head 必須等於它才准 merge」、不符回 **409**。409 → 讀 PR 的 head → `HeadChanged{actual_head}`（FRG-6）；**405**（不能 merge：branch protection、要人 review、只准 squash…）→ `MergeFailed`，出現「需要你」`merge-blocked:<task>`，寫出 GitHub 的訊息。
  - 一律 merge commit，不 squash（V1-LESSONS #6：squash 讓「從 git 推論」失效）；repo 只准 squash 就是上面的 405。
  - main 前進（D14）：送 merge 前 `git fetch origin main`，head 不含 `origin/main` → 照第 10 施工關 P7 在持有者的 worktree rebase、`--force-with-lease` 推、比 patch-id。
  - 「已經 merge 了嗎」（開機對帳、送 merge 前）：`gh api repos/<o>/<r>/pulls/<n>` 的 `merged` 與 `merge_commit_sha`。不找 trailer（第 10 施工關 P7 說 GitHub 在本關另外處理）。
  - merge 後刪遠端 branch；task 取消或失敗 → 關 PR、刪遠端 branch（v1 的 137 個 branch）。
- 理由：GitHub 自己比對 SHA，是跟 forge local 的 CAS 一樣強的保證；PR 狀態是 GitHub 的真相，不必推論。
- 替代方案：merge 前自己讀 head 再 merge（中間有競態）；跟 repo 設定走 squash（main 上找不到 head）；PR 留著不關。
- 例子：你核准後、merge 前有人往 PR 推了一個 commit → log `t-5 merge: GitHub 409 (head is 9f8e…, approved a1b2…); checks run again`，沒有 merge。
- 關係：D14、D29、FRG-5..8；第 10 施工關 P7、P8（`merge-blocked`）。
- [ ] 使用者確認

### P13：GitHub CI 怎麼等

- 問題：D29 定了用 `command` 關卡跑 `gh pr checks {pr} --watch`。但第 10 施工關的 checks 在沙箱裡跑，`gh` 在沙箱裡拿得到登入嗎？
- 建議：照 D29 不變，workflow 寫 `gh pr checks {pr} --watch`，`timeout_ms` 寫明（例如 30 分鐘）。沙箱不動：`gh` 只讀 `~/.config/gh`（沙箱允許讀）、走 HTTPS（允許）；macOS 上 token 可能在 keychain，沙箱裡讀不讀得到**未查證**（U13）。讀不到時再提新的 P，**不**把 token 放進 checks 的環境（checks 跑的是 agent 寫的程式碼，拿到 token 就能改你的 GitHub）。
- 理由：先用已確認的做法，只有一個未查證點；拿不到 token 會 fail closed（checks 失敗、退回 work），不會假綠。
- 替代方案：加一種 daemon 自己在沙箱外跑的「等 CI」關卡（要改 D29 與 core 的關卡種類）；`GH_TOKEN` 放進 checks 環境（安全問題見上）。
- 例子：`checks t-5/ci/1: gh pr checks '42' --watch → exit 0 (3 checks passed)`。
- 關係：D29、第 10 施工關 P6。
- [ ] 使用者確認

### P14：Telegram 的設定與 secret

- 問題：bot token、群組、誰可以操作，寫在哪？第 13 關才有 `agend telegram setup`，這關怎麼設？
- 建議：
  - 本關建 `config.toml`（第 9 施工關 P9：第一個讀它的施工關負責建）。你手寫：

    ```toml
    [telegram]
    chat_id = -1001234567890          # 開了 topic 的 supergroup
    allowed_user_ids = [123456789]    # 誰可以按按鈕、回答
    token_file = "secrets/telegram.token"   # 相對 AGEND_HOME；檔案必須 0600
    ```

  - token 不寫在 `config.toml` 裡，放獨立的 0600 檔。**與 [tui-and-setup](../architecture/tui-and-setup.md#設定與目錄d8)「config.toml 放 Telegram 等連線設定」字面不同，請明確決定**（連線設定在，只有 token 分開）。
  - 沒有 `[telegram]` → notifier 關閉，`doctor` 那一列是 `skip`。有 `[telegram]` 但 `allowed_user_ids` 空 → daemon **不收**任何訊息、每次開機 log 一行 error，`doctor` 失敗 exit 1（v1 #2207）。
  - 第 13 關的 `agend telegram setup` 之後只是幫你寫這三個值。
- 理由：token 分開，`config.toml` 可以貼給別人看；空 allowlist 大聲失敗，不再靜靜丟訊息。
- 替代方案：token 放 `config.toml`（貼設定時容易外流）；環境變數（v1 #2005 變數名不一致）；本關就做 `telegram setup`（第 13 關的範圍）。
- 例子：`allowed_user_ids = []` → `agend doctor` → `FAIL telegram: allowed_user_ids is empty; every message would be dropped. Fix: add your user id to [telegram] allowed_user_ids in <home>/config.toml`，`exit=1`。
- 關係：D13；第 9 施工關 P8、P9；第 13 施工關 `telegram setup`。
- [ ] 使用者確認

### P15：新的依賴：HTTP client

- 問題：Telegram 要 HTTPS；opencode 要 HTTP＋SSE。用什麼？跑在哪？
- 建議：`agend-daemon` 加一個**阻塞式** HTTP client（建議 `ureq`＋rustls，不用 openssl）。Telegram 的 long poll 與每個 opencode 的 SSE 連線各一條自己的 std thread（第 6 施工關 H7、第 7 施工關用 `tungstenite` 的同一個做法），不碰 tokio runtime 的巢狀 `block_on`（v1 #1476）。`check-deps` 加一條：`agend-client`、`agend-shim` 不能依賴它。
- 理由：一個 crate 同時滿足兩邊；阻塞式跟第 7 施工關一致，少一個 async 依賴。
- 替代方案：`reqwest`（async，帶進更多依賴）；自己寫 HTTP（testkit 的 `http.rs` 只給假的用，不做 TLS）；`teloxide`（v1 用的，runtime 問題的來源）。
- 例子：`cargo tree -p agend-daemon | grep ureq` 有；`cargo xtask check-deps` 多一條規則仍 `ok`。
- 關係：D10、D11；ARCHITECTURE「外部指令一律帶 timeout」。
- [ ] 使用者確認

### P16：Telegram 發什麼、發到哪

- 問題：哪些事會吵你的手機？topic 誰建？內容太長怎麼辦？
- 建議：
  - 「需要你」topic：每個「需要你」項目一則訊息，按鈕就是它的 `actions`（`approve`、`request_changes`、`retry`…）。按鈕的 `callback_data` 上限 64 bytes（官方文件），所以存一個短代號，對照表在 DB。
  - team topic：只發結果摘要：task merged、failed、cancelled，與逾時動作「通知」（第 10 施工關記給本關）。Info 等級不發。個別 instance topic 不做（D13：可選、非預設）。
  - topic 第一次要用時才 `createForumTopic`，id 存 DB；bot 要有管理 topic 的權限（**未查證**確切權限名，U16）。
  - 內容：純文字、不用 `parse_mode`（NTF-3 保留空白）；超過 4096 字（官方上限）就依序切成多則（NTF-2 不截斷、NTF-4 照順序）。
  - 在 TUI 解決了 → 把 Telegram 那則改成「已解決（TUI）」並拿掉按鈕，手機上不會留著一個還能按的舊按鈕。
- 理由：只有例外會叫你（ARCHITECTURE）；按鈕跟 TUI 走同一條 `resolve_attention`。
- 替代方案：每個事件都發（吵）；啟動時一次建好所有 topic（team 以後加的還是要再建）；長內容傳成檔案（手機上要多點一次）。
- 例子：手機「需要你」topic：`approval t-5/approve/1 — approve agend/t-5/hello at a1b2c3d? [approve] [request changes]`。
- 關係：D13、D35、D37；NTF-1..4；第 10 施工關 P8。
- [ ] 使用者確認

### P17：Telegram 收什麼、誰能按

- 問題：手機上按按鈕、回覆文字，daemon 怎麼收？怎麼防別人？
- 建議：
  - `getUpdates` long poll（沒有對外 port，本機可用），`offset` 存 DB，重開機不重收。設了 webhook 時 `getUpdates` 不能用（官方文件）→ log＋`doctor` 失敗。
  - 只接受 `allowed_user_ids` 裡的人；其他人 → 忽略、每個人只 log 一次。
  - 按鈕 → 以**操作者**身分 `resolve_attention`（同一條路，核准一樣綁 head）。`request_changes` 要理由：bot 回「請回覆這則訊息寫理由」，你回覆的文字當 `note`。
  - 回覆請示那則訊息（reply）→ `answer_ask` 自由文字（D35）；選項用按鈕。
  - 同一個 bot 只能一個 daemon 收（兩個會互搶更新；Telegram 回什麼錯誤**未查證**）；跟 v1 並行那一週要用另一個 bot（ROADMAP 已寫）。
- 理由：跟 TUI 同一條權限與同一條解決路徑，不另寫一套核准邏輯。
- 替代方案：webhook（要對外開 port）；群組裡任何人都能按（誰都能核准 merge）。
- 例子：你在手機按 `approve` → watch 印 `attention_resolved approval:t-5/approve/1 by telegram:123456789` → merge。
- 關係：第 8 施工關 P2（操作者權限）、D35。
- [ ] 使用者確認

### P18：通知的可靠性

- 問題：daemon 當掉、Telegram 暫時連不上、被限流，通知會不會掉、會不會重複？
- 建議：通知先寫 DB 的 outbox（`notifications`：id、項目、狀態、Telegram message id），再送；送成功記 message id。開機時補送沒送成功的。429 照回覆的 `retry_after` 等（欄位名**未完整查證**，見 U18）。送出與記錄之間當掉 → 可能重送一次（Bot API 沒有冪等 key），接受並寫進已知風險。保留 30 天（同訊息，D31）。
- 理由：手機是例外的最後一道通知，不能靜靜掉；重複一則比漏一則好。
- 替代方案：只存記憶體（daemon 重啟就掉）；每次送前查 Telegram 有沒有（Bot API 不能查自己發過的訊息）。
- 例子：Telegram 連不上 10 分鐘 → log `telegram: 3 notifications waiting (network)`；恢復後依序送出，沒有漏。
- 關係：D31（新表的保留期限）；NTF-4。
- [ ] 使用者確認

### P19：G4 已讀狀態

- 問題：第 8 施工關把 G4（daemon 記已讀、TUI 與 Telegram 共用）移來本關，理由是「要跟 Telegram 共用」。可是 Telegram 能告訴我們你讀了嗎？
- 建議：**不做 G4**，TUI 維持本機已讀（第 11 施工關 T4、T17）。Bot API 沒有「使用者讀了哪則訊息」的資料（官方 API 文件裡找不到這類欄位，**未完整查證**，U17），所以 Telegram 只能提供「按了、回了」＝已解決，不是已讀，共用的唯一理由不成立。**與已追認的第 11 施工關 G4（daemon 記已讀、與 Telegram 共用）與第 8 施工關 P6 不同，請明確決定**。
- 理由：做一張表、一個協定版本，卻沒有第二個會寫已讀的來源。
- 替代方案：照 G4 做：daemon 存已讀、TUI 寫、Telegram 只讀（多開幾台 TUI 時已讀會同步；要協定 1.4 多一組請求與事件）。
- 例子：你在 TUI 展開一項 → 那項不再粗體；手機上同一項照樣顯示，按了才消失。
- 關係：第 8 施工關 P6、第 11 施工關 G4、T4、T17。
- [ ] 使用者確認

### P20：其他施工關記給本關的事

- 問題：前面幾關把幾件事「記給第 12 施工關」。哪些做、哪些不做？
- 建議：

  | 事 | 從哪來 | 建議 |
  |---|---|---|
  | `doctor`：各 backend 安裝／版本／登入、gh 登入、Telegram | 第 9 施工關 P8 | **做**。版本範圍＝錄製檔 header 的版本（「測過的版本」），不同就 `warn` 不 `fail`；登入只查不花 token 的（`claude`、`opencode` 怎麼查**未查證**，開工時看 `--help`） |
  | 逾時動作「通知」送 Telegram | 第 10 施工關 | **做**（P16 team topic） |
  | holder 死後的清掃：claude、opencode | 第 7 施工關 | **做**（P6、P7） |
  | 卡住偵測、usage limit、依緊急程度自動選忙碌等級 | 第 10 施工關「建議移到第 12 施工關」 | **不做**，移到之後另排（v2.0 之後或一個新的施工關）。**與已確認的第 10 施工關「本關不做」清單裡的「建議移到第 12 施工關」不同，請明確決定** |
  | 你自己的 codex MCP server／plugin 在每個 agent 生效 | 第 7 施工關已知風險 | **不做**，維持記錄（claude、opencode 也一樣用你的設定，P3、P8） |
  | codex approval 轉給人回答 | 第 10 施工關建議給第 11 施工關 | 不在本關 |

- 理由：本關已有四大塊；卡住與額度偵測要真 backend 的長時間資料，現在做只能猜規則（V1-LESSONS #3：每次 CLI 改版就要重新校準）。
- 替代方案：本關連卡住與 usage limit 一起做（每個 backend 要螢幕規則與 fixture，範圍大約加一倍）。
- 例子：`agend doctor` 多四列：`claude 2.1.282 (tested)`、`opencode 1.18.31 (tested)`、`gh: logged in as suzuke`、`telegram: ok (chat -100…, 1 allowed user)`。
- 關係：第 7、9、10 施工關；D24（`doctor` 每個檢查都有故意弄壞的測試，第 13 關補齊）。
- [ ] 使用者確認

### P21：什麼是假的、什麼是真的

- 問題：CI 不能跑真的 claude、opencode、GitHub、Telegram。各自對什麼測？真的誰跑、花多少？
- 建議：
  - **假的**（CI）：`fake-claude`、`fake-opencode-serve`（已有，補密碼、`messageID`、`POST /session` 的形狀）；新的 `fake-gh`（testkit bin：`pr list/create/view/checks`、`api …/merge`、`repo view`，對一個本機 bare repo 當 origin；409／405 可編排）；新的 `fake-telegram`（HTTP 假 Bot API，`api_base` 只在測試用的環境變數裡能改）。契約：DRV 對 claude、opencode driver；FRG 對 github forge＋`fake-gh`；NTF 對 Telegram notifier＋`fake-telegram`。DRV-6、DRV-9 用四次開機。
  - **錄製**（你核准才跑、花少量 token）：claude 加 `resume_empty`（U4）、`startup_dialogs`（對話框畫面 fixture，U3）；opencode 加 `message_id`（U8）、`password`（U9、U10），並用新版 CLI 重錄既有 5 個。
  - **真的、選做**：`claude_live`、`opencode_live`（`agend-daemon` 的 example，`AGEND_REAL_CLAUDE=1`／`AGEND_REAL_OPENCODE=1` 才跑、在 `record-sandbox.sh` 裡跑；各約 3 個短 turn：記一個詞、`command -v git pkill killall`、`kill -9` holder 後問那個詞）。GitHub 用你自己的 sandbox repo；Telegram 用你自己的 bot 與群組。
  - 本 agent 與 verifier **都不跑**真的 claude、opencode、gh、Telegram。
- 理由：跟第 7 施工關 P8 一樣：邏輯全部在 CI 驗，只有「真 CLI 接不接受」要真跑。
- 替代方案：CI 用真的 GitHub（要 token、會留下 PR）；不做 `fake-gh`，forge github 只靠真跑（CI 驗不到 409／405）。
- 例子：`cargo test -p agend-daemon` → `contract Forge: github+fake-gh 10/10 pass`、`contract Notifier: telegram+fake-telegram 4/4 pass`。
- 關係：D9；第 2 施工關契約；第 7 施工關 P8；使用者 2026-09-25 的一致性檢查決定。
- [ ] 使用者確認

### 本關不做（明確列出）

- 卡住偵測、usage limit、自動選忙碌等級（P20）。
- codex、claude、opencode 的授權請求轉給人回答；本關一律拒絕並記 log（P3、P9）。claude 的 channel permission relay（官方文件的 `claude/channel/permission`）之後再看。
- Telegram 個別 instance topic（D13 非預設）、`agend telegram setup`（第 13 施工關）。
- `ctrl+enter` 強送（P4、U2）。
- 改派、臨時 instance、角色範本（第 10 施工關 P3 已列）。

### 已知風險（開工時處理）

- 依賴第 9、10、11 施工關的實作：協定號（1.4）、migration 號（取當時下一個空號）、`agend send`／`doctor`／`task create` 的確切輸出，開工時照實際的改這頁。
- **claude 用 development channel**：官方標為 research preview，旗標名與對話框可能隨版本改（錄製已經看到 2.1.282 比 spike 多一個對話框）。driver 啟動時 log `claude --version`，版本不同先跑一致性檢查。
- claude 的 hooks 用你的 `~/.claude` 設定時，你自己的 hooks 也會跑（每次 Stop 可能兩個 hook，陷阱）；你的 hook 很慢會拖慢每一輪。
- opencode 的密碼在 agent 的環境裡（P7），agent 自己印得出來；接受（同一個 agent 的 server）。
- GitHub 的 branch protection、必要的 review、只准 squash 都會讓 merge 405；本關只把它變成「需要你」，不嘗試配合 repo 設定。
- Telegram 送出與記錄之間當掉會重送一則（P18）。
- `sandbox-exec` 裡讀不到 keychain 的話，GitHub CI 關卡會一直失敗（P13、U13）；開工第一件事先查。

**未查證的事實**（本 agent 沒有跑任何真 CLI；「怎麼查」裡不花 token 的你可以自己跑，其餘等你核准錄製或 `*_live`）：

| # | 事實 | 影響 | 怎麼查 |
|---|---|---|---|
| U1 | claude 忙碌時送到的 channel 訊息會自己排隊，這輪結束後處理 | P4（不改 D16，只是證據） | **部分查證**：官方 [channels reference](https://code.claude.com/docs/en/channels-reference)「Events queue into the session and are processed in order」；2.1.282 錄製一次（RECORDER.md） |
| U2 | `ctrl+enter`（`chat:sendNow`）對 channel 送來、正在排隊的訊息有沒有作用；PTY 送得出 ctrl+enter 嗎 | P4 | 官方 [interactive mode](https://code.claude.com/docs/en/interactive-mode) 只寫「你排隊的訊息」、需要終端機回報 extended keys、v2.1.275 起。真跑：`claude_live` 加一段（忙碌時 channel 送一則 → 送 ctrl+enter → 看 `UserPromptSubmit` 何時出現） |
| U3 | `settings.json` 的 `enabledMcpjsonServers` 能不能跳過「專案 MCP server」對話框；三個對話框的確切畫面與要按的單鍵 | P3 | 錄製情境 `startup_dialogs`（`agend-record startup-check claude` 已能只啟動、回答、`/exit`，不送 prompt） |
| U4 | 起來後沒送過訊息就被殺，`--resume <id>` 會怎樣（錯誤字樣、exit） | P6 | 錄製情境 `resume_empty`（不送 prompt，不花 token） |
| U5 | claude 的 Bash 工具是不是 login shell、PATH 會不會被 `path_helper` 重排、shell 快照會不會蓋掉 | P3 | `claude_live` 的 `command -v git pkill killall` |
| U6 | `--permission-mode bypassPermissions` 在互動模式第一次用會不會多一個確認對話框 | P3 | 同 U3 的 `startup-check` 加這個旗標 |
| U7 | `transcript_path` 的 jsonl 格式（只在選了替代方案時需要） | P5 替代方案 | 讀一個錄製時留下的 transcript |
| U8 | opencode 的 `messageID` 格式要求（我們自己給的 id 收不收） | P9 | 官方 [server 文件](https://opencode.ai/docs/server/)列了欄位、沒寫格式；錄製情境 `message_id` |
| U9 | 真 `opencode serve --port 0` 會印出實際 port（錄製器 2026-09-27 改成 `--port 0` 之後沒有重錄真的） | P7 | `opencode serve --hostname 127.0.0.1 --port 0` 看第一行（不花 token） |
| U10 | `OPENCODE_SERVER_PASSWORD` 設了之後 `attach` 用同一個變數連得上；serve 死了 TUI 會不會自己結束 | P7 | 官方 [CLI 文件](https://opencode.ai/docs/cli/)寫 `--password` 預設讀這個變數；真跑 `opencode_live` |
| U11 | opencode 的 bash 工具用什麼 shell、shim 在不在第一個 | P9 | `opencode_live` 的 `command -v git pkill killall` |
| U12 | `OPENCODE_PERMISSION` 全部 `allow` 之後 `GET /permission` 是否永遠空 | P9 | 錄製情境 `approval` 在這個設定下重錄 |
| U13 | macOS `sandbox-exec`（第 10 施工關的設定檔）裡 `gh` 讀不讀得到 keychain 的 token | P13 | 第 10 施工關做完後，在沙箱裡跑 `gh auth status`（不改任何東西） |
| U14 | 你的 sandbox repo 的 remote 是 SSH 還是 HTTPS；保留 `SSH_AUTH_SOCK` 夠不夠 push | P11 | `git -C <repo> remote -v` |
| U15 | GitHub merge API 帶 `sha` 不符回 409、不能 merge 回 405 | P12 | **已查證（官方文件）**：[Merge a pull request](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request) |
| U16 | `createForumTopic` 要 bot 有哪個管理員權限 | P16 | [Bot API](https://core.telegram.org/bots/api#createforumtopic)（本 agent 讀到的頁面被截斷，沒讀到這段） |
| U17 | Bot API 完全沒有「已讀」資訊 | P19 | [Bot API](https://core.telegram.org/bots/api) 讀到的部分沒有這類欄位；頁面被截斷，未完整查證 |
| U18 | 訊息上限 4096 字、`callback_data` 1–64 bytes；429 的 `retry_after` 欄位 | P16、P18 | **前兩個已查證**（[sendMessage](https://core.telegram.org/bots/api#sendmessage)）；`retry_after` 沒讀到 |

## 自動驗收（完成定義）

每段（P1）做完就跑自己那幾項；四段都過才算本關完成。

- [ ] `~/.cargo/bin/cargo test -p agend-daemon`、`~/.cargo/bin/cargo test -p agend-testkit`、`~/.cargo/bin/cargo test -p agend` 單獨通過，包括：claude driver 與 opencode driver 對假 agent 跑 DRV-1..9（DRV-6、DRV-9 四次開機）；hook 在 daemon 停著時寫 spool、開機補送、Stop hook 不 block；Stop hook 一次取出全部佇列、`stop_hook_active: true` 不再 block；`Esc` 後立刻經 channel 送；已存在、不是我們寫的 `.mcp.json`／`CLAUDE.md` → `failed`；`--resume` 找不到且沒送過訊息 → 用 `--session-id` 再起一次，送過 → `failed`；opencode 沒密碼的請求回 401；清掃對 claude／opencode 的標記只殺自己的 group；forge github 對 `fake-gh` 跑 FRG-1..10，409 → `HeadChanged`、405 → `merge-blocked`、PR 不重開、開機時用 PR 狀態判斷已 merge；notifier 對 `fake-telegram` 跑 NTF-1..4，超過 4096 字切段、空 allowlist 不收、非 allowlist 的人被忽略、outbox 補送；migration 的 schema fixture 與 golden 更新
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過），含 P15 的新規則
- [ ] `~/.cargo/bin/cargo xtask accept adapters` 通過，並印出下方「你親自驗收」步驟 1 的 demo
- [ ] 真 CLI 一致性檢查（必要；使用者已決定 2026-09-25）：`claude --version`、`opencode --version` 和 `crates/agend-testkit/transcripts/{claude,opencode}/` 錄製檔 header 的 `version` 相同，不同就先用錄製器重錄（[RECORDER.md](../../crates/agend-testkit/RECORDER.md#重錄cli-升版時)）；`~/.cargo/bin/cargo test -p agend-testkit --test conformance` 通過
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新；[BACKEND-BEHAVIORS](../BACKEND-BEHAVIORS.md) 與 backends 分頁照 U 的結果更新
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出；`<t-N>` 這類尖括號是會變的值。步驟 3、4、5 跑真的 backend、會花少量 token，步驟 6、7 用你的 GitHub sandbox repo，步驟 8–10 用你的 Telegram bot；每一段都在那一段（P1）驗收時做。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.x.y`（目前是 `agend 0.0.0`）。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

`AGEND_HOME` 一定要設（第 13 施工關之前沒有預設值），沒設的話 `agend` 會拒絕執行。步驟 1、2 自己建暫存 home。步驟 5 起用同一個暫存 home：每個步驟的指令第一行都是 `export AGEND_HOME=<home>`，把 `<home>` 換成步驟 5 印出的路徑（同一個分頁設過一次就好，新分頁要再設）。

1. 跑 demo（全部對假的）。

   **這步在驗什麼**：四段在假 agent、`fake-gh`、`fake-telegram` 上都走得通：claude 與 opencode 的三級忙碌與確認、四次開機補回事件、github 的 409／405、Telegram 切段與空 allowlist。錯了代表後面真的步驟看到的都不可信。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo xtask accept adapters
   ```

   應該看到：依序 `== claude`、`== opencode`、`== github`、`== telegram`、`== restart`，倒數第二行 `adapters demo: all sections passed`，最後一行 `gate 12 (adapters): checks passed`（確切輸出開工時細化）。

   - [ ] 通過

2. 真 CLI 一致性檢查（必做；使用者已決定 2026-09-25）。

   **這步在驗什麼**：driver 測試用的假 claude／假 opencode 和你機器上真的 CLI 形狀一致。壞了的話，driver 對假的全綠、接上真的才出錯（v1 #1483）。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   claude --version; opencode --version
   head -1 crates/agend-testkit/transcripts/claude/one_turn.jsonl crates/agend-testkit/transcripts/opencode/one_turn.jsonl
   ~/.cargo/bin/cargo test -p agend-testkit --test conformance
   ```

   應該看到：兩個版本各自和錄製檔 header 的 `"version"` 相同；最後 `test result: ok.`（通過數開工時細化）。版本不同：先重錄那個 backend（`~/.cargo/bin/cargo xtask record claude --sandbox ~/Documents/Hack/AgEnD-ops/record-sandbox.sh`，或 `opencode`；會跑真的 CLI、花少量 token，見 [RECORDER.md](../../crates/agend-testkit/RECORDER.md)）再跑一次；檢查不過就改假 agent，不改錄製檔。

   - [ ] 通過

3. 真 claude 端到端（`claude_live`，花約 3 個短 turn）。

   **這步在驗什麼**：假的驗不到的事：三個啟動對話框都被規則處理掉（U3、U6）；channel 訊息被確認；claude 跑的 `git`／`pkill`／`killall` 是 shim（U5）；holder 被 `kill -9` 後沒有 claude 留下、重起後上下文還在；你的 `~/.claude.json` 沒被改。錯了的話，真 claude 會卡在對話框，或 agent 的 git 繞過 shim。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ls -l ~/.claude.json
   ~/.cargo/bin/cargo build -q -p agend --bin agend && ~/.cargo/bin/cargo build -q -p agend-daemon --example claude_live
   AGEND_REAL_CLAUDE=1 ~/Documents/Hack/AgEnD-ops/record-sandbox.sh target/debug/examples/claude_live 2>/tmp/g12-claude.log | tee /tmp/g12-claude.out
   ls -l ~/.claude.json
   pgrep -fl "agend (holder|daemon|channel)"
   ```

   應該看到（開工時細化）：`startup dialogs handled: trust, mcp, channels`、`m-1 idle → channel → confirmed`、三行 `git: <H>/bin/git (the shim)`…、`kill -9 holder` 之後 `claude left: []`、`restart 1/3, session <S> resumed`、`m-2 → confirmed; reply mentions m-1's word: true`、最後 `claude_live: ok`；前後兩次 `ls -l` 的修改時間相同（沙箱擋寫入時例外，開工時確認 `record-sandbox.sh` 是否允許）；`pgrep` 沒有輸出。

   - [ ] 通過
   - [ ] 這次不做（寫進驗收紀錄）

4. 真 opencode 端到端（`opencode_live`，花約 3 個短 turn，用免費模型）。

   **這步在驗什麼**：真 opencode 接受 `--port 0`、密碼、`attach --session`（U9、U10）；`messageID` 收得下、確認得到（U8）；bash 工具找到的是 shim（U11）；沒有密碼連不上。錯了的話，driver 在真 opencode 上確認不了訊息，或 checks 可以繞過沙箱叫它跑指令。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo build -q -p agend --bin agend && ~/.cargo/bin/cargo build -q -p agend-daemon --example opencode_live
   AGEND_REAL_OPENCODE=1 ~/Documents/Hack/AgEnD-ops/record-sandbox.sh target/debug/examples/opencode_live 2>/tmp/g12-oc.log | tee /tmp/g12-oc.out
   pgrep -fl "opencode (serve|attach)"
   ```

   應該看到（開工時細化）：`serve ready on 127.0.0.1:<port> (auth on)`、`request without password → 401`、`session <S> created`、`m-1 idle → prompt_async → confirmed (messageID accepted)`、三行 `(the shim)`、`restart 1/3, session <S> resumed`、最後 `opencode_live: ok`；`pgrep` 沒有輸出。

   - [ ] 通過
   - [ ] 這次不做（寫進驗收紀錄）

5. 三個真 backend 互傳訊息。

   **這步在驗什麼**：同一個真 daemon 上，claude、codex、opencode 各收到一則、各自回覆，三則都到 `confirmed`（里程碑「三個 backend」）。錯了的話某個 backend 的訊息會停在 `sent` 或 `queued`。

   第一個分頁：

   ```bash
   export AGEND_HOME="$(mktemp -d /tmp/g12.XXXX)" && echo "export AGEND_HOME=$AGEND_HOME"
   agend daemon
   ```

   第二個分頁（先跑開頭那段）：

   ```bash
   export AGEND_HOME=<home>    # 步驟 5 印出的那個
   agend instance add g12-c claude; agend instance add g12-x codex; agend instance add g12-o opencode
   agend send g12-c "Reply with exactly: C-OK"; agend send g12-x "Reply with exactly: X-OK"; agend send g12-o "Reply with exactly: O-OK"
   agend status
   ```

   應該看到：三個 `accepted`；幾秒後 daemon log 三行 `… confirmed`，`agend status` 三個 instance 都 `idle`（確切輸出開工時細化，看第 9 施工關的 `status`）。

   - [ ] 通過

6. forge github：在你的 sandbox repo 跑完整流水線。

   **這步在驗什麼**：真的 GitHub 上：push 只推 `agend/…`、開一個 PR、`gh pr checks` 關卡等到 CI、你核准後 daemon 帶著核准的 SHA merge，遠端 branch 被刪（P11–P13）。錯了的話不是 merge 不了，就是 merge 了沒核准的 head。

   先準備：一個你自己的 GitHub repo（`<owner>/<repo>`，可以是空的，有一個會通過的 GitHub Actions），clone 到本機 `<clone>`。第二個分頁：

   ```bash
   export AGEND_HOME=<home>
   ~/.cargo/bin/cargo run -q -p agend-daemon --example adapters_probe -- github-setup <clone>
   agend task create --team g12gh --role dev --workflow gh-demo "hello"
   ```

   （`adapters_probe github-setup` 建 team `g12gh`、兩個假 agent（第 10 施工關的 `fake-worker`）、workflow `gh-demo`：work → submit(github) → `gh pr checks {pr} --watch` → review → approve(human) → merge；開工時細化。）等「需要你」出現 `approval:<t-N>/approve/1`，然後：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-client --example client_probe -- resolve approval:<t-N>/approve/1 approve
   gh pr view --repo <owner>/<repo> <pr> --json state,mergeCommit,headRefName
   git -C <clone> ls-remote origin "refs/heads/agend/*"
   ```

   應該看到：`"state":"MERGED"`；merge commit 的訊息有 `Agend-Task: <t-N>`；`ls-remote` 什麼都不印（branch 已刪）。

   - [ ] 通過

7. 故意弄壞：核准之後、merge 之前，PR 多了一個 commit。

   **這步在驗什麼**：GitHub 只 merge 你核准的那個 SHA；head 變了就回 409、重跑 checks，不會 merge 新的 head（P12）。錯了的話，核准後被推進來的任何東西都會跟著 merge。

   操作：照步驟 6 再開一個 task（`"second"`），等到 `approval:<t-M>/approve/1` 出現**先不要按**；在 `<clone>` 裡：

   ```bash
   git -C <clone> fetch origin agend/<t-M>/second && git -C <clone> switch agend/<t-M>/second
   echo extra > <clone>/extra.txt && git -C <clone> add extra.txt && git -C <clone> commit -m extra && git -C <clone> push origin HEAD
   ```

   再按步驟 6 的 `resolve … approve`。應該看到：daemon log `<t-M> merge: GitHub 409 (head is …, approved …); checks run again`；PR 仍是 open；「需要你」重新出現 `approval:<t-M>/approve/2`（新的 attempt）。最後 `agend task cancel <t-M>` → PR 被關、遠端 branch 被刪。

   - [ ] 通過

8. Telegram：手機收到「需要你」，在手機上核准。

   **這步在驗什麼**：「需要你」會推到手機、手機上按的核准走的是跟 TUI 同一條路，TUI 那邊跟著變成已解決（P16、P17）。錯了的話你離開電腦就不知道有事卡著，或手機按了沒用。

   先準備：一個 bot（token 存成 `<home>/secrets/telegram.token`、`chmod 600`）、一個開了 topic 的 supergroup（bot 是管理員）、你的 user id。寫 `<home>/config.toml`（格式見 P14），重開 daemon（第一個分頁 Ctrl-C 後 `agend daemon`）。然後照步驟 6 開一個 task。

   應該看到：手機的「需要你」topic 出現 `approval <t-K>/approve/1 …` 與兩個按鈕；按 `approve` → watch 印 `attention_resolved approval:<t-K>/approve/1 by telegram:<你的 id>`；手機那則變成「已解決」、按鈕消失；team topic 出現 `<t-K> merged`。

   - [ ] 通過

9. Telegram：在手機上回答請示。

   **這步在驗什麼**：請示是對話（D35）：手機上回覆的文字送到 agent，TUI 的那一項變成已解決（P17）。錯了的話你在手機上回了，agent 還在等。

   操作（開工時細化）：`adapters_probe ask g12-c "Pick a color?"` 讓一個 agent 發請示；在手機上**回覆**那則訊息寫 `blue`。

   應該看到：watch 印 `ask <id> answered by telegram:<你的 id>: blue`；TUI 那一項消失；agent 收到的訊息 `confirmed`。

   - [ ] 通過

10. 故意弄壞：Telegram 的 allowlist 是空的。

    **這步在驗什麼**：設定錯了不會靜靜不動：`doctor` 失敗並給修正方式，daemon 開機也大聲說（v1 #2207、P14）。錯了的話你以為手機會收到，其實全部被丟掉。

    把 `<home>/config.toml` 的 `allowed_user_ids` 改成 `[]`：

    ```bash
    export AGEND_HOME=<home>
    agend doctor; echo "exit=$?"
    ```

    應該看到：Telegram 那一列 `FAIL`：allowlist 是空的、所有訊息會被丟棄，並附修正方式（在 `<home>/config.toml` 的 `allowed_user_ids` 加回你的 user id）；`exit=1`。重開 daemon 時 log 有一行 `telegram: allowed_user_ids is empty; not receiving`。改回來後 `doctor` 那一列 `ok`。第 13 施工關之後，修正方式改成指向 `agend telegram setup`。

    - [ ] 通過

11. 收尾。

    **這步在驗什麼**：什麼都不留（第 6 施工關的孤兒巡查照舊）。

    操作：各分頁 Ctrl-C，然後 `~/.cargo/bin/cargo run -q -p agend-daemon --example adapters_probe -- teardown`（開工時細化）。

    應該看到：`pgrep -fl "agend holder g12-"`、`pgrep -fl "opencode (serve|attach)"` 都不印；`<home>` 不見了；你的 sandbox repo 上沒有 `agend/*` branch、沒有 open 的 PR。

    - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-28 開工前提案 P1–P21 寫定（draft PR，branch `docs/gate-12-proposal`），待使用者逐題確認；U1–U18 未查證事實列表；「你親自驗收」改成 11 步；狀態改為提案中。
- 2026-09-25 使用者決定：真 CLI 一致性檢查（錄製器 + `tests/conformance.rs`）列為必要完成條件（`feat/backend-recorder`）。

## 下一步

```bash
cat docs/gates/gate-12-adapters.md
~/.cargo/bin/cargo xtask accept adapters
```
