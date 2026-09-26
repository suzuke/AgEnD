# 第 9 施工關：agend CLI（`cli`）

> **TL;DR**
> - agent 命令、操作者命令（`instance add/remove/list`、`daemon restart`）、`status`、`doctor`、`init`；真 daemon 本關做 `status`、`send`、`inbox`（接第 7 施工關的送達，完成里程碑「兩個 codex agent 互傳訊息、重啟不漏不重」）與操作者命令，其他 agent 命令先對假 daemon 做完。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：逐題確認 P1–P10。**P1、P2、P3、P8、P9 與已定的文件不同，要明確決定**；確認後 merge 這份提案，等第 7、8 施工關都 merge 後開工。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**提案中**（2026-09-26）：開工前提案 P1–P10 寫定，待使用者確認。依賴（**兩個都是開工的硬前提**）：第 8 施工關（client protocol 1.1、`agend-client`）正在實作（`feat/gate-08-client`），本頁照它已確認的設計寫，不照程式；第 7 施工關（送達、`messages` 表、`deliver` API）的提案在審（`docs/gate-07-proposal`），本頁照它的提案寫，確切介面以它 merge 後為準。

## 範圍

- 11 個 agent 命令（D17）：CLI 全部做完、對假 daemon 驗；真 daemon 本關做哪些見 P1
- `agend send` → 第 7 施工關 `deliver` 的接線、client protocol `Send` 加 `level` 與 `message_id`（第 7 施工關提案 P9 交給第 8、9 施工關；第 8 施工關沒做，由本關接）（P1、P5）
- 操作者命令：`agend instance add/remove/list`（正式命令；`daemon_probe` 降為只給開發與舊驗收用的工具，見 P6）、`agend daemon restart`（P1、P6、P7）
- `agend status`：agent 看自己、操作者看全貌（P1）
- `agend doctor`、`agend init`（服務註冊在第 13 施工關）（P8、P9）
- `agend doctor` 列出所有 holder 並標出孤兒（DB 沒有的 instance）（第 4 施工關 P2，使用者 2026-09-25 決定；做法見 P8）
- `agend daemon restart` 的 D2 重啟預檢：從第 6 施工關移來（[gate-06 P7](gate-06-daemon-holder.md#p7d2-的重啟預檢)，使用者 2026-09-26 追認）——新 binary 先用最新 DB 快照的複本跑 migration 加 `quick_check`，再用暫存 home 起一個自己的 holder 跑一次（hello、Spawn、Shutdown），都過了才切換，任何一步失敗就留在舊版（做法見 P7）
- 從第 8 施工關移來（[gate-08 P7](gate-08-client.md#p7agend-client-的-api重試錯誤訊息)）：逐個命令決定斷線後可不可以重送（P5）；依錯誤型別選訊息與 exit code（P4）
- `AGEND_HOME` 的預設值（第 6 施工關 P1 交給第 9、13 施工關）（P3）

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子。文件已定、這裡不重問的：CLI 分 agent 命令與操作者命令、agent 命令 11 個、daemon 依身分限制權限並附正確命令（D17）；本關只做 `doctor`、`init`，安裝規則放 core `setup`（D24）；CLI 不建 runtime、不讀設定、不開 DB，client 同步 I/O（D11、[tui-and-setup](../architecture/tui-and-setup.md#agent-介面clid7d17)）；instance 存 DB（D8）；`general` team 內建（D12）；D2 預檢的三個步驟（第 6 施工關 P7）；身分用 `hello` 的 `caller`、從 `AGEND_INSTANCE` 來、不做 cookie，DB 沒有的 instance 當 agent（第 8 施工關 P2）；連不上重試 10 秒、版本不合立刻失敗、錯誤型別三種、`--help` 範例優先、`--json`（第 8 施工關 P3、P7，規劃 §4.7）；daemon 只在前景跑（第 6 施工關 P1）。

### P1：命令面：本關做哪些、誰能跑、真 daemon 做到哪

- 問題：D17 定了 11 個 agent 命令，但真 daemon 還沒有 task、請示、訊息。操作者命令要做哪些？`agend status` 對人和對 agent 一樣嗎？權限在哪擋？
- 建議：
  - 本關的命令（其他都不做）：

    | 命令 | 誰 | 真 daemon 本關 |
    |---|---|---|
    | `agend status` | 兩者 | agent：自己的 instance、目前 task（本關一律「沒有 task」）；操作者：全貌摘要（`get_fleet`） |
    | `agend send`、`agend inbox` | agent | 做：`send` 呼叫第 7 施工關的 `deliver(…, level)`（本關負責這段接線），`inbox` 讀第 7 施工關的 `messages` 表裡寄給自己的；第 7 施工關 merge 是本關開工的硬前提 |
    | `done`、`result`、`review approve`／`changes`、`block`／`unblock`、`task create`、`remind` | agent | `not_supported`，訊息寫「第 10 施工關」 |
    | `agend ask` | agent | `not_supported`，**移到第 10 施工關**（見下） |
    | `agend instance add`／`remove`／`list` | 操作者（`list` 唯讀，兩者都可） | 做（P6） |
    | `agend daemon restart` | 操作者 | 做（P7） |
    | `agend doctor`、`agend init` | 操作者 | 不需要 daemon（P8、P9） |

    已有的留著：`agend daemon`、`agend holder`、`--version`、`--help`；`agend debug ping/watch` 在第 8 施工關的實作 merge 後才有，本關不動它們。
  - 權限只在 daemon 擋（D17），CLI 不預先判斷：`command` 請求（agent 命令）只收 agent；會改東西的操作者請求（P6 的 `operator`、第 8 施工關的 `resolve_attention`、`terminal_input`）只收操作者；唯讀請求（`get_fleet`、`subscribe_*`）大家都能用。拒絕一律 `forbidden`，訊息附正確做法。
  - 第 9、10 施工關的分工：**本關負責**所有命令的 CLI 語法、client 接線、錯誤對應與重送規則（P2、P4、P5）；**第 10 施工關負責** `ask`（daemon 端與 store）、`done`／`result`／`review`／`block`／`unblock`／`remind`／`task create` 的 daemon handler、`agend workflow`／`agend team` 命令。這些 handler 在第 10 施工關之前，真 daemon 一律回 `not_supported`（訊息寫「第 10 施工關」），不做半套。第 10 施工關在派工訊息印本關的 ticket（P2），`inbox` 照本關的模型（唯讀游標、`--after`）。
  - **待你決定（本頁不決定）**：操作者能不能跑 `task create`？本頁照 D17 把它列為 agent 命令；第 10 施工關的驗收在你決定前先用開發用的 probe 建 task。
  - **待你決定（KISS，本頁不套用）**：(a) 沒有真 handler 的命令（`done`、`result`、`review`、`block`／`unblock`、`remind`、`task create`、`ask`）的 CLI 語法與假 daemon 的列，整批移到第 10 施工關，跟 handler 一起做；(b) `init` 再縮成只建目錄（不跑 doctor）。目前的建議是兩者都留在本頁。
  - `agend status` 一個命令兩種：有 `AGEND_INSTANCE` 送 `command {status}`，沒有送 `get_fleet`。
  - **`ask` 移到第 10 施工關**（**與第 8 施工關 P6「請示在第 9、10 施工關」不同，請明確決定**）：真的請示要新表、「需要你」來源、`answer_ask`、追問與結論（D35），本關做等於提前一半的第 10 施工關；第 11 施工關 B 段「回答請示」那半一樣等第 10 施工關。
  - 收件者是 claude／opencode（driver 在第 12 施工關）：`send` 照樣寫進 `messages` 表、狀態停在 `queued`，回 `queued`；收件者用 `agend inbox` 看得到；第 12 施工關的 driver 上線後照第 7 施工關的規則補送（實際行為以第 7 施工關 `deliver` 對沒有 driver 的 instance 怎麼做為準）。`CLI-n` 有一列驗它。
  - `send` 的 `level`：`agend send <to> "<message>" [--level queue|steer|interrupt]`，預設 `queue`（第 7 施工關 P6：等級由呼叫者傳入、預設 `Queue`）。`Send` 加選填的 `level` 與 `message_id`（additive，P5）。
- 理由：里程碑「第 1–9 施工關：兩個 codex agent 互傳訊息；重啟 daemon 時不中斷、不遺失、不重複」（[ROADMAP](../ROADMAP.md#里程碑使用者可見)）只需要 `status`、`send`、`inbox` 與加 instance，而第 7 施工關只做 `deliver` API、把 `agend send` 與 `level` 交給後面，第 8 施工關沒接，所以接線由本關負責，本關結束時里程碑要真的跑得起來（步驟 8）；其他 agent 命令沒有 task 就沒有真資料可回，回明確錯誤比假資料好（第 8 施工關 P6 同一條路）。權限只放 daemon 一處，TUI、Telegram、CLI 同一套規則。
- 替代方案：本關就做 `ask` 的真實作（範圍多一張表與請示流程）；`send`／`inbox` 在第 7 施工關沒 merge 時先回 `not_supported`、里程碑延到第 10 施工關（**會讓 ROADMAP 的里程碑延後，要改的話請明確決定**；本頁不建議）；CLI 也擋一次權限（兩處規則會漂移）；agent 連唯讀的全貌都不能看（`agend instance list` 在 agent 裡就要另開例外）。
- 例子：操作者跑 `agend done t-1/work/1` → `agend: forbidden: agend done is an agent command; it runs inside an agent, where AGEND_INSTANCE is set`、exit 1；agent `g9-1` 跑 `agend instance add x claude` → `agend: forbidden: only the operator can add instances; ask the operator`、exit 1。
- [ ] 使用者確認

### P2：agent 命令的語法；task id 與 attempt 從哪來（ticket）

- 問題：core 的 `Done`、`Result`、`ReviewApprove`、`ReviewChanges` 要 `task_id` 加 `identity`（關卡 id、attempt），`Block`、`Unblock`、`Remind` 要 `task_id`。規劃說 agent 只傳意圖、task id 由 daemon 推得；可是 attempt 推不得：reviewer 核准得晚，main 前進後已經是新的 attempt、新的 head，daemon 若拿「現在的 attempt」補上，就等於核准了沒審過的 head（D14）。
- 建議：
  - 結果類命令帶一個 **ticket**：`<task_id>/<stage_id>/<attempt>`，例如 `t-42/review/2`。它印在派工訊息（第 10 施工關產生）與 `agend status` 裡；agent 照抄派工訊息裡的那一個。CLI 把它拆成 `task_id` 與 `identity`，protocol 不改。
  - 只需要 task 的命令（`block`、`unblock`、`remind`）不帶 ticket：CLI 先送 `status` 拿目前的 `task_id` 再送（D33：一個 agent 一個 task）。
  - `StatusData` 加選填的 `identity`（additive），`agend status` 才印得出 ticket。
  - 語法（常用命令參數 ≤ 2）：

    | 命令 | 語法 |
    |---|---|
    | status | `agend status` |
    | send | `agend send <to> "<message>" [--level queue\|steer\|interrupt]`（預設 `queue`） |
    | inbox | `agend inbox [--after <message-id>]`（不帶：最近 20 則）；「之後」依寫入順序（`messages` 的 rowid），不比 UUID；第 10 施工關的假 agent 用同一個規則 |
    | done | `agend done <ticket>` |
    | result | `agend result <ticket> "<summary>"` |
    | review | `agend review approve <ticket>`、`agend review changes <ticket> "<what to change>"` |
    | ask | `agend ask "<question>" [--option <text>]…`；追問 `--follow-up <ask-id>`、結束 `--resolve <ask-id>` |
    | block | `agend block "<reason>"`、`agend unblock` |
    | task create | `agend task create --role <role> "<title>" [--team <team>] [--workflow <id>]` |
    | remind | `agend remind <delay>`（`90s`、`30m`、`2h`） |

  - **與 [tui-and-setup](../architecture/tui-and-setup.md#agent-介面clid7d17)「daemon 推得 task_id」字面不同，請明確決定**：ticket 裡有 task id；推不得的是 attempt，task id 只是順便帶著。
- 理由：第 1 施工關的事件身分（每個結果帶 attempt，過期一律 `stale_result`）是 verifier 推翻四輪才定下的結構；在 CLI 或 daemon 補上「現在的」attempt 會把這層保護拿掉。一個 ticket 參數仍然符合 ≤ 2 個參數。
- 替代方案：daemon 從呼叫者目前的 assignment 補 `task_id` 與 attempt（agent 最簡單；晚到的核准會算在新的 head 上）；CLI 先問 `status` 再補（同樣的問題，只是在 CLI 做）；分開三個旗標 `--task --stage --attempt`（參數太多，agent 容易抄錯一個）。
- 例子：`agend status` → `t-42 · review (attempt 2) · ticket t-42/review/2`、`next: agend review approve t-42/review/2 | agend review changes t-42/review/2 "<what to change>"`；attempt 3 開始後才送 `agend review approve t-42/review/2` → `agend: stale_result: t-42/review/2 is no longer current (now t-42/review/3); run agend status`、exit 1。
- [ ] 使用者確認

### P3：`AGEND_HOME` 怎麼找

- 問題：第 6 施工關的 daemon 必須設 `AGEND_HOME`，預設值留給第 9、13 施工關。CLI 要用同一個 home 算出 socket。預設放哪？會不會踩到 v1？
- 建議：
  - 順序只有兩步：`AGEND_HOME`（必須是絕對路徑，否則 `AGEND_HOME must be an absolute path` exit 2）→ 沒設就用 **`$HOME/.agend-v2`**。
  - 一個函式 `agend` 的 `home::resolve()`，CLI、`agend daemon`、`doctor`、`init` 都用它；`agend daemon` 改成收這個路徑（第 6 施工關 P1 的「沒設就 exit 1」改成套用預設）。holder 與 agent 照舊由 daemon 明確設 `AGEND_HOME`（第 6 施工關 H3），agent 裡的 CLI 自然連到同一個 daemon。
  - v1 防護：解出來的 home 裡有 `fleet.yaml`（v1 的檔）就拒絕：`<path> looks like an AgEnD v1 home (fleet.yaml); set AGEND_HOME to another directory`、exit 1。
  - 不做 `--home` 旗標、不從 `config.toml` 讀 home：設定檔本身就在 home 裡，是循環。**與已定的 D8／[tui-and-setup](../architecture/tui-and-setup.md#設定與目錄d8)「`config.toml`：home 路徑、Telegram 等連線設定」不同，請明確決定**：建議把「home 路徑」從 `config.toml` 的內容拿掉，只留連線設定。
- 理由：v1 用 `~/.agend`（舊的是 `~/.agend-terminal`），而里程碑之後要與 v1 並行一週、分開 home；預設踩到 v1 的 DB 目錄後果最嚴重。名字直接寫 `v2`，一眼分得出。預設要一開始就定，之後改名等於要搬家。
- 替代方案：`~/.agend` 加 v1 防護（並行那週一定被擋，最後還是要設 `AGEND_HOME`）；`~/.local/share/agend`（XDG，沒有 v2 字樣但多一條 `XDG_DATA_HOME` 規則）；macOS 的 `~/Library/Application Support/agend`（路徑有空白，指令常要加引號）；永遠必須設 `AGEND_HOME`（最簡單，但 `init` 之後每個終端都要 export）。
- 例子：沒設 `AGEND_HOME` → `agend status` 連 `/Users/you/.agend-v2/run/daemon.sock`；`AGEND_HOME=~/.agend agend daemon`（v1 home）→ 被拒、exit 1。
- [ ] 使用者確認

### P4：輸出、exit code、錯誤訊息

- 問題：人和 agent 都讀 CLI 的輸出。人看的格式、`--json` 格式、exit code 怎麼定？第 8 施工關的三種錯誤怎麼對應到訊息？
- 建議：
  - 人看的：結果寫 stdout、錯誤寫 stderr（開頭 `agend: `），英文；有沒有 TTY 輸出都一樣（只有 `instance remove` 會在 TTY 上問，P6）。沒有顏色。
  - `--json`（每個命令都有，位置不限）：stdout 只印**一個** JSON 值：core 型別直接用 `serde_json` 編（`CommandResult`、第 8 施工關的全貌、P8 的 doctor 報告），不另訂格式；錯誤也印在 stdout：`{"error":{"code":"…","message":"…"}}`，stderr 不印。JSON 的相容規則就是 client protocol 的（D26，只加欄位）。
  - exit code 只有三個：`0` 成功；`1` 失敗（連不上、版本不合、daemon 拒絕、`doctor` 有 fail、斷線沒重送）；`2` 用法錯誤（參數錯、未知命令；目前的 `cli.rs` 已經是 2）。細分靠 `--json` 的 `code`。
  - 錯誤對應：

    | 來源 | 訊息（stderr） | `--json` 的 code |
    |---|---|---|
    | 連不上（10 秒後） | 第 8 施工關 P7 那段：`cannot reach the AgEnD daemon at … after 10 s (…). Is it running? Start it with: agend daemon` | `daemon_unreachable` |
    | 版本不合 | 第 8 施工關 P3 那段（`… restart the daemon with this binary`，P7 之後改成 `run: agend daemon restart`） | `version_mismatch` |
    | daemon 回錯誤 | daemon 的 `message` 原樣印出，前面加 `agend: <code>: ` | daemon 的 `code` |
    | 送出後斷線、不重送（P5） | `daemon restarted during the request; check with <這個命令的檢查指令>` | `interrupted` |
    | 參數錯 | clap 的訊息加一行範例 | `usage` |

  - 修正指令由**發現錯誤的那一方**寫：daemon 的錯誤訊息自己附正確命令（D17），CLI 不再加字；CLI 自己發現的（連不上、版本、參數、home）才由 CLI 寫。
  - 新錯誤碼（core 常數，跟第 8 施工關的放一起）：`unknown_instance`、`instance_exists`、`preflight_failed`。
  - 參數解析用 `clap`（derive）；`--help` 先列範例（`before_help`）。
- 理由：agent 讀 exit code 只需要分「成功／失敗／我打錯」；更細的分類放 JSON，exit code 就不用當成第二套錯誤碼維護。`--json` 用 protocol 型別，只有一份格式、一套版本規則（#1493）。
- 替代方案：每種錯誤一個 exit code（連不上 = 3 之類；兩套錯誤碼要同步）；`--json` 的錯誤寫 stderr（agent 要讀兩個串流）；手寫參數解析（約 15 個命令、每個都要自己處理 `--help` 與錯誤）。
- 例子：`agend status --json`（操作者）→ `{"as_of_event_id":…,"teams":[…],"instances":[…],"attention":[…]}`、exit 0；daemon 沒在跑：`agend status --json` → 10 秒後 `{"error":{"code":"daemon_unreachable","message":"cannot reach …"}}`、exit 1。
- [ ] 使用者確認

### P5：daemon 重啟時，哪些命令斷線後重送

- 問題：第 8 施工關 P7 定了：**請求送出後**斷線，只有呼叫端標「可重做」的才重送，逐個命令交給本關決定。
- 建議：規則一句話：**唯讀的重送；會改東西的只有 `send` 重送（靠訊息 id 去重）**，其他不重送、印出該用哪個命令確認。

  | 命令 | 重送 | 理由／不重送時的檢查指令 |
  |---|---|---|
  | `status`、`inbox`、`instance list`、`doctor` 裡的查詢 | 是 | 唯讀（`inbox` 帶游標讀，不消耗訊息） |
  | `send` | 是 | CLI 產生 `message_id`（UUID v4），`Send` 加選填欄位（additive），daemon 把它當成 `deliver` 的訊息 id；第 7 施工關 P5 的 `messages` 表以 id 為主鍵、`deliver` 先查表，同一個 id 再送回目前的狀態、不再送一次——這就是唯一的一套冪等。daemon 在 `deliver` 寫入 DB **之後**才回應。**client 的 id 跟別人分開**：daemon 只收 UUID v4 格式的 `message_id`（第 10 施工關的派工訊息用 `dispatch:<ticket>`，不是 UUID，撞不到）；表裡已有這個 id、但 from／to／body／level 有任何一個不同 → `invalid_request: message id … is already used by another message`，不當成去重、不回 `accepted`。沒帶 `message_id` 的 `Send`（1.1 的 client）由 daemon 產生 UUID |
  | `done`、`result`、`review …` | 否 | 重送會被當成過期（`stale_result`），訊息反而誤導；檢查 `agend status` |
  | `block`／`unblock`、`remind`、`task create`、`ask` | 否 | 會重複建立或覆蓋；檢查 `agend status` |
  | `instance add`／`remove` | 否 | 重送會回 `instance_exists`／`unknown_instance`；檢查 `agend instance list` |
  | `daemon restart` | 不適用 | 斷線本來就是預期的（P7），CLI 改等新 daemon |

- 理由：里程碑要求「重啟 daemon 時訊息不遺失、不重複」：只有 `send` 同時需要「不遺失」（agent 以為送出了）與「不重複」，用它自己的 id 去重是唯一兩者都做得到的路，而且就是送達模型已經有的那套冪等（V1-LESSONS #1：不再多一套去重）。
- 替代方案：全部不重送（`send` 斷在中間時 agent 只能猜，重下一次就可能重複）；daemon 依 `request_id` 去重（要跨重啟記住，第 8 施工關 P7 已否決）；`block`／`unblock` 當成冪等也重送（第 10 施工關有真實作再說）。
- 例子：`agend send g9-2 "hi"` 送出後 daemon 重啟 → CLI 等 1.4 秒、用同一個 `message_id` 重送 → daemon 看到已經有這個 id，回 `accepted`，`g9-2` 只收到一次；`agend instance add g9-3 claude` 斷在中間 → `agend: daemon restarted during the request; check with agend instance list`、exit 1。
- [ ] 使用者確認

### P6：操作者命令：`instance`、協定的下一個 minor

- 問題：client protocol 只有給 agent 的 `command`。加減 instance、重啟 daemon 要怎麼送？`instance add` 要哪些參數？`daemon_probe` 怎麼辦？
- 建議：
  - 新請求 `operator { request_id, command: OperatorCommand }`，`OperatorCommand` 有 `instance_add`、`instance_remove`、`daemon_restart`（P7）與 `unknown`；成功回現有的 `command_result`：`accepted`，或新的 `instance_added { instance_id, session_id, working_directory }`、`restarting { preflight }`。`instance list` 與操作者的 `status` 用第 8 施工關的 `get_fleet`，不另開請求。
  - 本關所有新增算一次 minor：client protocol 用**實作時的下一個 minor**（目前是 1.2；第 10 施工關的提案也要加一個 minor，兩關誰先 merge 誰拿 1.2，本頁的 1.2 都照這條換）。
  - 本關加的欄位（全部選填、additive）：`hello` 回應加 `daemon_version`（`agend 0.x.y`）、`daemon_pid`、`boot_id`（P7）；全貌的每個 instance 加 `working_directory`；`Send` 加 `level`、`message_id`（P1、P5）；`StatusData` 加 `identity`（P2）。`status`、`restart`、`doctor` 印的 pid 與版本、`instance list` 的 DIR 欄都從這裡來。
  - **`daemon_restart` 的形狀之後不再改**：舊 daemon 要能聽懂新 CLI 叫它重啟，否則升級時「重啟到新版」這條路會斷。
  - `agend instance add <id> <backend> [--dir <path>] [--program <path>] [-- <args>…]`：id 規則 `[a-z0-9-]{1,24}`（第 6 施工關 P2）；`--dir` 預設 `$AGEND_HOME/workspace/<id>`（daemon 建）；`--program` 預設就是 backend 名（claude／codex／opencode，從 agent 的 `PATH` 找），測試與驗收用它指向假 agent。daemon 寫入 `new`、claude 產生 session id（第 6 施工關 H2），**馬上啟動**，回傳。team 一律 `general`（D12；本關沒有 team 表）。同一個 id 的 holder 鎖還被持有（剛 `remove`、holder 還沒結束，或留著的孤兒）→ 拒絕：`instance_exists: a holder for g9-1 is still running (run/holders/g9-1.lock); wait a few seconds or restart the daemon to sweep it`，不寫 DB。
  - `agend instance remove <id> [--yes]`：TTY 上問 `remove g9-1? its agent is stopped; the workspace is kept [y/N]`；非 TTY 必須 `--yes`（否則 exit 2）。daemon 送 `Shutdown` 給 holder（最多等 5 秒）、刪那一列、發 `instance_changed`。holder 連不上也照樣刪，下次開機的孤兒巡查會收掉它（第 6 施工關 P2）。**workspace 不刪**，印出路徑。
  - `agend instance list`：`ID  BACKEND  STATE  DIR` 一行一個。
  - `agend instance` 是**正式**命令；`daemon_probe` 不刪，降為開發工具（daemon 停著時直接改 DB；`--dies` 與第 6、8 施工關的驗收步驟在用）。第 9 施工關之後的文件與驗收一律用 `agend instance`。
- 理由：寫 DB 的路只有 daemon 一條，daemon 跑著時也能加減 instance（`daemon_probe` 做不到）；一個 `operator` 請求加一個 enum，之後的 team、workflow 命令只加變體。
- 替代方案：每個操作者命令一種請求（請求種類一直變多）；把操作者命令塞進 `AgentCommand`（權限規則要逐個變體判斷）；`instance add` 在 daemon 沒跑時直接開 DB（兩條寫入路徑；CLI 路徑不開 DB，D11）；刪 instance 時一起刪 workspace（刪資料前要問，v1 教訓）。
- 例子：`agend instance add g9-1 claude --program "$PWD/target/debug/fake-claude"` → `added g9-1 (claude, session 3f…, /Users/you/.agend-v2/workspace/g9-1); starting`；`agend instance list` → `g9-1  claude  starting  /Users/you/.agend-v2/workspace/g9-1`。
- [ ] 使用者確認

### P7：`agend daemon restart` 與 D2 預檢

- 問題：D2 要能重啟 daemon、先預檢新 binary、失敗不切換。第 13 施工關之前 daemon 在前景跑，沒有 launchd／systemd 幫忙重起。誰啟動新的 daemon？新 binary 是哪一個？
- 建議：
  - **原地 `exec`**：daemon 預檢通過後，照第 6 施工關 P1 的順序收尾（停排程、關 listener、刪 socket、關 holder 連線、關 DB；**不送 `Shutdown`**），再 `exec` 新 binary（`agend daemon`，環境不變）。pid 不變、終端不變；新 daemon 照 `plan_boot` 接回所有 holder（D3）。
  - 新 binary ＝ 下命令的那個 `agend`（CLI 自己的 `current_exe()`，取絕對路徑），也可以 `--binary <path>` 指定。這就是第 8 施工關 P3「restart the daemon with this binary」的意思。`--binary` 等於讓 daemon 在自己的環境（含 secret，第 6 施工關 H3）跑任意程式：只收操作者，而同 uid 的程式本來就做得到同樣的事，在 D6「同 uid 只是安全帶」的已接受範圍內。
  - 同時只能有一個重啟：預檢進行中再收到 `daemon_restart` 回 `invalid_request: a restart is already in progress`；暫存 home 用 `mkdtemp`（`/tmp/agend-pf-XXXXXX`），不會跟別的撞名。
  - 流程：
    1. CLI 送 `operator {daemon_restart {binary}}`（只收操作者）。
    2. daemon 用第 5 施工關的快照函式（`VACUUM INTO`）把 DB 複製到暫存 home `/tmp/agend-pf-XXXXXX/agend.db`（0700；路徑短，holder socket 才不超過 100 bytes，第 6 施工關 H13）。**細化**：「最新快照」用當下新拍的一份，不是今天稍早的每日快照，預檢看到的才是現在的資料。
    3. daemon 起子程序 `<binary> daemon preflight /tmp/agend-pf-XXXXXX`（60 秒逾時）：用**新 binary 的** migration 開複本、`PRAGMA quick_check`、讀一次 `instances`；DB 比新 binary 新（降版）會被第 5 施工關的 too-new 規則擋下，也算失敗。接著在暫存 home 起自己的 `agend holder pf-check`，跑 hello、`Spawn`（`/bin/sh -c 'sleep 60'`）、`Shutdown`，等鎖放掉。每步印一行。
    4. 任何一步失敗：回 `preflight_failed`（附那一行），刪暫存 home，舊 daemon 照跑，**DB 本身一個 byte 都沒動**。
    5. 都過了：回 `restarting { preflight }`，然後收尾、`exec`。
    6. CLI 印預檢結果，**先等這條連線 EOF**（daemon 收尾時關掉它，代表舊的已經不聽了），才用 `Client::connect`（10 秒重試）連新 daemon；連上後比 `hello` 的 `boot_id`，**必須跟重啟前不同**才印 `the daemon is back`，否則繼續重試到 10 秒後報錯。pid 與版本在同一個 binary 的 `exec` 前後都一樣，不能拿來判斷。
  - `hello` 回應再加選填的 `boot_id`：這次開機的起點（第 8 施工關 P4 的事件 id 起點＝開機時間 unix ms × 1000），每次開機（含 `exec`）都會變。
  - `exec` 之後 daemon 以前起的 holder 仍是它的子程序（pid 沒變），但負責 `wait()` 的 thread 已經不在。新 daemon 開機時只對**從鎖檔讀到、接回的 holder pid** 定期 `waitpid(pid, WNOHANG)`（不是自己的子程序會回 `ECHILD`）；**第一次收到 `ECHILD` 或收屍成功就不再查那個 pid**，免得 pid 被重用後誤收別的程序；**不用 `waitpid(-1)`**：那會搶走 `runtime.rs` 每個自己起的 holder 的 `child.wait()` thread 與預檢子程序的 exit status（步驟 4 靠它判斷失敗）。
  - 預檢不連**正在跑的** holder（新連線會踢掉 daemon 的長連線，第 6 施工關 P4）；「新 daemon 聽得懂舊 holder」靠 core 的 `SUPPORTED_VERSIONS` 包含前一個 major（第 6 施工關 P7 的測試）。
- 理由：`exec` 不 fork、不另起程序，所以沒有 v1 #881／#882／#903 那種「自己 fork 到背景」的問題，也沒有「舊的還沒走、新的已經起來」的兩個 daemon；同一個 pid 在前景、launchd、systemd 下行為都一樣，本關就驗得到。預檢都在複本與暫存 home 上跑，失敗時什麼都沒改。
- 替代方案：daemon 以特定 exit code 結束、交給服務管理器重起（第 13 施工關之前前景跑時沒人重起，本關驗不到）；CLI 自己起新 daemon（v1 的路，D2 否決）；直接對 `agend.db` 預檢（第 6 施工關 P7 已否決）；用今天的每日快照（可能差好幾個小時的資料）。
- 例子：

  ```text
  $ agend daemon restart
  preflight agend 0.0.0 (/Users/you/AgEnD-v2/target/debug/agend):
    db copy: migrations 3 -> 3, quick_check ok, instances 2
    holder: hello ok, spawn ok, shutdown ok
  restarting the daemon (pid 5101) ...
  the daemon is back: pid 5101, agend 0.0.0, client protocol 1.2, instances=2 (1.2 s)
  $ agend daemon restart --binary /usr/bin/false
  agend: preflight_failed: /usr/bin/false daemon preflight exited with status 1; the daemon keeps running agend 0.0.0
  ```
- [ ] 使用者確認

### P8：`agend doctor` 本關查什麼

- 問題：[tui-and-setup](../architecture/tui-and-setup.md#安裝與設定) 列了一長串 doctor 檢查（backend 登入、版本範圍、gh、服務、Telegram…），很多要等第 12、13 施工關才有東西可查。本關做哪些？沒裝某個 backend 算失敗嗎？
- 建議：
  - 本關的檢查（每一行 `ok`／`warn`／`fail`，非 `ok` 的一定附 `fix:` 指令）：

    | 檢查 | fail 的條件 | warn 的條件 |
    |---|---|---|
    | home | 不存在、不是目錄、不能寫、是 v1 home（P3） | 權限不是 0700 |
    | daemon | — | 連不上（`connect_once`，不等 10 秒）；fix `agend daemon` |
    | git | 找不到，或版本 < 2.38（merge-tree 需要） | — |
    | claude、codex、opencode | 找不到，**而且** daemon 裡有 instance 用它 | 找不到（沒有 instance 用它）；找得到就印 `--version` 的第一行 |
    | holder | — | 孤兒：鎖被持有、daemon 的 instance 清單沒有它；daemon 沒跑時標 `unknown`；fix `agend daemon`（開機巡查會收掉） |
    | 磁碟 | home 所在磁碟剩 < 1 GB | home 超過 20 GB（v1 長到 161G） |

  - exit 1 ⟺ 至少一個 `fail`。`--json` 印每一項 `{check, status, detail, fix}`。
  - 規則（git 最低版本、各 backend 的安裝指令、門檻數字）放 `agend_core::setup` 當資料；執行在 `agend` 的 `setup` 模組（D24）。
  - **不做**：backend 登入與版本範圍（第 12 施工關接真 backend 時才有依據）、gh 登入（第 12 施工關 forge github）、服務狀態（第 13 施工關）、Telegram（第 12 施工關）。**與已定的 [tui-and-setup](../architecture/tui-and-setup.md#安裝與設定)（doctor 查 gh 登入、各 backend 安裝／版本／登入、服務狀態、Telegram）不同，請明確決定**：本關只做表裡這幾項，其餘延到上面寫的施工關，D24 的「每個 doctor 檢查都有故意弄壞的測試」在第 13 施工關對全部檢查補齊。
  - Must NOT：改任何東西、開 DB、連 holder 的 socket（會踢掉 daemon 的長連線）；holder 是否活著只看鎖（跟 daemon 的 `recover_holders` 用同一個函式）。
- 理由：只有「用得到的 backend 沒裝」才是錯；每台機器都缺一兩個 backend，若一律算 `fail`，doctor 永遠是紅的，人就不看了。instance 清單只問 daemon（CLI 不開 DB）。
- 替代方案：缺任何 backend 都 `fail`（原本步驟 3 的寫法）；doctor 自己開 DB 讀 instance（daemon 跑著時拿不到鎖）；本關就做登入檢查（沒有真 backend 的錄製資料，規則只能猜）。
- 例子：

  ```text
  ok    home      /Users/you/.agend-v2 (0700)
  ok    daemon    pid 5101, agend 0.0.0, client protocol 1.2
  fail  git       git 2.30.0 is older than 2.38 (merge-tree needs it)
        fix: brew install git
  ok    claude    2.1.3 (Claude Code)
  warn  opencode  not on PATH; no instance uses it
        fix: brew install opencode
  ok    holders   2 running, 0 orphans
  ok    disk      212 GB free, home 38 MB
  1 check failed
  ```
- [ ] 使用者確認

### P9：`agend init` 本關做到哪

- 問題：[tui-and-setup](../architecture/tui-and-setup.md#安裝與設定) 的 `init` 要建 home、`config.toml`、註冊服務、偵測 backend、建 `general` team 與一個 agent、在 repo 裡問要不要登記、最後跑 doctor。本關做哪些？原本步驟 4 的 `--non-interactive` 要不要？
- 建議：
  - 本關的 `init`：照 P3 解出 home → 建目錄（0700；已存在就不動，印 `already exists`）→ 跑 `doctor` → 印下一步（`agend daemon`、`agend instance add <id> <backend>`）。可以重跑。v1 home 拒絕（P3）。
  - **不問任何問題**，所以本關**不做 `--non-interactive`**：第一個問題出現的施工關（第 13 施工關的服務註冊）再加。
  - **不建 `config.toml`**：本關沒有任何程式讀它（Telegram 在第 12 施工關）；骨架規則是不寫佔位的東西。第一個讀它的施工關負責建。
  - **不建 agent**：寫 instance 只經 daemon（P6），而 `init` 時 daemon 還沒跑；印出下一步的指令。`general` 本來就是內建的（D12），本關沒有 team 表可寫。
  - 不註冊服務、不偵測後問 backend、不登記 repo（第 13、10 施工關）。
  - **與 tui-and-setup 與原本步驟 4 的「建 `config.toml`、`general` team 與一個 agent」不同，請明確決定**。
- 理由：`init` 做的每一件事都要有人用得到；只建目錄加跑 doctor，第一次使用前就能指出設定錯誤（V1-LESSONS #14），其餘等有使用者的施工關再加，不留假的檔案。
- 替代方案：寫一份只有註解的 `config.toml`（沒人讀）；`init` 自己開 DB 加一個 agent（第二條寫入路徑）；`init` 順便在背景起 daemon（v1 的自動起 daemon，D2 否決）。
- 例子：`HOME=/tmp/g9h.x agend init` → `created /tmp/g9h.x/.agend-v2 (0700)`，接著 doctor 的輸出，最後 `next: agend daemon   (then, in another terminal) agend instance add dev-1 claude`。
- [ ] 使用者確認

### P10：怎麼測：假 daemon、真 daemon、同一張表

- 問題：CLI 的輸出要對假 daemon 驗每個命令，也要對真 daemon 驗。怎麼保證兩邊一致？假 daemon 要補什麼？
- 建議：
  - CLI 測試放 `crates/agend/tests/`，跑**真的 `agend` binary**，比對 stdout、stderr、exit code。
  - 假 daemon 加 `FakeDaemon::start_at(path)`，綁在 `$AGEND_HOME/run/daemon.sock`，CLI 照正常路徑找到它；假 daemon 補上本關的規則：P1 的權限、`operator` 請求、`send` 的 `level` 與依 `message_id` 去重、`StatusData.identity`、`daemon_pid`、instance 的 `working_directory`。回應一律用 core 型別編（#1493）。
  - 第 8 施工關的 `CLP` 契約加本關的列（權限兩個方向、`operator` 請求、`daemon_restart` 的形狀、`not_supported` 不改狀態、`send` 同 id 只收一次），照舊對假 daemon 與真 daemon 各跑一次、每條一個 mutant。
  - CLI 層一張表 `CLI-n`（放 `crates/agend/TESTING.md`）：每列＝一個命令＋輸入＋預期輸出／exit code；真 daemon 支援的列跑兩次（假、真），不支援的列標「只對假」並寫哪個施工關補。
  - 預檢的失敗路徑用 `--binary` 指向會失敗的程式（`/usr/bin/false`、一個 `daemon preflight` 時印錯的 shell script），production 程式不加測試用的開關。
  - 啟動時間：加 `clap` 後 `agend --version` p50 仍 < 10 ms（D11 實測 4.1 ms），`xtask accept cli` 印出量到的值。
  - 本關預計**不需要 migration**；若需要，用實作時下一個空號（第 8 施工關用 `0003`、第 7 施工關預計 `0004`）。
- 理由：同一張表跑兩邊，假 daemon 一偏離就失敗（第 8 施工關 P9 同一條路）；測真的 binary 才驗得到 exit code、stderr 與 `home::resolve`。
- 替代方案：只對假 daemon 測（真的 daemon 行為沒被釘住）；在 `agend` 的 lib 層測函式（驗不到 exit code 與輸出串流）；production 加 `AGEND_TEST_PREFLIGHT_FAIL` 之類的開關（測試碼進 production）。
- 例子：`CLI-7`：`AGEND_INSTANCE=g9-1 agend instance add x claude` → stderr `agend: forbidden: only the operator can add instances; ask the operator`、exit 1；假 daemon `fake ok`、真 daemon `real ok`。
- [ ] 使用者確認

### 本關不做（明確列出）

- 真 daemon 的 task 類命令與 `ask` 的 handler、store（第 10 施工關，P1；在那之前回 `not_supported`）；`send`／`inbox` 的送達本身（第 7 施工關）。
- `agend workflow …`、`agend team …`（D19；第 10 施工關，連同第 5 施工關 S1 的 `save_workflow_toml`）。
- `agend daemon stop`（在 launchd／systemd 下「停」要先卸載服務，第 13 施工關一起做）；`export`／`import`、`telegram setup`、`uninstall`（第 13 施工關）。
- 從 CLI 處理「需要你」（`resolve_attention` 由 TUI 做，第 11 施工關 B 段）。
- `config.toml`、`--non-interactive`、服務註冊（P9）；doctor 的登入、版本範圍、gh、服務、Telegram 檢查（P8）。
- 顏色、shell completion、MCP 轉接層（D7「需要時」）、Windows。

### 已知風險（開工時處理）

- 第 8 施工關還在實作：本關從它 merge 後的 `v2` 開 branch；1.1 的型別名稱、錯誤型別以 merge 後的程式為準，本關的 minor 疊在上面（P6：實作時的下一個 minor，與第 10 施工關的提案不要撞號）。
- 第 7 施工關的提案還在審（`docs/gate-07-proposal`）：`deliver(…, level)` 的簽名、`messages` 表的欄位、「同 id 再送回目前狀態」的行為以它 merge 後為準；**第 7 施工關 merge 是本關開工的硬前提**（P1）。它若改成不以 id 冪等，P5 的 `send` 重送就不成立，要回來重新決定。
- 步驟 8 的假 codex instance 怎麼起（`--program` 指向什麼、要不要包裝）依第 7 施工關的啟動方式，開工時細化。
- `exec`（P7）：所有 fd 要有 `CLOEXEC`（Rust 預設有；SQLite 的檔案要實測），否則新 daemon 會繼承舊的 DB 鎖；`exec` 失敗（預檢之後 binary 被刪）時 daemon 已經收尾，只能印錯誤、exit 1——前景要人重跑 `agend daemon`，第 13 施工關由服務管理器重起；launchd／systemd 下 `exec` 的行為在第 13 施工關實測。
- doctor 看鎖（P8）：檢查的那一瞬間若剛好有 holder 在啟動，可能拿不到鎖而啟動失敗；用 `LOCK_SH | LOCK_NB` 並立刻放掉，實測機率。
- ticket（P2）決定了第 10 施工關的派工訊息格式；第 10 施工關若要改，CLI 語法跟著改。
- `~/.agend-v2`（P3）一旦有人用就不好改名；v1 退場後要不要改名，由第 13 施工關決定並附搬家步驟。
- `clap` 讓 binary 變大、編譯變慢；啟動時間超過 10 ms 就改手寫。

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend` 單獨通過，包括：`CLI-n` 表每列對假 daemon、真 daemon 支援的列也對真 `agend daemon`（P10）；`home::resolve` 的預設、相對路徑、v1 home（P3）；`--json` 只印一個 JSON 值、錯誤也在 stdout（P4）；`send` 送出後斷線用同一個 `message_id` 重送、其他會改東西的命令不重送（P5）；非 TTY 的 `instance remove` 沒有 `--yes` 回 exit 2（P6）；`daemon restart` 成功（pid 不變、holder 不變）與 `--binary` 失敗（daemon 照跑、DB 檔 sha256 不變）、同時兩個 restart 第二個被拒、CLI 在舊連線 EOF 且 `boot_id` 變了才報成功、重用別則訊息的 `message_id` 回 `invalid_request`、接回的 holder 死掉後不留殘屍、自己起的 holder 與預檢子程序的 exit status 沒被搶（P7）；兩個假 codex instance 互送 10 則訊息、中途 `daemon restart`：每則恰好送達一次、狀態 `confirmed`（P1、P5，里程碑）；`doctor` 每個 `fail`／`warn` 都有 `fix:`（P8）
- [ ] `~/.cargo/bin/cargo test -p agend-core`、`-p agend-client`、`-p agend-daemon`、`-p agend-testkit` 單獨通過：1.1 的 peer 解得開本關 minor（目前 1.2）的訊息；`CLP` 新增的列對假 daemon 與真 daemon 都通過，每條有 mutant（P6、P10）
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [ ] `~/.cargo/bin/cargo xtask accept cli` 通過，並印出下方「你親自驗收」用到的 demo 與 `agend --version` 的啟動時間（p50 < 10 ms）
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新；想改的共用文件（名詞表：ticket、`~/.agend-v2`、`operator` 請求；tui-and-setup 的 init；第 8、10、13 施工關頁）列在 PR 裡由你決定
- [ ] 測試不留殘留：沒有 `g9-`、`pf-check` 或測試 id 的 holder，沒有 `/tmp/agend-pf-*`；kill 只對自己起的、大於 1 的 pid
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.x.y`（目前是 `agend 0.0.0`）。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

步驟 4 起用同一個暫存 home。第二個終端也要貼上步驟 4 印出的那行 `export AGEND_HOME=…`。

1. 跑 demo。

   **這步在驗什麼**：每個命令對假 daemon 的輸入、輸出、exit code，以及真 daemon 支援的那些命令兩邊結果一樣（P1、P4、P10）。錯了代表 agent 讀到的輸出跟真 daemon 給的不一樣。

   ```bash
   ~/.cargo/bin/cargo xtask accept cli
   ```

   應該看到：每個命令一段：輸入、輸出、exit code；拒絕的都附正確命令；`CLI-n` 每列印 `fake ok`，真 daemon 支援的列另印 `real ok`；一行 `startup: agend --version p50 <n> ms`（小於 10）；最後一行 `gate 9 (cli): checks passed`（確切輸出開工時細化）。

   - [ ] 通過

2. `agend init` 在暫存 HOME，再故意弄壞成 v1 home。

   **這步在驗什麼**：沒設 `AGEND_HOME` 時預設是 `~/.agend-v2`，不會碰到 v1 的 `~/.agend`；看起來像 v1 的 home 一律拒絕（P3、P9）。錯了的話 v2 可能寫進你正在用的 v1 目錄。

   ```bash
   H=$(mktemp -d /tmp/g9h.XXXX)
   env -u AGEND_HOME HOME="$H" agend init; echo "exit=$?"
   touch "$H/.agend-v2/fleet.yaml"
   env -u AGEND_HOME HOME="$H" agend init; echo "exit=$?"
   ```

   應該看到：第一次 `created /tmp/g9h.…/.agend-v2 (0700)`、doctor 的輸出、`next: agend daemon …`，`exit=0`（git 太舊時是 1，看 doctor 那行）；第二次 `… looks like an AgEnD v1 home (fleet.yaml); set AGEND_HOME to another directory`、`exit=1`。`$H` 先留著，步驟 3 還要用。

   - [ ] 通過

3. `agend doctor`，再故意弄壞 git 版本。

   **這步在驗什麼**：每項檢查都有結果；壞掉的那項是 `fail`、附修正指令、exit 1；沒 instance 用的 backend 缺了只是 `warn`（P8）。錯了的話設定錯誤會像 v1 一樣「靜靜不動」。

   在同一個終端接著步驟 2（用同一個 `$H`；先拿掉步驟 2 放的 `fleet.yaml`，home 才會是 ok）：

   ```bash
   rm "$H/.agend-v2/fleet.yaml"
   env -u AGEND_HOME HOME="$H" agend doctor; echo "exit=$?"
   T=$(mktemp -d)
   printf '#!/bin/sh\necho "git version 2.30.0"\n' > "$T/git" && chmod +x "$T/git"
   env -u AGEND_HOME HOME="$H" PATH="$T:$PATH" agend doctor; echo "exit=$?"
   rm -rf "$T" "$H"
   ```

   應該看到：第一次每項一行（home、daemon、git、claude、codex、opencode、holders、disk）；home 是 `ok`；daemon 沒跑是 `warn` 附 `fix: agend daemon`；沒裝的 backend 是 `warn` 附安裝指令；`exit=0`（你機器的 git 本來就太舊時是 1）。第二次 git 那行 `fail  git  git 2.30.0 is older than 2.38 …`、下一行 `fix: …`、`exit=1`。

   - [ ] 通過

4. 啟動 daemon，用 `agend instance add` 加一個 agent，看 `status`。

   **這步在驗什麼**：daemon 跑著時也能加 instance，而且馬上啟動；操作者的 `status` 與 `instance list` 看得到它（P1、P6）。錯了的話加 instance 還是得停 daemon。

   第一個終端：

   ```bash
   export AGEND_HOME="$(mktemp -d /tmp/g9.XXXX)" && echo "export AGEND_HOME=$AGEND_HOME"
   agend daemon
   ```

   第二個終端（先跑開頭那段、貼上 `export AGEND_HOME=…`；假 agent 的路徑開工時細化）：

   ```bash
   ~/.cargo/bin/cargo build -q -p agend-testkit --bin fake-claude
   agend instance add g9-1 claude --program "$PWD/target/debug/fake-claude"
   agend instance list
   agend status
   ```

   應該看到：`added g9-1 (claude, session …, …/workspace/g9-1); starting`；daemon 那邊 `g9-1: start --session-id …`；`list` 一行 `g9-1  claude  …`；`status` 印 daemon 的 pid、版本、`client protocol 1.2`、`instances: 1`。

   - [ ] 通過

5. 故意弄壞：身分用錯。

   **這步在驗什麼**：agent 跑不了操作者命令、人跑不了 agent 命令，拒絕時都說該怎麼做；`--json` 的錯誤也是一個 JSON（P1、P4）。錯了的話 agent 能加減 instance、重啟 daemon。

   ```bash
   AGEND_INSTANCE=g9-1 agend status
   AGEND_INSTANCE=g9-1 agend instance add x claude; echo "exit=$?"
   agend done t-1/work/1; echo "exit=$?"
   agend done t-1/work/1 --json; echo "exit=$?"
   ```

   應該看到：第一行是 agent 自己的狀態（`g9-1 (claude): no task`、`next: …`）；第二行 `agend: forbidden: only the operator can add instances; ask the operator`、`exit=1`；第三行 `agend: forbidden: agend done is an agent command; …`、`exit=1`；第四行只有一行 `{"error":{"code":"forbidden",…}}`、`exit=1`。

   - [ ] 通過

6. 重啟 daemon 到同一個 binary。

   **這步在驗什麼**：D2：預檢通過才切換，切換後 daemon 同一個 pid、holder 沒換、agent 沒斷（P7）。錯了的話每次升級都要停掉所有 agent，或預檢形同虛設。

   ```bash
   pgrep -fl "agend holder g9-"
   agend daemon restart
   pgrep -fl "agend holder g9-"
   agend status
   ```

   應該看到：`preflight agend 0.0.0 (…)`、`db copy: … quick_check ok`、`holder: hello ok, spawn ok, shutdown ok`、`restarting the daemon (pid <D>) ...`、`the daemon is back: pid <D>, …`（同一個 D）；第一個終端的 daemon 印出新的開機紀錄、`recovered=1`；兩次 `pgrep` 同一個 pid；`status` 正常。`ls /tmp/agend-pf-*` 什麼都沒有。

   - [ ] 通過

7. 故意弄壞：重啟到一個壞掉的 binary。

   **這步在驗什麼**：預檢失敗時什麼都不切換，舊 daemon 照跑、DB 沒被動過（P7）。錯了的話壞掉的新版會把 daemon 換掉，agent 全部失聯。

   ```bash
   agend daemon restart --binary /usr/bin/false; echo "exit=$?"
   agend status
   ```

   應該看到：`agend: preflight_failed: /usr/bin/false daemon preflight exited with status 1; the daemon keeps running agend 0.0.0`、`exit=1`；`status` 的 pid 跟步驟 6 一樣；daemon 那邊沒有新的開機紀錄。（「DB 一個 byte 都沒動」由自動測試比對 sha256；daemon 跑著時會寫 WAL，這裡手動比不準。）

   - [ ] 通過

8. 里程碑：兩個 codex agent 互傳訊息，中途重啟 daemon。

   **這步在驗什麼**：ROADMAP 里程碑「第 1–9 施工關：兩個 codex agent 互傳訊息；重啟 daemon 時不中斷、不遺失、不重複」：`send` 經第 7 施工關的送達、斷線時用同一個 `message_id` 重送、daemon 靠 id 去重（P1、P5、P7）。錯了的話重啟 daemon 會掉訊息或重複送。

   操作（第二個終端；假 codex 的 `--program` 開工時細化，依第 7 施工關的啟動方式）：

   ```bash
   agend instance add g9-a codex --program "<假 codex>"
   agend instance add g9-b codex --program "<假 codex>"
   (for i in $(seq 1 10); do AGEND_INSTANCE=g9-a agend send g9-b "a$i"; AGEND_INSTANCE=g9-b agend send g9-a "b$i"; sleep 0.5; done) &
   sleep 2; agend daemon restart; wait
   AGEND_INSTANCE=g9-b agend inbox
   AGEND_INSTANCE=g9-a agend inbox
   ```

   應該看到：20 次 `send` 都成功（剛好碰上重啟的印 `(retried <t> s)`）；`restart` 照步驟 6 的樣子；g9-b 的 `inbox` 剛好 10 行 `from g9-a`（`a1`…`a10` 各一次），g9-a 的剛好 10 行 `from g9-b`（`b1`…`b10`），沒有重複；daemon log 每則都有 `confirmed`（確切字樣開工時細化）。手動不一定剛好碰到「請求送出、回應之前」的那一瞬間；那個情況由自動測試負責（「自動驗收」的兩個 instance 互送、中途重啟）。

   - [ ] 通過

9. 移除 instance、收尾。

   **這步在驗什麼**：`instance remove` 停掉 agent、刪掉那一列，但留下 workspace；非 TTY 沒有 `--yes` 不動手（P6）。錯了的話誤刪 agent，或把工作目錄一起刪了。

   ```bash
   agend instance remove g9-1 < /dev/null; echo "exit=$?"
   agend instance remove g9-1 --yes
   pgrep -fl "agend holder g9-1"
   ls "$AGEND_HOME/workspace/"
   ```

   應該看到：第一行拒絕（`… needs --yes when not on a terminal`）、`exit=2`；第二行 `removed g9-1; workspace kept at …/workspace/g9-1`；`pgrep` 什麼都不印（g9-a、g9-b 還在跑，所以只查 g9-1）；`ls` 還有 `g9-1`。再 `agend instance remove g9-a --yes`、`agend instance remove g9-b --yes`，之後 `pgrep -fl "agend holder g9-"` 什麼都不印；最後第一個終端 Ctrl-C 停 daemon，再 `rm -rf "$AGEND_HOME"`。

   - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-26 第 2 輪 review REFUTED（3 MEDIUM、數個 LOW）後修正：restart 先等舊連線 EOF、以 `hello` 的 `boot_id` 確認換了；client `message_id` 只收 UUID v4、同 id 內容不同回 `invalid_request`；步驟 9 的 `pgrep` 只查 g9-1；送給 claude／opencode 停在 `queued`；`--after` 依寫入順序；`ECHILD` 後不再查；步驟 8 雙向；KISS 兩項列為待你決定。
- 2026-09-26 補第 9、10 施工關分工（第 10 施工關負責 `ask` 與 task 類 handler、`workflow`／`team`；之前回 `not_supported`）；ticket 由第 10 施工關印在派工訊息；待你決定：操作者能不能 `task create`。
- 2026-09-26 fresh review REFUTED（5 MEDIUM、6 LOW）後修正：收屍只對接回的 holder pid `waitpid(pid, WNOHANG)`；`hello` 加 `daemon_pid`、全貌 instance 加 `working_directory`；步驟 3 改在步驟 2 的暫存 HOME 跑；本關接 `send` → `deliver` 與 `level`，第 7 施工關 merge 為硬前提、加步驟 8（兩個 agent 互傳、中途重啟）；TL;DR 與 P1、P3、P8 標出與已定文件不同之處；`daemon_probe` 定位、重加仍在跑的 id、restart 一次一個＋`mkdtemp`、`--binary` 在 D6 範圍內、協定版本寫成「實作時的下一個 minor」。
- 2026-09-26 開工前提案 P1–P10 寫定（draft PR），待使用者確認；`ask` 建議移到第 10 施工關（P1）、結果類命令帶 ticket（P2）、預設 home `~/.agend-v2`（P3）、restart 用原地 `exec`（P7）、`init` 不建 `config.toml` 與 agent（P9）；「你親自驗收」改成 9 步；狀態改為提案中。

## 下一步

```bash
cat docs/gates/gate-09-cli.md          # 逐題確認 P1–P10
~/.cargo/bin/cargo xtask accept cli    # 實作後：你親自驗收步驟 1
```
