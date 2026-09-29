# 第 9 施工關：agend CLI（`cli`）

> **TL;DR**
> - agent 命令、操作者命令（`instance add/remove/list`、`daemon restart`）、`status`、`doctor`、`init`；真 daemon 本關做 `status`、`send`、`inbox`（接第 7 施工關的送達，完成里程碑「兩個 codex agent 互傳訊息、重啟不漏不重」）與操作者命令，其他 agent 命令先對假 daemon 做完。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：已完成（2026-09-29：L1–L21 已追認、你親自驗收 9 步通過）；merge 後開第 10 施工關。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**完成**（2026-09-29，PR #136，branch `feat/gate-09-cli`）：P1–P10 照使用者確認的版本實作完成，自動驗收都有證據（見「自動驗收」）；L1–L21 已追認；你親自驗收 9 步通過（見「驗收紀錄」）。依賴都已 merge：第 7 施工關（`deliver`、`messages` 表）、第 8 施工關（client protocol 1.1、`agend-client`）。client protocol 升到 **1.2**（本關先 merge，第 10 施工關拿 1.3）。

## 範圍

- 11 個 agent 命令（D17）：CLI 全部做完、對假 daemon 驗；真 daemon 本關做哪些見 P1
- `agend send` → 第 7 施工關 `deliver` 的接線、client protocol `Send` 加 `level` 與 `message_id`（第 7 施工關提案 P9 交給本關）（P1、P5）
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
    | `done`、`result`、`review approve`／`changes`、`block`／`unblock`、`remind` | agent | `not_supported`，訊息寫「第 10 施工關」 |
    | `agend task create` | 兩者（操作者要帶 `--team`） | `not_supported`，handler 在第 10 施工關 |
    | `agend ask` | agent | `not_supported`，**移到第 10 施工關**（見下） |
    | `agend task cancel <task>` | 操作者 | `not_supported`，handler `task_cancel` 在第 10 施工關（使用者 2026-09-26 決定加入） |
    | `agend instance add`／`remove`／`list` | 操作者（`list` 唯讀，兩者都可） | 做（P6） |
    | `agend daemon restart` | 操作者 | 做（P7） |
    | `agend doctor`、`agend init` | 操作者 | 不需要 daemon（P8、P9） |

    已有的留著：`agend daemon`、`agend holder`、`--version`、`--help`；`agend debug ping/watch` 在第 8 施工關的實作 merge 後才有，本關不動它們。
  - 權限只在 daemon 擋（D17），CLI 不預先判斷：`command` 請求（agent 命令）只收 agent；會改東西的操作者請求（P6 的 `operator`、第 8 施工關的 `resolve_attention`、`terminal_input`）只收操作者；唯讀請求（`get_fleet`、`subscribe_*`）大家都能用。拒絕一律 `forbidden`，訊息附正確做法。
  - 第 9、10 施工關的分工：**本關負責**所有命令的 CLI 語法、client 接線、錯誤對應與重送規則（P2、P4、P5）；**第 10 施工關負責** `ask`（daemon 端與 store）、`done`／`result`／`review`／`block`／`unblock`／`remind`／`task create` 的 daemon handler、`agend workflow`／`agend team` 命令。這些 handler 在第 10 施工關之前，真 daemon 一律回 `not_supported`（訊息寫「第 10 施工關」），不做半套。第 10 施工關在派工訊息印本關的 ticket（P2），`inbox` 照本關的模型（唯讀游標、`--after`）。
  - **使用者決定（2026-09-26）**：操作者**也能**跑 `agend task create`：它同時是 agent 命令與操作者命令；操作者的形式必須帶 `--team`（agent 的 team 由 daemon 從身分推得）；daemon 的 handler 仍在第 10 施工關。操作者的形式不走只收 agent 的 `command` 請求，而是 P6 `operator` 請求的 `task_create` 變體（第 10 施工關連同 handler 一起加；在那之前 CLI 回 `not_supported`）。
  - KISS 選項（使用者 2026-09-26 決定**不套用**）：(a) 沒有真 handler 的命令（`done`、`result`、`review`、`block`／`unblock`、`remind`、`task create`、`ask`）的 CLI 語法與假 daemon 的列，整批移到第 10 施工關，跟 handler 一起做；(b) `init` 再縮成只建目錄（不跑 doctor）。兩者都留在本頁。
  - `agend task cancel <task>`（使用者 2026-09-26 從第 10 施工關決定）：操作者命令；本關做 CLI 語法、client 接線（P6 `operator` 請求的 `task_cancel` 變體）、錯誤對應；daemon handler 在第 10 施工關，之前回 `not_supported`。
  - `agend status` 一個命令兩種：有 `AGEND_INSTANCE` 送 `command {status}`，沒有送 `get_fleet`。
  - **`ask` 移到第 10 施工關**（**與第 8 施工關 P6「請示在第 9、10 施工關」不同，請明確決定**）：真的請示要新表、「需要你」來源、`answer_ask`、追問與結論（D35），本關做等於提前一半的第 10 施工關；第 11 施工關 B 段「回答請示」那半一樣等第 10 施工關。
  - 收件者是 claude／opencode（driver 在第 12 施工關）：`send` 照樣寫進 `messages` 表、狀態停在 `queued`；CLI 一樣回 `accepted`（`send` 一律回 `accepted`，core 的 `CommandResult` 沒有 `queued`；送達狀態看 daemon log、TUI）；收件者用 `agend inbox` 看得到；第 12 施工關的 driver 上線後照第 7 施工關的規則補送（實際行為以第 7 施工關 `deliver` 對沒有 driver 的 instance 怎麼做為準）。`CLI-n` 有一列驗它。
  - `send` 的 `level`：`agend send <to> "<message>" [--level queue|steer|interrupt]`，預設 `queue`（第 7 施工關 P6：等級由呼叫者傳入、預設 `Queue`）。`Send` 加選填的 `level` 與 `message_id`（additive，P5）。
- 理由：里程碑「第 1–9 施工關：兩個 codex agent 互傳訊息；重啟 daemon 時不中斷、不遺失、不重複」（[ROADMAP](../ROADMAP.md#里程碑使用者可見)）只需要 `status`、`send`、`inbox` 與加 instance，而第 7 施工關只做 `deliver` API、把 `agend send` 與 `level` 交給本關，所以接線由本關負責，本關結束時里程碑要真的跑得起來（步驟 8）；其他 agent 命令沒有 task 就沒有真資料可回，回明確錯誤比假資料好（第 8 施工關 P6 同一條路）。權限只放 daemon 一處，TUI、Telegram、CLI 同一套規則。
- 替代方案：本關就做 `ask` 的真實作（範圍多一張表與請示流程）；`send`／`inbox` 在第 7 施工關沒 merge 時先回 `not_supported`、里程碑延到第 10 施工關（**會讓 ROADMAP 的里程碑延後，要改的話請明確決定**；本頁不建議）；CLI 也擋一次權限（兩處規則會漂移）；agent 連唯讀的全貌都不能看（`agend instance list` 在 agent 裡就要另開例外）。
- 例子：操作者跑 `agend done t-1/work/1` → `agend: forbidden: agend done is an agent command; it runs inside an agent, where AGEND_INSTANCE is set`、exit 1；agent `g9-1` 跑 `agend instance add x claude` → `agend: forbidden: only the operator can add instances; ask the operator`、exit 1。
- [x] 使用者確認（2026-09-26）

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
    | inbox | `agend inbox [--after <message-id>]`（不帶：最近 20 則）；「之後」依第 7 施工關 `messages` 表明確的 `seq INTEGER PRIMARY KEY`（寫入順序；不用隱含的 rowid，`VACUUM` 可能重編），不比 UUID；`--after` 回那一則之後、寄給自己的**全部**訊息（不設上限；保留期限 30 天本身就是上限，D31）；`--after` 的 id 不存在、已過保留期限，或不是寄給呼叫者自己的 → 一律 `unknown_message: … run agend inbox without --after`、exit 1（`CLI-n` 有一列）；第 10 施工關的假 agent 用同一個規則 |
    | done | `agend done <ticket>` |
    | result | `agend result <ticket> "<summary>"` |
    | review | `agend review approve <ticket>`、`agend review changes <ticket> "<what to change>"` |
    | ask | `agend ask "<question>" [--option <text>]…`；追問 `--follow-up <ask-id>`、結束 `--resolve <ask-id>` |
    | block | `agend block "<reason>"`、`agend unblock` |
    | task create | `agend task create --role <role> "<title>" [--team <team>] [--workflow <id>]`；操作者跑時 `--team` 必填 |
    | remind | `agend remind <delay>`（`90s`、`30m`、`2h`） |

  - **與 [tui-and-setup](../architecture/tui-and-setup.md#agent-介面clid7d17)「daemon 推得 task_id」字面不同，請明確決定**：ticket 裡有 task id；推不得的是 attempt，task id 只是順便帶著。
- 理由：第 1 施工關的事件身分（每個結果帶 attempt，過期一律 `stale_result`）是 verifier 推翻四輪才定下的結構；在 CLI 或 daemon 補上「現在的」attempt 會把這層保護拿掉。一個 ticket 參數仍然符合 ≤ 2 個參數。
- 替代方案：daemon 從呼叫者目前的 assignment 補 `task_id` 與 attempt（agent 最簡單；晚到的核准會算在新的 head 上）；CLI 先問 `status` 再補（同樣的問題，只是在 CLI 做）；分開三個旗標 `--task --stage --attempt`（參數太多，agent 容易抄錯一個）。
- 例子：`agend status` → `t-42 · review (attempt 2) · ticket t-42/review/2`、`next: agend review approve t-42/review/2 | agend review changes t-42/review/2 "<what to change>"`；attempt 3 開始後才送 `agend review approve t-42/review/2` → `agend: stale_result: t-42/review/2 is no longer current (now t-42/review/3); run agend status`、exit 1。
- [x] 使用者確認（2026-09-26）

### P3：`AGEND_HOME` 怎麼找

- 問題：第 6 施工關的 daemon 必須設 `AGEND_HOME`，預設值留給第 9、13 施工關。CLI 要用同一個 home 算出 socket。要不要有預設？會不會踩到 v1？
- 決定（**使用者 2026-09-26 改寫**；原本建議預設 `~/.agend-v2`）：
  - **`AGEND_HOME` 一律必須設**，本關沒有預設值。沒設 → `AGEND_HOME is not set; choose a directory for AgEnD's data and run: export AGEND_HOME=<absolute path>`、exit 2。必須是絕對路徑，否則 `AGEND_HOME must be an absolute path`、exit 2。
  - 一個函式 `agend` 的 `home::resolve()`，CLI、`agend daemon`、`doctor`、`init` 都用它（第 6 施工關 P1 的「沒設就拒絕」照舊，訊息統一成上面那句）。holder 與 agent 照舊由 daemon 明確設 `AGEND_HOME`（第 6 施工關 H3），agent 裡的 CLI 自然連到同一個 daemon。
  - v1 防護：home 裡有 `fleet.yaml`（v1 的檔）就拒絕：`<path> looks like an AgEnD v1 home (fleet.yaml); set AGEND_HOME to another directory`、exit 1。
  - 不做 `--home` 旗標。
  - **延到第 13 施工關**：預設位置（例如 `~/.agend`）、`config.toml` 要不要列 home 路徑（D8／[tui-and-setup](../architecture/tui-and-setup.md#設定與目錄d8) 的寫法）。
- 理由：沒有預設就不可能意外寫進 v1 的 `~/.agend`（里程碑之後要與 v1 並行一週）；預設位置是安裝的事，等第 13 施工關有服務註冊時一起定，不必現在挑一個之後要搬家的名字。
- 替代方案：預設 `~/.agend-v2`（原本的建議）；`~/.agend` 加 v1 防護（並行那週一定被擋）；XDG 路徑。
- 例子：沒設 `AGEND_HOME` → `agend status` 印上面那句、exit 2；`AGEND_HOME=~/.agend agend daemon`（v1 home）→ 被拒、exit 1。
- [x] 使用者確認（2026-09-26，使用者改成「一律必須設」）

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
  - 新錯誤碼（core 常數，跟第 8 施工關的放一起）：`unknown_instance`、`instance_exists`、`preflight_failed`、`unknown_message`。
  - 參數解析用 `clap`（derive）；`--help` 先列範例（`before_help`）。
- 理由：agent 讀 exit code 只需要分「成功／失敗／我打錯」；更細的分類放 JSON，exit code 就不用當成第二套錯誤碼維護。`--json` 用 protocol 型別，只有一份格式、一套版本規則（#1493）。
- 替代方案：每種錯誤一個 exit code（連不上 = 3 之類；兩套錯誤碼要同步）；`--json` 的錯誤寫 stderr（agent 要讀兩個串流）；手寫參數解析（約 15 個命令、每個都要自己處理 `--help` 與錯誤）。
- 例子：`agend status --json`（操作者）→ `{"as_of_event_id":…,"teams":[…],"instances":[…],"attention":[…]}`、exit 0；daemon 沒在跑：`agend status --json` → 10 秒後 `{"error":{"code":"daemon_unreachable","message":"cannot reach …"}}`、exit 1。
- [x] 使用者確認（2026-09-26）

### P5：daemon 重啟時，哪些命令斷線後重送

- 問題：第 8 施工關 P7 定了：**請求送出後**斷線，只有呼叫端標「可重做」的才重送，逐個命令交給本關決定。
- 建議：規則一句話：**唯讀的重送；會改東西的只有 `send` 重送（靠訊息 id 去重）**，其他不重送、印出該用哪個命令確認。

  | 命令 | 重送 | 理由／不重送時的檢查指令 |
  |---|---|---|
  | `status`、`inbox`、`instance list`、`doctor` 裡的查詢 | 是 | 唯讀（`inbox` 帶游標讀，不消耗訊息） |
  | `send` | 是 | CLI 產生 `message_id`（UUID v4），`Send` 加選填欄位（additive），daemon 把它當成 `deliver` 的訊息 id；第 7 施工關 P5 的 `messages` 表的 id 為 UNIQUE（`seq` 為主鍵）、`deliver` 先查表，同一個 id 再送回目前的狀態、不再送一次——這就是唯一的一套冪等。daemon 在 `deliver` 寫入 DB **之後**才回應；「同 id、內容不同」的檢查與 `deliver` 的寫入在同一個 DB thread 的同一個 closure 裡做，沒有競態。**client 的 id 跟別人分開**：daemon 只收 UUID v4 格式的 `message_id`（第 10 施工關的派工訊息用 `dispatch:<ticket>`，不是 UUID，撞不到）；表裡已有這個 id、但 `from_instance`／`to_instance`／body／level 有任何一個不同 → `invalid_request: message id … is already used by another message`，不當成去重、不回 `accepted`。沒帶 `message_id` 的 `Send`（1.1 的 client）由 daemon 產生 UUID |
  | `done`、`result`、`review …` | 否 | 重送會被當成過期（`stale_result`），訊息反而誤導；檢查 `agend status` |
  | `block`／`unblock`、`remind`、`task create`、`ask` | 否 | 會重複建立或覆蓋；檢查 `agend status` |
  | `task cancel` | 否 | 取消後再送會回錯誤或重複取消；檢查 `agend status` |
  | `instance add`／`remove` | 否 | 重送會回 `instance_exists`／`unknown_instance`；檢查 `agend instance list` |
  | `daemon restart` | 不適用 | 斷線本來就是預期的（P7），CLI 改等新 daemon |

- 理由：里程碑要求「重啟 daemon 時訊息不遺失、不重複」：只有 `send` 同時需要「不遺失」（agent 以為送出了）與「不重複」，用它自己的 id 去重是唯一兩者都做得到的路，而且就是送達模型已經有的那套冪等（V1-LESSONS #1：不再多一套去重）。
- 替代方案：全部不重送（`send` 斷在中間時 agent 只能猜，重下一次就可能重複）；daemon 依 `request_id` 去重（要跨重啟記住，第 8 施工關 P7 已否決）；`block`／`unblock` 當成冪等也重送（第 10 施工關有真實作再說）。
- 例子：`agend send g9-2 "hi"` 送出後 daemon 重啟 → CLI 等 1.4 秒、用同一個 `message_id` 重送 → daemon 看到已經有這個 id，回 `accepted`，`g9-2` 只收到一次；`agend instance add g9-3 claude` 斷在中間 → `agend: daemon restarted during the request; check with agend instance list`、exit 1。
- [x] 使用者確認（2026-09-26）

### P6：操作者命令：`instance`、協定的下一個 minor

- 問題：client protocol 只有給 agent 的 `command`。加減 instance、重啟 daemon 要怎麼送？`instance add` 要哪些參數？`daemon_probe` 怎麼辦？
- 建議：
  - 新請求 `operator { request_id, command: OperatorCommand }`，`OperatorCommand` 有 `instance_add`、`instance_remove`、`daemon_restart`（P7）與 `unknown`；成功回現有的 `command_result`：`accepted`，或新的 `instance_added { instance_id, session_id, working_directory }`、`restarting { preflight }`。`instance list` 與操作者的 `status` 用第 8 施工關的 `get_fleet`，不另開請求。
  - 本關所有新增算一次 minor：client protocol 用**實作時的下一個 minor**（目前是 1.2；第 10 施工關的提案也要加一個 minor，兩關誰先 merge 誰拿 1.2，本頁的 1.2 都照這條換）。
  - 本關加的欄位（全部選填、additive）：`hello` 回應加 `daemon_version`（`agend 0.x.y`）、`daemon_pid`、`boot_id`（P7）；全貌的每個 instance 加 `working_directory`；`Send` 加 `level`、`message_id`（P1、P5）；`StatusData` 加 `identity`（P2）。`status`、`restart`、`doctor` 印的 pid 與版本、`instance list` 的 DIR 欄都從這裡來。
  - **`daemon_restart` 的形狀之後不再改**：舊 daemon 要能聽懂新 CLI 叫它重啟，否則升級時「重啟到新版」這條路會斷。
  - **用詞（使用者 2026-09-26 決定）**：對使用者一律叫 **`name`**（命令參數、`--help`、訊息、表頭）；協定與 DB 的欄位照舊叫 `instance_id`（D26，不改 wire）。`--help` 寫 `<name>  agent name you choose: a-z 0-9 -, up to 24 chars (e.g. dev-1)`。
  - `agend instance add <name> <backend> [--dir <path>] [--program <path>] [-- <args>…]`：name 規則 `[a-z0-9-]{1,24}`（第 6 施工關 P2 的 id 規則）；已經有同名的 → `instance_exists: name dev-1 is already used`；`--dir` 預設 `$AGEND_HOME/workspace/<name>`（daemon 建）；`--program` 預設就是 backend 名（claude／codex／opencode，從 agent 的 `PATH` 找），測試與驗收用它指向假 agent。daemon 寫入 `new`、claude 產生 session id（第 6 施工關 H2），**馬上啟動**，回傳。team 一律 `general`（D12；本關沒有 team 表）。同名的 holder 鎖還被持有（剛 `remove`、holder 還沒結束，或留著的孤兒）→ 拒絕：`instance_exists: name g9-1 is already used: its holder is still running (run/holders/g9-1.lock); wait a few seconds or restart the daemon to sweep it`，不寫 DB。
  - `agend instance remove <name> [--yes]`：TTY 上問 `remove g9-1? its agent is stopped; the workspace is kept [y/N]`；非 TTY 必須 `--yes`（否則 exit 2）。daemon 送 `Shutdown` 給 holder（最多等 5 秒）、刪那一列、發 `instance_changed`。holder 連不上也照樣刪，下次開機的孤兒巡查會收掉它（第 6 施工關 P2）。**workspace 不刪**，印出路徑。
  - `agend instance list`：表頭 `NAME  BACKEND  STATE  DIR`，一行一個。
  - `agend instance` 是**正式**命令；`daemon_probe` 不刪，降為開發工具（daemon 停著時直接改 DB；`--dies` 與第 6、8 施工關的驗收步驟在用）。第 9 施工關之後的文件與驗收一律用 `agend instance`。
- 理由：寫 DB 的路只有 daemon 一條，daemon 跑著時也能加減 instance（`daemon_probe` 做不到）；一個 `operator` 請求加一個 enum，之後的 team、workflow 命令只加變體。
- 替代方案：每個操作者命令一種請求（請求種類一直變多）；把操作者命令塞進 `AgentCommand`（權限規則要逐個變體判斷）；`instance add` 在 daemon 沒跑時直接開 DB（兩條寫入路徑；CLI 路徑不開 DB，D11）；刪 instance 時一起刪 workspace（刪資料前要問，v1 教訓）。
- 例子：`agend instance add g9-1 claude --program "$PWD/target/debug/fake-claude"` → `added g9-1 (claude, session 3f…, $AGEND_HOME/workspace/g9-1); starting`；`agend instance list` → `NAME  BACKEND  STATE  DIR`、`g9-1  claude  starting  $AGEND_HOME/workspace/g9-1`。
- [x] 使用者確認（2026-09-26）

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
    6. CLI 印預檢結果，**先等這條連線 EOF**（daemon 收尾時關掉它，代表舊的已經不聽了；最多等 30 秒，逾時報 `the daemon accepted the restart but did not stop within 30 s; check the daemon's terminal or log`、exit 1），才用 `Client::connect`（10 秒重試）連新 daemon；連上後比 `hello` 的 `boot_id`，**必須跟重啟前不同**才印 `the daemon is back`，否則繼續重試到 10 秒後報錯。pid 與版本在同一個 binary 的 `exec` 前後都一樣，不能拿來判斷。
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
- [x] 使用者確認（2026-09-26）

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
  ok    home      /Users/you/agend-home (0700)
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
- [x] 使用者確認（2026-09-26）

### P9：`agend init` 本關做到哪

- 問題：[tui-and-setup](../architecture/tui-and-setup.md#安裝與設定) 的 `init` 要建 home、`config.toml`、註冊服務、偵測 backend、建 `general` team 與一個 agent、在 repo 裡問要不要登記、最後跑 doctor。本關做哪些？原本步驟 4 的 `--non-interactive` 要不要？
- 建議：
  - 本關的 `init`：照 P3 取 `AGEND_HOME`（沒設就停，exit 2）→ 建目錄（0700；已存在就不動，印 `already exists`）→ 跑 `doctor` → 印下一步（`agend daemon`、`agend instance add <name> <backend>`）。可以重跑。v1 home 拒絕（P3）。
  - **不問任何問題**，所以本關**不做 `--non-interactive`**：第一個問題出現的施工關（第 13 施工關的服務註冊）再加。
  - **不建 `config.toml`**：本關沒有任何程式讀它（Telegram 在第 12 施工關）；骨架規則是不寫佔位的東西。第一個讀它的施工關負責建。
  - **不建 agent**：寫 instance 只經 daemon（P6），而 `init` 時 daemon 還沒跑；印出下一步的指令。`general` 本來就是內建的（D12），本關沒有 team 表可寫。
  - 不註冊服務、不偵測後問 backend、不登記 repo（第 13、10 施工關）。
  - **與 tui-and-setup 與原本步驟 4 的「建 `config.toml`、`general` team 與一個 agent」不同，請明確決定**。
- 理由：`init` 做的每一件事都要有人用得到；只建目錄加跑 doctor，第一次使用前就能指出設定錯誤（V1-LESSONS #14），其餘等有使用者的施工關再加，不留假的檔案。
- 替代方案：寫一份只有註解的 `config.toml`（沒人讀）；`init` 自己開 DB 加一個 agent（第二條寫入路徑）；`init` 順便在背景起 daemon（v1 的自動起 daemon，D2 否決）。
- 例子：`AGEND_HOME=/tmp/g9h.x/home agend init` → `created /tmp/g9h.x/home (0700)`；沒設 `AGEND_HOME` 時印 P3 那句、exit 2。接著 doctor 的輸出，最後 `next: agend daemon   (then, in another terminal) agend instance add dev-1 claude`。
- [x] 使用者確認（2026-09-26）

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
- [x] 使用者確認（2026-09-26）

### 本關不做（明確列出）

- 真 daemon 的 task 類命令與 `ask` 的 handler、store（第 10 施工關，P1；在那之前回 `not_supported`）；`send`／`inbox` 的送達本身（第 7 施工關）。
- `agend workflow …`、`agend team …`（D19；第 10 施工關，連同第 5 施工關 S1 的 `save_workflow_toml`）。
- `agend daemon stop`（在 launchd／systemd 下「停」要先卸載服務，第 13 施工關一起做）；`export`／`import`、`telegram setup`、`uninstall`（第 13 施工關）。
- doctor 檢查 checks 的寫入沙箱工具（macOS `sandbox-exec`、Linux `bwrap`）：第 10 施工關的 checks 在沙箱裡跑、找不到就拒絕（fail closed），doctor 之後要報告它在不在；本關不做。**已認領**：第 10 施工關 P6 決定這一列由第 10 施工關自己加（見 [gate-10-pipeline P6](gate-10-pipeline.md#p6command-關卡runner)）。
- 從 CLI 處理「需要你」（`resolve_attention` 由 TUI 做，第 11 施工關 B 段）。
- `config.toml`、`--non-interactive`、服務註冊（P9）；doctor 的登入、版本範圍、gh、服務、Telegram 檢查（P8）。
- 顏色、shell completion、MCP 轉接層（D7「需要時」）、Windows。

### 已知風險（開工時處理）

- **實作後補（verifier F4）：`agend daemon restart --binary <path>` 等於任何同 uid 的程序都能把 daemon 換成任意程式，這在 D6「同 uid 只是安全帶」的信任模型下是接受的。** 三個原因疊在一起：(1) 操作者的身分只是 `hello` 裡沒有 `caller`（第 8 施工關 P2），agent 裡 `env -u AGEND_INSTANCE agend …` 就是操作者；(2) 預檢只看新 binary 自己印的步驟（L5），一個印出那三行的 script 就能通過；(3) `exec` 用路徑再找一次 binary。本關做了便宜的那一半：預檢結束時 binary 必須還是預檢開始前的同一個檔案（device、inode、大小、修改時間、change time 都沒變；change time 一般使用者改不回去，第 2 輪 verifier #3），否則 `preflight_failed: … changed while its preflight ran`；從那一刻到 `exec` 之間仍有很短的空窗（macOS 沒有 `fexecve`，不硬做）。真正的防線是 D6 之外的東西（例如 agent 跑在不同 uid 或沙箱裡），不在本關。
- **實作後補（第 2 輪 verifier #1，第 7 施工關的 link 在本 PR 修）**：codex link 的寫入最多等 10 秒沒有進度就當連線壞了，走既有的重連；關閉 link 時先 shutdown 它的 socket，最多等 5 秒 thread 結束（之後放著讓它自己結束並記一行 log），所以 `instance remove`、Ctrl-C、`daemon restart` 都不會被一個不讀的 app-server 卡住。**留下的限制**：一個永遠不讀、但連線照樣連得上的 app-server，link 會一直「寫逾時 → 重連 → 再送 → 再逾時」，訊息停在 `queued`，不會變成 `failed`（既有的 `failed` 只在 20 秒都連不上時觸發）；daemon 本身保持可用。真的 codex app-server 是 tokio 程式、每條連線讀寫分開，照設計會一邊寫一邊讀（沒有實跑驗證，本關不准跑真 codex）；假 app-server 原本寫的時候不讀、不忠實，已改成讀寫交錯（`--disable duplex-io` 保留舊行為，當「不讀的 peer」測試用）。
- **實作後補（verifier F1、F2）**：預檢的暫存 home（`agend.db` 的完整複本，含訊息內容）與預檢子程序由一個 Drop guard 管：預檢完成、失敗、逾時，或 daemon 停止時連線的 task 被 abort，都會 kill 並收屍子程序、對還在暫存 home 裡的 `pf-check` holder 送 `Shutdown`、刪掉暫存 home；60 秒逾時不等子程序的 stdout 關閉（子程序留下的背景程序可能一直開著它）。

- 第 8 施工關還在實作：本關從它 merge 後的 `v2` 開 branch；1.1 的型別名稱、錯誤型別以 merge 後的程式為準，本關的 minor 疊在上面（P6：實作時的下一個 minor，與第 10 施工關的提案不要撞號）。
- 第 7 施工關的提案還在審（`docs/gate-07-proposal`）：`deliver(…, level)` 的簽名、`messages` 表的欄位、「同 id 再送回目前狀態」的行為以它 merge 後為準；**第 7 施工關 merge 是本關開工的硬前提**（P1）。它若改成不以 id 冪等，P5 的 `send` 重送就不成立，要回來重新決定。
- `inbox --after` 依賴第 7 施工關 `messages` 表的 `seq INTEGER PRIMARY KEY`：第 7 施工關的提案已經加上，以它 merge 後的版本為準；merge 後若沒有，`--after` 的順序要重新決定。錯誤碼 `unknown_message` 跟 P4 的新錯誤碼放一起。
- 步驟 8 的假 codex instance 怎麼起（`--program` 指向什麼、要不要包裝）依第 7 施工關的啟動方式，開工時細化。
- `exec`（P7）：所有 fd 要有 `CLOEXEC`（Rust 預設有；SQLite 的檔案要實測），否則新 daemon 會繼承舊的 DB 鎖；`exec` 失敗（預檢之後 binary 被刪）時 daemon 已經收尾，只能印錯誤、exit 1——前景要人重跑 `agend daemon`，第 13 施工關由服務管理器重起；launchd／systemd 下 `exec` 的行為在第 13 施工關實測。
- doctor 看鎖（P8）：檢查的那一瞬間若剛好有 holder 在啟動，可能拿不到鎖而啟動失敗；用 `LOCK_SH | LOCK_NB` 並立刻放掉，實測機率。
- ticket（P2）決定了第 10 施工關的派工訊息格式；第 10 施工關若要改，CLI 語法跟著改。
- 本關沒有預設 home（P3）：每個終端都要 `export AGEND_HOME`；預設位置與 `config.toml` 是否列 home 由第 13 施工關決定。
- `clap` 讓 binary 變大、編譯變慢；啟動時間超過 10 ms 就改手寫。

## 自動驗收（完成定義）

- [x] `~/.cargo/bin/cargo test -p agend` 單獨通過，包括：`CLI-n` 表每列對假 daemon、真 daemon 支援的列也對真 `agend daemon`（P10）；`home::resolve` 沒設（exit 2 與 export 提示）、相對路徑、v1 home（P3）；`--json` 只印一個 JSON 值、錯誤也在 stdout（P4）；`send` 送出後斷線用同一個 `message_id` 重送、其他會改東西的命令不重送（P5）；非 TTY 的 `instance remove` 沒有 `--yes` 回 exit 2（P6）；`daemon restart` 成功（pid 不變、holder 不變）與 `--binary` 失敗（daemon 照跑、DB 檔 sha256 不變）、同時兩個 restart 第二個被拒、CLI 在舊連線 EOF 且 `boot_id` 變了才報成功、重用別則訊息的 `message_id` 回 `invalid_request`、接回的 holder 死掉後不留殘屍、自己起的 holder 與預檢子程序的 exit status 沒被搶（P7）；兩個假 codex instance 互送 10 則訊息、中途 `daemon restart`：每則恰好送達一次、狀態 `confirmed`（P1、P5，里程碑）；`doctor` 每個 `fail`／`warn` 都有 `fix:`（P8）
- [x] `~/.cargo/bin/cargo test -p agend-core`、`-p agend-client`、`-p agend-daemon`、`-p agend-testkit` 單獨通過：1.1 的 peer 解得開本關 minor（目前 1.2）的訊息；`CLP` 新增的列對假 daemon 與真 daemon 都通過，每條有 mutant（P6、P10）
- [x] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [x] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過）
- [x] `~/.cargo/bin/cargo xtask accept cli` 通過，並印出下方「你親自驗收」用到的 demo 與 `agend --version` 的啟動時間（p50 < 10 ms）
- [x] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新；想改的共用文件（名詞表：ticket、`operator` 請求、使用者看到的 `name`＝`instance_id`；tui-and-setup 的 init；第 8、10、13 施工關頁）列在 PR 裡由你決定
- [x] 測試不留殘留：沒有 `g9-`、`pf-check` 或測試 id 的 holder，沒有 `/tmp/agend-pf-*`；kill 只對自己起的、大於 1 的 pid
- [x] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」——第 1、2 輪 REFUTED，全修；第 3 輪 2026-09-29 CONFIRMED（add6d10）

證據（實作者 2026-09-28，macOS）：`cargo xtask accept cli` 對 agend-core、agend-client、agend-daemon、agend-testkit、agend 各自跑 fmt、clippy、`cargo test -p <crate>` 都通過，`check-deps: ok (… no-std build ok)`，demo `cli demo: all sections passed`、`startup: agend --version p50 5.2 ms`、`gate 9 (cli): checks passed`；`cargo test --workspace` 全過；`cargo xtask accept codex`／`client`／`daemon-holder`（回歸）通過；跑完 `pgrep -fl "agend (holder|daemon)"` 沒有任何輸出、沒有 `/tmp/agend-pf-*`。對照：P3 → CLI-2..4、`init_and_doctor`；P4 → CLI-5、6、11、12、`unreachable_daemon_after_ten_seconds`；P5 → `a_lost_send_is_resent_with_its_id_and_nothing_else_is`、CLP-17；P6 → CLI-18..24、CLP-14；P7 → `restart_keeps_the_pid_and_the_holders`、`a_failed_preflight_changes_nothing`（`agend.db` 位元組完全相同，比 sha256 更嚴）、`one_restart_at_a_time`、`restart_waits_for_eof_and_a_new_boot_id`、`inherited_holders_are_reaped_and_nothing_else_is`；重用別則訊息的 `message_id` → CLP-17（`invalid_request`）；里程碑 → `milestone_two_codex_agents_across_a_restart`；P8 → `init_and_doctor`（`--json` 每個非 ok 都有 `fix`）；1.1 的 peer 解得開 1.2 → `xtask/tests/protocol_compat.rs::a_1_1_peer_decodes_1_2_messages`；CLP-13..17 → 假 daemon（`contract_fakes`）與真 daemon（`client_protocol.rs`）都 17／16 條全過，`contract_teeth` 每個 mutant 都被抓到。

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。下面的「應該看到」是實作者 2026-09-28 在 macOS 照抄指令實跑的輸出（暫存目錄名、pid、session id、訊息 id 每次不同；實作者的 `PATH` 沒有 claude／codex／opencode，所以那三行是 `warn`）。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
unset AGEND_BIN AGEND_HOME AGEND_INSTANCE   # 前幾關留下的 export 可能指到已刪除的目錄
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.0.0`。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

步驟 4 起用同一個暫存 home。第二個終端也要貼上步驟 4 印出的那行 `export AGEND_HOME=…`。

**注意**：`agend doctor`（步驟 2、3）會對 `PATH` 上找到的 claude、codex、opencode 跑一次 `--version`（P8）。所有 `agend instance add` 都帶 `--program` 指向假 agent；**不要省略 `--program`**，否則會啟動你真的 claude／codex。

1. 跑 demo。

   **這步在驗什麼**：每個命令對假 daemon 的輸入、輸出、exit code，以及真 daemon 支援的那些命令兩邊結果一樣；重送、重啟、預檢、`init`／`doctor`、里程碑都跑一遍（P1、P4、P5、P7、P8、P10）。錯了代表 agent 讀到的輸出跟真 daemon 給的不一樣。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo xtask accept cli
   ```

   要跑十幾分鐘（前面是 5 個 crate 的 fmt／clippy／測試，最後約 2 分鐘是 demo）。應該看到：

   | 段落 | 要找的字 |
   |---|---|
   | `== contract (gate 9 rules)` | `CLP-13 fake ok · real ok` … 到 `CLP-17`，五行 |
   | `== CLI-n` | 每一列：`CLI-n  $ …` 命令、輸出、`exit N · fake ok · real ok`（不需要 daemon 的列 `· ok`；CLI-33..35 `· real ok`；CLI-36..46 `· fake ok (fake only; the real daemon's handler arrives in gate 10)`）；拒絕的訊息都附正確做法（例如 CLI-11 `… it runs inside an agent, where AGEND_INSTANCE is set`） |
   | `== resend` | `(retried … s)`、`the daemon got the send twice with the same message id …; g9-b has it once`、`agend: daemon restarted during the request; check with agend instance list`、`instance_add reached the daemon once and was not sent again` |
   | `== restart` | `the daemon is back: pid <D>, …`、`same daemon pid <D>, new boot id; holder of g9-r still pid …`、`its preflight home /tmp/agend-pf-… is gone` |
   | `== preflight` | 四個 `agend: preflight_failed: …`（`status 1`、`status 3: db copy: broken on purpose`、`without reporting its steps`、`cannot run`），然後 `agend.db byte-identical …; same pid … and boot id; no exec` |
   | `== one restart at a time`、`== restart waits` | `a restart is already in progress`；`did not stop within 30 s`（約 30 秒）、`did not come back within 10 s`、最後 `the daemon is back` |
   | `== after exec` | `reaped inherited holder pid …`、`(no zombie)`、`own holder g9-o: holder g9-o (pid …) exited: exit status: 0`、`preflight child: preflight_failed: /usr/bin/false daemon preflight exited with status 1; …` |
   | `== init and doctor` | 見步驟 2、3（這裡用的是 stub 的 git 與 claude） |
   | `== milestone` | `a5 after the restart: accepted: … (retried … s)`、兩行 `agend inbox of …: 10 lines from …, each once`、`all 20 messages confirmed`、兩行 `10 rows, all confirmed; each message exactly once in its codex thread` |
   | 最後 | `startup: agend --version p50 <n> ms (limit 10 ms)`（實測 1.9–5.2 ms，機器忙時較慢）、`running now: 0`、`/tmp/agend-pf-*: 0`、`cli demo: all sections passed`，然後 `gate 9 (cli): checks passed` |

   - [x] 通過

2. `agend init`：先故意不設 `AGEND_HOME`，再在暫存目錄建，最後故意弄壞成 v1 home。

   **這步在驗什麼**：沒設 `AGEND_HOME` 時什麼都不建、直接說要 export 什麼；看起來像 v1 的 home 一律拒絕（P3、P9）。錯了的話 v2 可能寫進你正在用的 v1 目錄。

   ```bash
   H=$(mktemp -d /tmp/g9h.XXXX)
   env -u AGEND_HOME agend init; echo "exit=$?"
   AGEND_HOME="$H/home" agend init; echo "exit=$?"
   touch "$H/home/fleet.yaml"
   AGEND_HOME="$H/home" agend init; echo "exit=$?"
   ```

   應該看到（`$H` 先留著，步驟 3 還要用）：

   ```text
   agend: AGEND_HOME is not set; choose a directory for AgEnD's data and run: export AGEND_HOME=<absolute path>
   exit=2
   created /tmp/g9h.Ab12/home (0700)
   ok    home      /tmp/g9h.Ab12/home (0700)
   warn  daemon    not reachable: cannot reach the AgEnD daemon at /tmp/g9h.Ab12/home/run/daemon.sock (No such file or directory (os error 2))
         fix: agend daemon
   ok    git       git 2.39.5
   warn  claude    not on PATH; no instance uses it
         fix: npm install -g @anthropic-ai/claude-code
   warn  codex     not on PATH; no instance uses it
         fix: npm install -g @openai/codex
   warn  opencode  not on PATH; no instance uses it
         fix: npm install -g opencode-ai
   ok    holders   0 running
   ok    disk      679 GB free, home 0 KB
   no check failed (4 warnings)
   next: agend daemon   (then, in another terminal) agend instance add dev-1 claude
   exit=0
   agend: /tmp/g9h.Ab12/home looks like an AgEnD v1 home (fleet.yaml); set AGEND_HOME to another directory
   exit=1
   ```

   你裝了的 backend 那一行是 `ok    claude    <它的 --version 第一行>`；git 太舊時 git 那行是 `fail`、`exit=1`。

   - [x] 通過

3. `agend doctor`，再故意弄壞 git 版本。

   **這步在驗什麼**：每項檢查都有結果；壞掉的那項是 `fail`、附修正指令、exit 1；沒 instance 用的 backend 缺了只是 `warn`（P8）。錯了的話設定錯誤會像 v1 一樣「靜靜不動」。

   在同一個終端接著步驟 2（用同一個 `$H`；先拿掉步驟 2 放的 `fleet.yaml`，home 才會是 ok）：

   ```bash
   rm "$H/home/fleet.yaml"
   AGEND_HOME="$H/home" agend doctor; echo "exit=$?"
   T=$(mktemp -d)
   printf '#!/bin/sh\necho "git version 2.30.0"\n' > "$T/git" && chmod +x "$T/git"
   AGEND_HOME="$H/home" PATH="$T:$PATH" agend doctor; echo "exit=$?"
   rm -rf "$T" "$H"
   ```

   應該看到：第一次跟步驟 2 的 doctor 那 11 行一樣、`exit=0`；第二次只有 git 那行變了、最後一行也變了：

   ```text
   fail  git       git 2.30.0 is older than 2.38 (merge-tree needs it)
         fix: brew install git
   …
   1 check failed
   exit=1
   ```

   - [x] 通過

4. 啟動 daemon，用 `agend instance add` 加一個 agent，看 `status`。

   **這步在驗什麼**：daemon 跑著時也能加 instance，而且馬上啟動；操作者的 `status` 與 `instance list` 看得到它（P1、P6）。錯了的話加 instance 還是得停 daemon。

   第一個終端：

   ```bash
   export AGEND_HOME="$(mktemp -d /tmp/g9.XXXX)" && echo "export AGEND_HOME=$AGEND_HOME"
   agend daemon
   ```

   應該看到（daemon 留在前景）：`listening on /tmp/g9.Xy34/run/daemon.sock`、`agend daemon ready: instances=0 recovered=0 started=0 orphans=0`。

   第二個終端（先跑開頭那段、貼上 `export AGEND_HOME=…`）：

   ```bash
   ~/.cargo/bin/cargo build -q -p agend-testkit --bin fake-claude
   agend instance add g9-1 claude --program "$PWD/target/debug/fake-claude"
   agend instance list
   agend status
   ```

   應該看到：

   ```text
   added g9-1 (claude, session 29d836f9-5085-4e57-859a-d988e122b948, /tmp/g9.Xy34/workspace/g9-1); starting
   NAME  BACKEND  STATE     DIR
   g9-1  claude   starting  /tmp/g9.Xy34/workspace/g9-1
   daemon: pid 24280, agend 0.0.0, client protocol 1.2
   instances: 1 (g9-1 starting)
   tasks: 0
   needs you: 0
   ```

   `STATE` 也可能已經是 `unknown`（在跑；忙碌／閒置要 driver）。第一個終端多了 `g9-1: added (claude, …/fake-claude, …/workspace/g9-1)`、`g9-1: start --session-id <同一個 session>`、`g9-1: holder pid=<H> started`。

   - [x] 通過

5. 故意弄壞：身分用錯。

   **這步在驗什麼**：agent 跑不了操作者命令、人跑不了 agent 命令，拒絕時都說該怎麼做；`--json` 的錯誤也是一個 JSON（P1、P4）。錯了的話 agent 能加減 instance、重啟 daemon。

   ```bash
   AGEND_INSTANCE=g9-1 agend status
   AGEND_INSTANCE=g9-1 agend instance add x claude; echo "exit=$?"
   agend done t-1/work/1; echo "exit=$?"
   agend done t-1/work/1 --json; echo "exit=$?"
   ```

   應該看到：

   ```text
   g9-1 (claude): no task
   next: agend inbox | agend send <name> "<message>"
   agend: forbidden: only the operator can add instances; ask the operator
   exit=1
   agend: forbidden: agend done is an agent command; it runs inside an agent, where AGEND_INSTANCE is set
   exit=1
   {"error":{"code":"forbidden","message":"agend done is an agent command; it runs inside an agent, where AGEND_INSTANCE is set"}}
   exit=1
   ```

   - [x] 通過

6. 重啟 daemon 到同一個 binary。

   **這步在驗什麼**：D2：預檢通過才切換，切換後 daemon 同一個 pid、holder 沒換、agent 沒斷（P7）。錯了的話每次升級都要停掉所有 agent，或預檢形同虛設。

   ```bash
   pgrep -fl "agend holder g9-"
   agend daemon restart
   pgrep -fl "agend holder g9-"
   agend status
   ls -d /tmp/agend-pf-* 2>/dev/null | wc -l
   ```

   應該看到（兩次 `pgrep` 同一個 pid；`restarting` 與 `is back` 是同一個 daemon pid）：

   ```text
   24299 /Users/you/AgEnD-v2/target/debug/agend holder g9-1
   preflight agend 0.0.0 (/Users/you/AgEnD-v2/target/debug/agend):
     db copy: migrations 4 -> 4, quick_check ok, instances 1
     holder: hello ok, spawn ok, shutdown ok
   restarting the daemon (pid 24280) ...
   the daemon is back: pid 24280, agend 0.0.0, client protocol 1.2, instances=1 (0.1 s)
   24299 /Users/you/AgEnD-v2/target/debug/agend holder g9-1
   daemon: pid 24280, agend 0.0.0, client protocol 1.2
   instances: 1 (g9-1 unknown)
   tasks: 0
   needs you: 0
          0
   ```

   第一個終端：`preflight passed; restarting with …`、`agend daemon stopping (restart with …); holders keep running`、`exec … daemon`、新的 `agend daemon starting: pid=24280 …`（同一個 pid）、`inherited holder children (exec restart): 24299`、`g9-1: reconnected to holder pid=24299 …`、`agend daemon ready: instances=1 recovered=1 started=0 orphans=0`。

   - [x] 通過

7. 故意弄壞：重啟到一個壞掉的 binary。

   **這步在驗什麼**：預檢失敗時什麼都不切換，舊 daemon 照跑、DB 沒被動過（P7）。錯了的話壞掉的新版會把 daemon 換掉，agent 全部失聯。

   ```bash
   agend daemon restart --binary /usr/bin/false; echo "exit=$?"
   agend status
   ```

   應該看到：

   ```text
   agend: preflight_failed: /usr/bin/false daemon preflight exited with status 1; the daemon keeps running agend 0.0.0
   exit=1
   daemon: pid 24280, agend 0.0.0, client protocol 1.2
   instances: 1 (g9-1 unknown)
   tasks: 0
   needs you: 0
   ```

   pid 跟步驟 6 一樣；第一個終端只多了 `restart requested: preflight of /usr/bin/false`、`preflight failed: … not restarting`，沒有新的開機紀錄。（「DB 一個 byte 都沒動」由自動測試比對：`== preflight` 的 `agend.db byte-identical`；daemon 跑著時會寫 WAL，這裡手動比不準。）

   - [x] 通過

8. 里程碑：兩個 codex agent 互傳訊息，中途重啟 daemon。

   **這步在驗什麼**：ROADMAP 里程碑「第 1–9 施工關：兩個 codex agent 互傳訊息；重啟 daemon 時不中斷、不遺失、不重複」：`send` 經第 7 施工關的送達、斷線時用同一個 `message_id` 重送、daemon 靠 id 去重（P1、P5、P7）。錯了的話重啟 daemon 會掉訊息或重複送。

   第二個終端（codex 是測試用的 `fake_codex`，不是真的 codex）：

   ```bash
   ~/.cargo/bin/cargo build -q -p agend --example fake_codex
   agend instance add g9-a codex --program "$PWD/target/debug/examples/fake_codex" -- --turn-ms 300
   agend instance add g9-b codex --program "$PWD/target/debug/examples/fake_codex" -- --turn-ms 300
   sleep 3
   (for i in $(seq 1 10); do AGEND_INSTANCE=g9-a agend send g9-b "a$i"; AGEND_INSTANCE=g9-b agend send g9-a "b$i"; sleep 0.5; done) &
   sleep 2; agend daemon restart; wait
   sleep 5
   AGEND_INSTANCE=g9-b agend inbox
   AGEND_INSTANCE=g9-a agend inbox
   grep -c ' confirmed (turn ' "$AGEND_HOME"/logs/daemon-*.log
   ```

   應該看到：`added g9-a (codex, /tmp/g9.Xy34/workspace/g9-a); starting`、`added g9-b (…)`；20 行 `accepted: message <id> to g9-b (queue)`／`… to g9-a (queue)`，中間夾著步驟 6 那樣的重啟四行；剛好碰上重啟的那一次多一段 `(retried 0.1 s)`（實跑時是 `a5`）；然後

   ```text
   56cc75fa-0562-4b4d-9088-aa3b169462ac from g9-a: a1
   …（a2 到 a9 各一行）
   547c0209-a7a7-4325-9fa0-98acf3b0859c from g9-a: a10
   b815e90b-1c5d-4dae-b3d5-d6d167bf8e8d from g9-b: b1
   …（b2 到 b9 各一行）
   5e7bb0b9-b44d-4698-b1d8-e597ae4b034a from g9-b: b10
   20
   ```

   g9-b 剛好 10 行 `from g9-a`（`a1`…`a10` 各一次），g9-a 剛好 10 行 `from g9-b`，沒有重複；最後的 `20`＝daemon log 裡 20 則都 `confirmed`。第一個終端每則有一行 `<收件者>: <id> (queue) → turn/start → sent (turn …)` 與 `<收件者>: <id> confirmed (turn …)`；重送的那一則在新 daemon 上是 `<id> already confirmed; not sent again`。手動不一定剛好碰到「請求送出、回應之前」的那一瞬間；那個情況由自動測試負責（`== milestone` 用代理把 `a5` 的回應丟掉、再重啟）。

   - [x] 通過

9. 移除 instance、收尾。

   **這步在驗什麼**：`instance remove` 停掉 agent、刪掉那一列，但留下 workspace；非 TTY 沒有 `--yes` 不動手（P6）。錯了的話誤刪 agent，或把工作目錄一起刪了。

   ```bash
   agend instance remove g9-1 < /dev/null; echo "exit=$?"
   agend instance remove g9-1 --yes
   pgrep -fl "agend holder g9-1"
   ls "$AGEND_HOME/workspace/"
   agend instance remove g9-a --yes
   agend instance remove g9-b --yes
   pgrep -fl "agend holder g9-"; echo "pgrep exit=$?"
   ```

   應該看到：

   ```text
   agend: agend instance remove needs --yes when not on a terminal
   example: agend instance remove g9-1 --yes
   exit=2
   removed g9-1; workspace kept at /tmp/g9.Xy34/workspace/g9-1
   g9-1
   g9-a
   g9-b
   removed g9-a; workspace kept at /tmp/g9.Xy34/workspace/g9-a
   removed g9-b; workspace kept at /tmp/g9.Xy34/workspace/g9-b
   pgrep exit=1
   ```

   第一個 `pgrep` 什麼都不印；`ls` 還有 `g9-1`。第一個終端：`g9-1: removed (workspace kept at …)`、`g9-a: sweep of agent group … (removed): already gone`、`g9-a: removed …`。最後第一個終端 Ctrl-C 停 daemon（`agend daemon stopping (SIGINT); holders keep running`），再收掉暫存檔：

   ```bash
   rm -rf "$AGEND_HOME"
   rm -f "${TMPDIR:-/tmp}"/fake-codex-*.sock   # fake_codex 被 SIGHUP 停掉時留下的 socket
   ```

   - [x] 通過

## 待你追認

實作時做了、提案沒寫到或與提案字面不同的選擇。每項：決定 · 理由 · 反悔的成本。

**L1–L21 使用者已全部追認（2026-09-29）**，含與提案字面不同的 L18（P6）、L19（P4）。

| # | 決定 | 理由 | 反悔成本 |
|---|---|---|---|
| L1 | `home::resolve` 的錯誤在 `agend daemon` 與 `agend debug` 也一樣：`agend: AGEND_HOME is not set; …`、**exit 2**（第 6 施工關原本 daemon 是 `AGEND_HOME is not set`、exit 1；第 8 施工關的 debug 也是 exit 1）；相對路徑也是 exit 2 | P3 說 CLI、daemon、doctor、init 用同一個函式、同一句話；一個 exit code 規則（P4：`AGEND_HOME` 不對是用法錯誤） | daemon／debug 改回 exit 1：`main.rs`、`debug.rs` 各一行＋2 個測試 |
| L2 | client 要 **1.2**（`version::NEEDED`），比 1.2 舊的 daemon 立刻失敗。訊息依 daemon 能不能自己重啟分兩種：1.2 以上 `— run: agend daemon restart`（P4 的寫法），更舊的 `— stop the daemon (Ctrl-C) and start this binary: agend daemon`（1.1 的 daemon 聽不懂 `daemon_restart`，照 P4 的字面會叫人跑一個一定失敗的命令）。`agend daemon restart` 自己只要 1.2（`Client::connect_needing`），以後的 CLI 仍能叫 1.2 的 daemon 重啟 | CLI 用了 1.2 的欄位與請求；P6「`daemon_restart` 的形狀之後不再改」要靠這個下限才有意義 | 訊息改一句：`version.rs` 一處＋測試；下限拿掉：`connect_needing` 刪掉、restart 改用 `connect` |
| L3 | `block`／`unblock`／`remind`：在 agent 裡先送 `status` 拿 task，**沒有 task 時照樣送、`task_id` 是空字串**（真 daemon 現在回 `not_supported`，第 10 施工關決定空 task 的錯誤與訊息）；操作者跑時不先問 `status`，直接送（daemon 回 `forbidden`，訊息寫的是這個命令，不是 `status`） | 「修正指令由發現錯誤的那一方寫」（P4）：CLI 不自己發明「你沒有 task」的錯誤碼；先問 `status` 會讓操作者看到 `agend status is an agent command` 這種對不上的拒絕 | CLI 自己擋：加一個錯誤碼（例如 `no_task`）＋ agent.rs 約 10 行 |
| L4 | P4 表以外的 `--json` code：`disconnected`（10 秒沒回應、收到不是協定的東西）、`restart_failed`（舊 daemon 30 秒沒關連線、10 秒內沒有新的 `boot_id`）、`v1_home`（exit 1）、`init_failed`（建不了 home）；`AGEND_HOME` 沒設或相對路徑用 `usage`（exit 2） | 這幾種錯誤 P4 表沒列，但 `--json` 一定要有 code；都是 CLI 自己發現的（不是 core 的 `error_code`，不進協定） | 改名：`cli.rs`／`operator.rs`／`home.rs` 各一處 |
| L5 | 預檢「通過」除了 exit 0，還要新 binary 印出自己的步驟（`agend …`、`db copy: …`、`holder: hello ok, spawn ok, shutdown ok`）；否則 `preflight_failed: … exited 0 without reporting its steps (is it agend?)`。失敗訊息附預檢 stderr 的最後一行（`exited with status 3: db copy: broken on purpose`） | 不檢查的話 `--binary /usr/bin/true` 會「通過」，daemon 接著 `exec` 一個不是 agend 的程式、整個消失——正是 D2 要擋的事 | 只看 exit code：`handlers/operator.rs` 刪 6 行（`/usr/bin/true` 那個測試要拿掉） |
| L6 | 收屍（reaper）對開機時**每個鎖檔裡的 pid** 做第一次 `waitpid(pid, WNOHANG)`，不只「接回」的 holder：`exec` 到新 daemon 開機之間死掉的 holder（鎖已放掉，不會被接回）也收得到。第一次查在起任何 holder 之前；之後只查還活著的自己的子程序，收到或 `ECHILD` 就不再查；仍然不用 `waitpid(-1)` | P7 的規則只涵蓋接回的 holder；那個空窗死掉的會一直是殭屍到 daemon 結束。擴大範圍不會誤收：開機時還沒有別的子程序 | 只查接回的：`reaper::inherited` 改用 `running_holders`，一行 |
| L7 | `instance remove` 一個 `failed` 的 instance 時，它的「需要你」項目以 `attention_resolved`（`action: unknown`）離開；instance 以 `instance_changed`（不帶 `instance`、summary `removed`）離開全貌 | 第 8 施工關的事件裡沒有「項目被刪除」；不發事件的話 TUI 會一直顯示一個已刪掉的 instance 的項目；`unknown` 表示沒有人對它做了什麼 | 加一個新事件（例如 `attention_withdrawn`）：core 一個 variant＋fleet 一處＋協定相容測試 |
| L8 | `status`、`send` 的呼叫者必須是 daemon 的 instance（否則 `unknown_instance: no instance x: AGEND_INSTANCE names no instance of this daemon …`）；`inbox` 對不存在的呼叫者回空清單 | 第 8 施工關 P2「DB 沒有的 instance 當 agent」只說它沒有操作者權限；寄件者不是 instance 的訊息，收件者無從回覆 | `send` 也放行：`handlers/agent.rs` 刪 3 行 |
| L9 | CLI 等回應的時間：一般 10 秒（第 8 施工關）；`send` 70 秒（daemon 在訊息寫進 DB、而且 codex 的長連線送出之後才回，第 7 施工關那段最多 60 秒）；`daemon restart` 等 `restarting` 70 秒（預檢最多 60 秒＋複製 DB） | 10 秒後放棄的 `send` 其實可能已經送出，agent 若重下一次就會用新的 id、變成重複；等得夠久才能用同一個 id 的重送涵蓋 | 改數字：`agent.rs`、`operator.rs` 各一個常數 |
| L10 | `instance add --dir` 給的目錄必須已存在（daemon 只建預設的 `$AGEND_HOME/workspace/<name>`，0700）；`--dir` 與路徑形式的 `--program` 由 CLI 轉成絕對路徑（daemon 的 cwd 跟 CLI 不同）；`--program` 只給名字時從 agent 的 `PATH` 找 | 自己建使用者指定的目錄容易建錯位置；相對路徑到 daemon 那邊意思就變了 | daemon 也建 `--dir`：supervisor 一行 |
| L11 | 假 daemon 的 `daemon_restart` 不跑真的預檢：跑 `<binary> --version`，要印 `agend …` 才過（失敗訊息格式同真的），過了就回 `restarting`、關掉所有連線、換新的 `boot_id`（狀態保留）；CLP-15 只釘失敗的形狀，成功的重啟由 CLI-31 對假、真各跑一次 | 假 daemon 沒有 DB、holder；讓它真的跑 `agend daemon preflight` 要在暫存目錄起 holder，測試變慢又多一處要清 | 假 daemon 改跑真預檢：約 20 行＋每個重啟測試多一個 holder |
| L12 | 人看的輸出格式（P4 只定了錯誤）：`send` → `accepted: message <id> to <name> (<level>)[ (retried 1.4 s)]`；`inbox` → 每則一行 `<id> from <name>: <第一行>`（多行的 body 其餘行縮兩格），沒有 → `no messages`；agent 的 `status` 印 daemon 給的 summary（`g9-1 (claude): no task` ＋ `next: …`），有 identity 時多一行 ticket；操作者的 `status` 四行（`daemon: pid …`、`instances: …`、`tasks: …`、`needs you: …`＋每個項目一行）；`daemon restart` 的進度行先印在 stdout 再等 | 一行一則讓 `grep -c "from g9-a"` 就能數（步驟 8）；進度先印，等 30 秒時人知道在等什麼 | 改格式：`cli/agent.rs`、`cli/operator.rs` 各幾行（CLI-n 表跟著改） |
| L13 | `operator` 的 `task_cancel` 在協定上只有 `{ task_id }`（第 10 施工關頁寫 `task_cancel { task_id, reason }`）；操作者形式的 `task create`（帶 `--team`）不送任何東西，CLI 直接回 `not_supported: the operator's agend task create arrives in gate 10; nothing was sent`（協定沒有 `task_create` 變體，第 10 施工關連同 handler 一起加）；沒帶 `--team` 時 CLI 就擋下：`needs --team <team>`、exit 2（CLI-26），不連 daemon | 本關的 CLI 沒有收 reason 的參數；選填欄位之後加是 additive（D26），現在加一個沒人填的欄位違反骨架規則 | 現在加 `reason`：core 一個選填欄位＋CLI 一個旗標 |
| L14 | `agend` 不帶參數照舊印說明、exit 0；未知命令改成 clap 的 `unrecognized subcommand 'x'` ＋ `For more information, try '--help'.`、exit 2（原本是 `unknown command 'x'` 加一行 `Run agend --help for usage.`）；`agend daemon <別的參數>` 走 clap（exit 2） | P4 決定用 clap；argv0 的測試跟著改 | 自己攔未知命令：`cli.rs` 約 5 行 |
| L15 | doctor 細節：daemon 跑著時孤兒的 `fix` 是 `agend daemon restart   (its boot sweep stops orphans)`（P8 寫 `agend daemon`，但 daemon 跑著時再跑一個會因為 DB 被占用而失敗）；daemon 沒跑、有 holder 在跑 → `N running; orphans unknown (the daemon is not running)`、fix `agend daemon`；git 的 `fix` 依作業系統（macOS `brew install git`，其他 `sudo apt-get install git …`）；三個 backend 的安裝指令都用 npm（opencode 是 `npm install -g opencode-ai`，頁上的例子寫 brew）；找 backend 時跳過 `$AGEND_HOME/bin`（shim）；每個 `--version` 最多等 5 秒 | 修正指令要照做就會好 | 改字：`doctor.rs`、`agend_core::setup` 各一處 |
| L16 | `SUPPORTED_VERSIONS` 改成 `[1.2]`：daemon 與所有 client 共用，所以第 11 施工關的 TUI（還有自己的連線程式）現在也宣告 1.2；TUI 的程式沒有改。第 8 施工關的測試跟著調整：終端那段裡操作者送 `command status` 現在是 `forbidden`（原本 `not_supported`）、版本訊息、`client_demo` 的 CLP 從 12 條變 17 條；CLP-8 的行程內 server 多開一個暫存的 store 與 codex driver（`Context` 需要） | 協定 minor 只加欄位、1.1 的 peer 照樣解得開（`protocol_compat.rs` 的凍結 1.1 型別） | 無（改回去 1.1 的 client 就用不了本關的請求） |
| L17 | 訊息大小上限（verifier F3）：`send` 的 body 最多 **1 MiB**（`MAX_MESSAGE_BYTES` = 1,048,576 bytes），超過 → `invalid_request: the message is N bytes; a message is limited to 1048576 bytes (1 MiB)`，什麼都不存；協定的一行最多 **8 MiB**（`MAX_LINE_BYTES`），超過 → `invalid_request: a protocol line is limited to 8388608 bytes …`、關連線，daemon 只讀到上限就停、不把整行讀進記憶體；CLI 在連 daemon 前檢查 body，超過 → 同一句、exit 2；假 daemon 同規則（CLP-17 多一項，mutant `AcceptsHugeBody`） | 上限擋的是「單一訊息把 daemon 與送達路徑拖垮」，理由是這幾條（第 2 輪 verifier 之後改寫）：codex 的 JSON-RPC 走 WebSocket，一個 frame 最多 16 MiB，整個 body 在一個 `turn/start`／`thread/queue/add` 裡，超過就送不出去（第 1 輪 20 MB 的 body 只換來一再重連）；沒有上限時一次 40 MB 的 send 讓 daemon 的 RSS 到約 600 MB；claude、opencode 的送達路徑在第 12 施工關，限制還沒量，1 MiB 對兩者都保守；agent 之間的訊息是指示與摘要，長內容應該放檔案或 PR。**原本寫的「20 MB 讓送達永遠卡住」其實是另一個問題**：link 與假 app-server 都只用一個 thread、寫的時候不讀，連續幾則大於 socket 緩衝區的訊息就互相卡住（8 KiB 就會），跟 1 MiB 上限無關；那個問題在 link 修掉了（寫入逾時、關閉不無限等，見「已知風險」與進度紀錄）。8 MiB 是 1 MiB 的 body 每個 byte 都被 JSON 跳脫成 `\u00XX` 時的上限；Linux 單一命令列參數本來就只有 128 KiB。**接受的缺口**：`inbox` 的回應沒有上限（20 則 1 MiB 的訊息是一個約 20 MB 的回應、`--json` 更大），讀的一方是 agent 自己要的 | 改數字：core 兩個常數（CLI-n 與 CLP-17 的測試跟著改）；第 12 施工關量到更小的限制時往下調 |
| L18 | **與 P6 不同，請明確決定**：`agend instance remove` 帶 `--json` 時一律要 `--yes`，即使在 TTY 上也不問（沒有 `--yes` → exit 2）；在 TTY 上回答不是 yes → `agend: g9-1 was not removed`、code `declined`、**exit 1**（什麼都沒做＝失敗；不是用法錯誤，所以不是 2） | `--json` 是給程式用的：stdout 只能有一個 JSON 值，問題會混進去；exit code 照 P4：2 只給用法錯誤 | 改成 exit 2：`cli/operator.rs` 一行 |
| L19 | **與 P4 不同，請明確決定**：`agend --json --help`（或任何命令的 `--help`）印的是一般文字說明，不是 JSON | 說明是給人看的；clap 的說明沒有 JSON 形式，另寫一份要跟 clap 同步 | 包成 `{"help":"…"}`：`cli.rs` 約 5 行 |
| L20 | `instance remove` 不刪 `messages` 表裡寄給它或它寄出的訊息；之後用同一個名字再 `instance add`，`agend inbox` 會看到舊的訊息（30 天保留期限內） | 訊息是送達紀錄（第 7 施工關 P5：唯一的冪等層），刪掉就可能讓舊 id 被重用；名字重用是操作者的選擇 | 移除時一起刪訊息：`store` 一個 `DELETE`＋supervisor 一行（要先決定 id 能不能重用） |
| L21 | 預檢逾時（60 秒）回的 code 是 `preflight_failed`（`… daemon preflight did not finish within 60 s`），不是另一個 code；逾時的子程序被 kill 並收屍、暫存 home 照樣刪、之後可以再 restart | P7 步驟 4：預檢任何一步失敗都回 `preflight_failed`；CLI 的 `restart_failed` 是 CLI 自己發現的（舊 daemon 沒關連線、沒回來），不是 daemon 的錯誤碼 | 改 code：core 加一個錯誤碼＋`handlers/operator.rs` 一處 |

另記（事實）：
- 啟動時間：加 `clap` 後 `agend --version` 50 次的中位數 1.9 ms（`--version` 在 clap 之前就回）。
- `exec` 與 fd：實測 `exec` 後新 daemon 1 ms 就拿到 `agend.db`（舊的已在 `exec` 前關掉；SQLite 用 `O_CLOEXEC` 開檔），client 連線在 `exec` 前由 `Server::stop` 關掉，CLI 讀到 EOF。
- 里程碑的自動測試用代理把 `a5` 的回應丟掉、再重啟 daemon：CLI 重連後用同一個 `message_id` 重送，新 daemon 回 `accepted`、log `already confirmed; not sent again`；codex thread 裡每則剛好一個 user message。
- 手動跑步驟 8 時 `fake_codex` 的 app-server 在 `instance remove`（holder 的 SIGHUP）時沒有刪掉自己在 `$TMPDIR` 的短 socket（`fake-codex-<hash>.sock`）；自動測試用 `BoundSocket` 刪，步驟 9 最後手動刪。

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
| 2026-09-29 | 通過 | 使用者在 `feat/gate-09-cli`（55a6285）照抄步驟 1–9 全部通過，里程碑步驟 8：a1–a10、b1–b10 各一次、20 則 confirmed，`b5` 碰上重啟 `(retried 0.1 s)`。已知問題（不擋本關）：① macOS 預設 `ulimit -n 256` 下步驟 1 的 `cargo xtask accept cli` 在 testkit `contract_teeth` 報 `Too many open files`，`ulimit -n 4096` 後通過——待修：xtask 自動調高 soft limit，並查是否有測試漏關 fd；② 步驟 6 的 `ls -d /tmp/agend-pf-*` 在 zsh 沒有檔案時印 `no matches found`（結果仍是 0），步驟 9 的清理 glob 同樣問題——待修文件寫法。 |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-29 使用者親自驗收步驟 1–9 通過（里程碑：兩個假 codex 互傳 20 則、中途重啟，各一次、全部 confirmed）；記下兩個不擋本關的已知問題（macOS `ulimit -n 256`、zsh glob 寫法）。狀態改成完成。
- 2026-09-29 fresh-context verifier 第 3 輪 CONFIRMED（add6d10；第 1、2 輪 REFUTED 的項目全修：preflight 清理、60 秒期限、大小上限、binary 換檔檢查含 ctime、codex link 死鎖與寫入逾時、負載下的計時測試）；使用者追認 L1–L21。
- 2026-09-28 第 2 輪 verifier REFUTED 後修正（每項先寫重現測試、確認在舊程式失敗）：#1 連續送 3 則大訊息給 codex agent 時 daemon 死鎖——根本原因兩邊都有：假 app-server 寫的時候不讀（不忠實；已改成讀寫交錯），daemon 的 **第 7 施工關 codex link**（在本 PR 修）寫入沒有逾時、`Link::close` 無限期 join → 寫入 10 秒逾時當斷線、關閉時 shutdown socket、最多等 5 秒（`large_messages_to_codex_are_delivered`、`a_stuck_codex_app_server_never_wedges_the_daemon`，後者用 `fake_codex --disable duplex-io` 做出不讀的 peer）；L17 的理由改寫；#2 兩個會被機器負載拖垮的時間上限改成量產品本身（hello 次數、訊息裡的 10 秒）；#3 binary 檢查加 change time（`a_binary_swapped_during_its_preflight_is_refused`）；#4 L18、L19 標「與 P6／P4 不同，請明確決定」。重跑 r2c（預檢中錯開時間送 SIGINT）時另外發現：預檢通過後、daemon 收尾準備 `exec` 的那一刻收到的 Ctrl-C 會被舊的 image 吃掉、新的照樣起來 → 收到停止訊號就不 `exec`、直接結束（`ctrl_c_during_a_restart_stops_the_daemon`，修之前 3／3 失敗）。
- 2026-09-28 fresh-context verifier REFUTED（1 MEDIUM、2 LOW-MEDIUM、2 LOW）後修正，每項先寫重現測試、確認在舊程式失敗再修：F1 daemon 在預檢中停止會留下 `/tmp/agend-pf-*`（含 DB 複本）與孤兒子程序 → Drop guard（`ctrl_c_during_a_preflight_leaves_nothing`）；F2 子程序留下的背景程序開著 stdout 時 60 秒逾時失效 → 逾時不等 pipe（`the_preflight_deadline_holds`）；F3 訊息沒有大小上限 → body 1 MiB、一行 8 MiB（L17，`oversized_messages_and_lines_are_refused`、CLP-17＋mutant `AcceptsHugeBody`）；F4 `--binary` 的信任範圍寫進已知風險，預檢後確認 binary 仍是同一個檔案；F5 文字：`resolve_attention` 的拒絕不再叫 agent 用 `agend ask`、`answer_ask` 寫第 10 施工關、拒絕移除的 code 改 `declined`（exit 1），補 L18–L21。
- 2026-09-28 實作（draft PR，`feat/gate-09-cli`）：client protocol 1.2（`operator` 請求、`send` 的 `level`／`message_id`、`hello` 的 `daemon_version`／`daemon_pid`／`boot_id`、`working_directory`、`identity`）；`agend` 的 clap CLI（11 個 agent 命令、`instance add|remove|list`、`daemon restart`、`task cancel`、`status`、`doctor`、`init`、`--json`、exit 0／1／2、`home::resolve`）；daemon 端權限、`send`→`deliver`、`inbox --after`、instance 增刪、D2 預檢（`VACUUM INTO` 複本＋暫存 home 的 holder）與原地 `exec`、繼承 holder 的收屍；假 daemon 補齊、CLP-13..17 各有 mutant；CLI-n 表 46 列（假／真）；里程碑自動測試（兩個假 codex、中途重啟、代理丟掉一個回應）通過；「你親自驗收」照實跑細化；待你追認 L1–L16。fresh-context verifier 還沒跑（實作者是 leaf agent，不能自己派）。
- 2026-09-26 加操作者命令 `agend task cancel <task>`（使用者從第 10 施工關決定；handler 在第 10 施工關，之前 `not_supported`，不重送）；記下 doctor 之後要查沙箱工具。
- 2026-09-26 使用者逐題確認 P1–P10：P1 含 `ask` 移第 10 施工關、操作者也能 `task create`（要 `--team`）、KISS 選項不套用；P3 改成 `AGEND_HOME` 一律必須設（預設位置與 `config.toml` 列不列 home 延到第 13 施工關）；P6 對使用者用 `name`、內部仍是 `instance_id`；步驟 2、3 改用 `AGEND_HOME`。
- 2026-09-26 第 3 輪 review REFUTED（2 MEDIUM、3 LOW）後修正：`--after` 依第 7 施工關 `messages.seq`、未知／過期回 `unknown_message`；`send` 一律回 `accepted`；restart 等 EOF 最多 30 秒；同 id 檢查與寫入在同一個 DB closure。
- 2026-09-26 第 2 輪 review REFUTED（3 MEDIUM、數個 LOW）後修正：restart 先等舊連線 EOF、以 `hello` 的 `boot_id` 確認換了；client `message_id` 只收 UUID v4、同 id 內容不同回 `invalid_request`；步驟 9 的 `pgrep` 只查 g9-1；送給 claude／opencode 停在 `queued`；`--after` 依寫入順序；`ECHILD` 後不再查；步驟 8 雙向；KISS 兩項列為待你決定。
- 2026-09-26 補第 9、10 施工關分工（第 10 施工關負責 `ask` 與 task 類 handler、`workflow`／`team`；之前回 `not_supported`）；ticket 由第 10 施工關印在派工訊息；待你決定：操作者能不能 `task create`。
- 2026-09-26 fresh review REFUTED（5 MEDIUM、6 LOW）後修正：收屍只對接回的 holder pid `waitpid(pid, WNOHANG)`；`hello` 加 `daemon_pid`、全貌 instance 加 `working_directory`；步驟 3 改在步驟 2 的暫存 HOME 跑；本關接 `send` → `deliver` 與 `level`，第 7 施工關 merge 為硬前提、加步驟 8（兩個 agent 互傳、中途重啟）；TL;DR 與 P1、P3、P8 標出與已定文件不同之處；`daemon_probe` 定位、重加仍在跑的 id、restart 一次一個＋`mkdtemp`、`--binary` 在 D6 範圍內、協定版本寫成「實作時的下一個 minor」。
- 2026-09-26 開工前提案 P1–P10 寫定（draft PR），待使用者確認；`ask` 建議移到第 10 施工關（P1）、結果類命令帶 ticket（P2）、預設 home `~/.agend-v2`（P3）、restart 用原地 `exec`（P7）、`init` 不建 `config.toml` 與 agent（P9）；「你親自驗收」改成 9 步；狀態改為提案中。

## 下一步

```bash
~/.cargo/bin/cargo xtask accept cli    # 你親自驗收步驟 1
```

然後照「你親自驗收」步驟 2–9 跑、填「驗收紀錄」（「待你追認」L1–L21 已於 2026-09-29 追認）。
