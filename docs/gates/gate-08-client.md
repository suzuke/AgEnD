# 第 8 施工關：agend-client + protocol server（整合施工關）（`client`）

> **TL;DR**
> - 真 daemon 開 `run/daemon.sock` 講 client protocol；`agend-client` 同步連線、daemon 重啟時重試 10 秒、版本不合立刻說清楚；順便補上第 11 施工關的協定缺口 G1–G3（G4 移到第 12 施工關）。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：先看「待你追認」C1–C14，再照「你親自驗收」7 步走（agent 帶著做）。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**已驗收，等 merge**（2026-09-26）：draft PR #131（branch `feat/gate-08-client`）；自動驗收全部通過；C1–C14 使用者已追認；使用者親自驗收 7 步通過。P1–P10 使用者已確認（含改 T6、G4 移第 12 施工關）。

## 範圍

- daemon 的 protocol server：`$AGEND_HOME/run/daemon.sock`，就緒訊號改成「socket 連得上」（第 6 施工關 P1、P5 已預告；做法見 P1）
- 呼叫者身分：`hello` 帶選填的 `caller`，不做 cookie（P2）
- client protocol 升到 1.1（只加欄位與新訊息）；錯誤碼搬進 core（P3）
- 協定缺口（從[第 11 施工關 G1–G4](gate-11-tui.md#待你追認) 移來）：
  - G1：一次拿到全貌的 `get_fleet` 請求，加事件游標規則（P4）
  - G2：`attention_required` 加 `attention_id`、放行數、等待起始時間、「不處理的話」（P5）
  - G3：非請示的「需要你」項目加操作與「已處理」事件；本關第一個真來源是 `failed` 的 instance（P5）
  - G4：已讀狀態**移到第 12 施工關**（P6）
- 真 daemon 本關做哪些請求、哪些之後才做（P6）
- agend-client：連線、重試、版本檢查、給第 9 施工關 CLI 用的 API；`agend debug ping`、`agend debug watch`（P7）
- server 的 thread 模型、多個 client、慢 client（P8）
- client 協定契約：假 daemon 與真 daemon 跑同一套規則（P9）
- `check-deps` 規則（P10）

## 開工前提案

每項：問題 · 建議 · 理由 · 替代方案 · 例子。文件已定、這裡不重問的：只有一套公開、有版本的協定（D1）；JSON Lines over unix socket、`hello` 協商版本、同 major 只加欄位（D26）；protocol 型別在 core、client 同步 I/O（D11）；CLI 在 daemon 重啟中重試最多 10 秒（規劃 §4.7，`RESTART_RETRY_WINDOW` 已釘）；不做 `api.cookie`、`api.port`、`.ready`（第 6 施工關 P1）；就緒訊號在本關改成 socket 連得上（第 6 施工關 P5）；同 uid 的防護只是安全帶（D6）；權限依呼叫者身分（D17）；「需要你」排序是 core 的 `policy::attention::order`（D36）；請示是對話（D35）；第 11 施工關 T1–T18 與 G1–G4 是否為缺口已追認（這裡只決定怎麼補）。

### P1：socket 放哪、權限、何時出現

- 問題：socket 放哪？誰能連？daemon 當掉留下的舊 socket 檔怎麼辦？什麼時候算「daemon 好了」？
- 建議：
  - 路徑固定 `$AGEND_HOME/run/daemon.sock`。daemon 開機時確保 `run/` 是 0700，bind 後把 socket 設成 0600。
  - 路徑超過 100 bytes（macOS 上限 104）就拒絕啟動，印 `socket path too long: … (max 100 bytes); use a shorter AGEND_HOME`，比照 holder（第 4 施工關 P3）。
  - 拿到 DB 的 EXCLUSIVE 鎖之後（已證明沒有別的 daemon），先刪掉舊的 `daemon.sock` 再 bind。不另做鎖檔。
  - **開機計畫做完才 bind**，接著印原本那行 `agend daemon ready: …`。所以「socket 連得上」＝ daemon 好了，連上的 client 一定看到完整的 instance 清單。
  - Ctrl-C／SIGTERM：先關 listener、刪掉 socket 檔、關所有 client 連線，再照第 6 施工關 P1 結束。
- 理由：DB 鎖已經保證只有一個 daemon，刪舊 socket 是安全的；bind 在開機計畫之後，就沒有 v1 #922 那種「port 檔出現了、清單還不完整」的時間差，也不需要 `.ready`。停止時刪檔，client 看到的是「沒有 socket」，比「連線被拒」好解釋。
- 替代方案：一拿到 DB 就 bind（client 會看到開機到一半的清單，要另外標「開機中」）；socket 放 `$XDG_RUNTIME_DIR`（兩個平台不同，而且跟 home 分開）；另做 `daemon.lock`（DB 鎖已經夠）。
- 例子：`ls -l "$AGEND_HOME/run/daemon.sock"` → `srw-------`；daemon 被 `kill -9` 後舊檔還在，client 連線被拒、照 P7 重試；新 daemon 起來後刪掉舊檔、重新 bind。
- [x] 使用者確認（2026-09-26）

### P2：怎麼知道是誰在連（身分、要不要 cookie）

- 問題：D17 說 daemon 依呼叫者身分限制權限。unix socket 上要不要 cookie？身分從哪來？
- 建議：
  - **不做 cookie、不做 token**。能連上 socket 的只有同一個使用者（P1 的 0700／0600）。
  - client 的 `hello` 改用自己的資料型別 `ClientHello { supported, caller }`（`caller` 選填）；core 的 `Hello` 是 holder 協定也在用的（`protocol/holder.rs`），不動它，`caller` 不會漏進 holder 協定。wire 上 `data` 只多一個選填欄位，1.0 的 peer 照樣解得開。
  - agent 裡的 CLI 填 `AGEND_INSTANCE`（daemon 起 agent 時設的，第 6 施工關 H3）；沒填就是操作者。填了但 DB 沒有這個 instance：**當成 agent**（權限只少不多）。
  - 這是跟 binding 快照同級的安全帶（D6）：同 uid 的 agent 可以不填或亂填，擋的是「agent 照說明跑錯命令」，不是惡意 agent。
  - 本關只有一個地方用到身分：`resolve_attention`（P5）只接受操作者；agent 送來回 `forbidden`，訊息附正確做法。agent 命令的權限在第 9 施工關做。
- 理由：v1 為了 TCP loopback 做 `api.cookie`，後來又加 `api.operator` 第二把，註解自己承認同 uid 的 agent 讀得到檔案、隔離「沒有解決」（v1 `src/auth_cookie.rs`）。unix socket 靠檔案權限就擋掉其他使用者，cookie 在同 uid 下多擋不了什麼。
- 替代方案：peer credential（`SO_PEERCRED`／`LOCAL_PEERPID` 拿 pid，再往上找屬於哪個 holder）：比較難假冒，但兩個平台寫法不同，agent 用 double fork 脫離 holder 也能繞過；cookie（v1 的路，同 uid 一樣繞得過）。
- 例子：agent `g8-1` 裡 `agend debug ping` 送 `{"type":"hello","data":{"supported":[…],"caller":"g8-1"}}`；它若送 `resolve_attention` 會拿到 `forbidden: only the operator can resolve needs-you items; ask the operator with agend ask`。
- [x] 使用者確認（2026-09-26）

### P3：協定版本與錯誤碼

- 問題：本關要在協定加東西（P4、P5）。版本號怎麼動？舊的 1.0 peer 怎麼辦？假 daemon 自己發明的錯誤碼要不要固定？
- 建議：
  - 本關所有新增算一次 minor：client protocol **1.1**，`SUPPORTED_VERSIONS = [1.1]`。只加選填欄位與新的請求／事件，1.0 的 peer 解得開（新的變成 `unknown`），D26 不變。
  - client 需要 1.1 的功能（`get_fleet`）卻協商到 1.0：立刻失敗，訊息 `the daemon speaks client protocol 1.0; this agend needs 1.1 — restart the daemon with this binary`，不重試。
  - 錯誤碼變成 core 的常數（`protocol::client::error_code`）：`hello_required`、`version_mismatch`、`invalid_request`、`unknown_request`、`unknown_ask`、`stale_result`（已有）、新增 `event_gap`、`not_supported`、`forbidden`、`no_terminal`、`unknown_attention`（沒有這個 `attention_id`，或這個操作不在它的 `actions` 裡）。`resolve_attention` 先檢查身分（`forbidden`）再找 id，所以 agent 不能拿它探測哪些項目存在。假 daemon 改用 core 的常數。
  - 唯一的 wire 格式：真 server、假 daemon、`agend-client`、testkit 的 `ProbeClient` 都用 `serde_json` 編 core 型別，沒有第二份手寫格式（#1493）。
- 理由：一次 minor 讓「協商到舊版要說清楚」這條路本關就被測到；錯誤碼只有一份，CLI（第 9 施工關）才能依錯誤碼決定訊息與 exit code。
- 替代方案：不升版本（v2 還沒發布，可以直接改）——但 D26 的相容規則就一直沒被真正用過；每加一項升一次 minor（版本號變多、沒有好處）。
- 例子：假 daemon `set_supported_versions(&[1.0])` → `agend debug ping` 印上面那行錯誤、exit 1，而且立刻結束（不等 10 秒）。
- [x] 使用者確認（2026-09-26）

### P4：G1 全貌與事件游標

- 問題：TUI 需要 team、task、agent 的清單與結構化狀態（G1）。要做很多個 list 請求，還是一個快照？重連時怎麼保證不漏、不重複事件？
- 建議：
  - **一個請求** `get_fleet { request_id }`，回新的 `ClientResponse::Fleet { request_id, fleet }`（現有的 `command_result` 是給 agent 命令的，全貌另開一種回應）；`fleet` 有：`as_of_event_id`、`teams`、`tasks`（含關卡清單與目前關卡）、`instances`（含 backend 與 `state`）、`attention`（目前的「需要你」清單）。名稱用「全貌」（fleet view），避開名詞表裡已經很多的「快照」。
  - 接著 `subscribe_events { after_event_id: as_of_event_id }`，之後的變化靠事件；`instance_changed`、`task_changed` 各加一個選填欄位帶新的結構化內容（1.0 的文字 `summary` 保留）。
  - agent 的 `state`：`starting`／`working`／`idle`／`stuck`／`failed`／`unknown`。本關真 daemon 只給得出 `starting`、`failed`、`unknown`（忙碌／閒置要 driver，第 7、12 施工關）。「需要你」不放進 `state`：由 client 從 `attention` 算（第 11 施工關 T18 已經這樣做），只有一個真相。
  - 事件 id 以「開機時間（unix ms）× 1000」為起點，**第一個事件是起點 + 1**（起點本身不發，比照假 daemon 的編號），事件只放記憶體（最近 1024 筆）。`after_event_id` 小於「留著的最舊一筆 − 1」（接不上：中間的事件已經丟了，包括上一次開機的 id），**或大於目前最新的 id**（例如時鐘往回調後，舊游標反而比新的大）→ `event_gap`，client 重拿全貌。等於「最舊一筆 − 1」算接得上，所以 `get_fleet` 與訂閱之間剛好有事件進來也沒問題。這兩條擋住大部分的跳號；時鐘往回調的極端情況仍可能漏（見已知風險），所以 1.1 的 client **重連後絕不沿用舊游標**，一律重拿全貌。還沒有任何事件時，「最新的 id」就是起點本身：`after_event_id` 等於起點算接得上，其他都是 `event_gap`；不帶 `after_event_id` **維持 1.0 的意思：重播留著的全部事件（最近 1024 筆）再接即時事件**（假 daemon 現在就是這樣，第 11 施工關的 `daemon_source.rs` 靠它），1.0 的 client 拿不到全貌也不會漏掉連線前的項目，D26 不破例。1.1 的 client 一律帶 `as_of_event_id`。所以 1.1 的規則只有一條：**重連一律重拿全貌**。
  - **與已追認的第 11 施工關 T6 不同，請明確決定**：T6 寫「重連後重播事件」（假 daemon 從頭重播 backlog）；這裡改成「重連一律重拿全貌、只接之後的事件」。TUI 回到原本畫面與選取的行為不變。
  - 真 daemon 本關能填的：`teams` 固定一個 `general`（D12；team 表由之後的施工關加）；`tasks` 照 DB 現有的列，關卡清單第 10 施工關補；`instances` 來自 DB 與 supervisor。
- 理由：一個請求、一個 `as_of`，全貌和事件之間沒有縫；很多個 list 請求各自有時間差。事件不存 DB：全貌本來就能從 DB 重建，重啟後重拿比記錄一份跨重啟的事件日誌簡單；id 以開機時間為底，舊 id 幾乎一定比新的小（時鐘往回調除外，見已知風險），不用另外的欄位。
- 替代方案：每種東西一個 list 請求（G1 原本的寫法，要自己處理 list 與訂閱之間的事件）；事件存 DB 讓 client 跨重啟接續（多一張表、多一套保留規則）；每次開機 id 從 1 開始（舊 client 的游標會「剛好在範圍內」而悄悄漏事件）。
- 例子：TUI 連上 → `get_fleet` 回 `as_of_event_id=1790000000000000`、1 個 team、2 個 instance → 訂閱；daemon 重啟 → 連線斷 → 重連、重拿全貌（新的 `as_of` 比較大）；如果拿舊游標訂閱，或游標比 daemon 最新的 id 還大，都回 `event_gap`。
- [x] 使用者確認（2026-09-26）

### P5：G2、G3「需要你」的欄位與操作

- 問題：`attention_required` 沒有 id、放行數、等待時間、「不處理的話」（G2、T16）；非請示的項目沒有操作，也不會消失（G3）。怎麼補？本關有真的來源嗎？
- 建議：
  - `attention_required` 加選填欄位：`attention_id`（請示用 ask id；其他是固定的字串，例如 `instance-failed:g8-2`）、`unblocks`、`waiting_since_unix_ms`、`if_ignored`、`actions`、`instance_id`。排序照 D36 用 core 的 `policy::attention::AttentionItem`：wire 上叫 `attention_id`（同一則訊息裡還有 `task_id`、ask 的 id，單叫 `id` 分不清），轉成 `AttentionItem` 時放進它的 `id`，其他兩個欄位同名。
  - 新請求 `resolve_attention { request_id, attention_id, action }`（只收操作者，P2），成功回現有的 `command_result { request_id, result: accepted }`（跟 `answer_ask` 一樣，不另加回應型別）；新事件 `attention_resolved { attention_id, action }`。請示照舊用 `answer_ask` 與 `ask_updated`。
  - 本關的真來源：supervisor 放棄的 instance（第 6 施工關 P6「交給人」）。`actions` 只有 `retry`：holder 還在（第 6 施工關 H8 留著它給人看最後的畫面）就先送 `Shutdown` 等它結束（H9：一個 holder 一生只跑一個 agent），再清掉 `failed`、重算重起次數，照 session 有沒有建立過決定怎麼起。
  - **session 有沒有建立過要存下來**：`failed` 之後 DB 只剩 `failed`，分不出它死之前是 `new` 還是 `running`（第 6 施工關 H1、H11：第一次 `Spawn` 前就失敗的 claude 也會變 `failed`）。所以 **migration `0003`** 在 `instances` 加一欄 `session_started INTEGER NOT NULL DEFAULT 0`：第一次 `Spawn` 被確認、寫 `running` 的同一個交易裡設成 1；migration 把 `status='running' OR (status='failed' AND backend<>'claude')` 的列設成 1，欄位加 `CHECK (session_started IN (0, 1))`。現有 `failed` 的 codex／opencode 幾乎一定跑起來過（第 6 施工關 H2：第一次 `Spawn` 確認就寫 `running`，下一次死就 `failed`），設 1 才不會被當成「沒跑過」而全新啟動（違反 P6）；現有 `failed` 的 claude 留 0 是安全的（見下表）。`retry` 時：

    | backend | `session_started` | `retry` |
    |---|---|---|
    | claude | 1 | 狀態回 `running`，帶 `--resume <id>` |
    | claude | 0 | 狀態回 `new`，帶 `--session-id <id>`。若 session 其實已存在：`Spawn` 一被確認就記 `running`，claude 拒絕後的下一次重起走 `--resume`，接回原對話 |
    | codex、opencode | 0 | 給 `retry`：從沒跑起來過，全新啟動不丟任何東西（第 6 施工關 H2） |
    | codex、opencode | 1 | 不給 `retry`（`actions: []`）：沒有 session id 可接，P6 不全新啟動（第 7、12 施工關有 session id 後再開）。TUI 顯示「沒有可用的操作」（第 11 施工關 G3 現在的字是「client protocol v1 還沒有處理這一項的操作」，B 段接上後改成這句），`if_ignored` 寫 `delete and re-add the instance (gate 9)` |

  - 真 daemon 送 `attention_required` 時也帶選填的 `instance_id`，TUI（第 11 施工關 T18）不用拆 `attention_id` 字串就知道它屬於哪個 agent。
  - 「需要你」清單不另存表：每次從 DB 的 `failed` instance 算。`waiting_since` 用「這個 daemon 第一次看到它 `failed` 的時間」：本次開機才放棄的，是放棄那一刻；開機時就已經是 `failed` 的，是開機時間。**`waiting_since` 不另加欄位**。代價：daemon 重啟後，舊的 `failed` 項目的等待時間從重啟那刻重算（排序變成同一批、再以 id 定序）。`unblocks` 是它手上的 task 數（第 10 施工關前一律 0）。
  - 其他操作（暫停、改派…）等有來源的施工關再加：`actions` 的 enum 有 `unknown`，舊 client 看到新操作不會壞。
- 理由：G3 有真來源才驗得到「操作 → daemon 決定 → 消失」這條路，不用發明假的項目；`failed` 本來就要人處理，而且目前**沒有任何方法**讓它再試：第 6 施工關的 `plan_boot` 開機時完全不碰 `failed` 的 instance（`boot.rs`：「a human decides」），重啟 daemon 也不會，只能刪掉重加。
- 替代方案：`session_started` 改成存「失敗前的狀態」（`failed_from`：`new`／`running`），資訊一樣、欄位語意比較繞；`failed_at_unix_ms` 也放進 `0003`，跨重啟保留真正的等待時間（目前只影響多個 `failed` 項目之間的先後，先不加）；G3 整個移到第 10 施工關（本關就沒有任何非請示項目可驗，第 11 施工關 B 段會缺一步）；`dismiss`（只從清單拿掉、instance 還是 `failed`，容易忘記）；另開 `attention` 表（本關只有一種來源，從 instance 算就夠）。
- 例子：`g8-2` 一起來就死 → 3 次後 `failed` → 事件 `attention_required {attention_id:"instance-failed:g8-2", unblocks:0, waiting_since_unix_ms:…, if_ignored:"g8-2 stays stopped", actions:["retry"]}` → 操作者送 `retry` → `attention_resolved`，daemon log `g8-2: start --resume …`（`retry` 重算次數，走 `start`、不是 `restart n/3`；確切字樣開工時細化）。
- [x] 使用者確認（2026-09-26）

### P6：真 daemon 本關做哪些請求（G4 移走）

- 問題：client protocol 有一堆請求，但真 daemon 還沒有 task、請示、訊息（第 9、10 施工關）。本關做到哪？G4 已讀狀態做不做？
- 建議：

  | 請求 | 真 daemon 本關 |
  |---|---|
  | `hello`、`get_fleet`、`subscribe_events`、`resolve_attention` | 做 |
  | `subscribe_terminal` | 做：先回 holder 當下的畫面，再轉送之後的 `terminal_bytes`（daemon 的 holder 長連線轉出來，client 絕不直接連 holder）。`failed` 的 instance：daemon 已關掉對它的長連線（第 6 施工關 H8），holder 還在就短暫連一次取 `Snapshot`、只回這張最後的畫面、不串流（這次連線會讓 holder 的 24 小時計時從斷線時重算，第 4 施工關 G5）；holder 已經不在回 `no_terminal` |
  | `terminal_input` | 回 `not_supported`；第 11 施工關 B 段做（要先確定只有操作者能打字） |
  | `answer_ask` | 沒有請示，回 `unknown_ask`；請示在第 9、10 施工關 |
  | `command`（agent 命令） | 回 `not_supported`，訊息寫在哪個施工關做；第 9、10 施工關 |

  - **G4 已讀狀態移到第 12 施工關**（**與已追認的第 11 施工關 G4「第 8 施工關補」不同，請明確決定**）：要 daemon 記已讀，唯一的理由是跟 Telegram 共用，而 Telegram 在第 12 施工關。在那之前 TUI 用本機已讀（第 11 施工關 T4、T17）。
  - 第 11 施工關 B 段步驟 2（「處理一項請示」）：本關之後真 daemon 還沒有請示（`answer_ask` 回 `unknown_ask`）。建議 B 段步驟 2 在第 8 施工關之後改用 `failed` instance 的「需要你」項目按 `retry`（驗的是同一件事：由 daemon 決定解決、只看過不會消失）；「回答請示」那半等第 9、10 施工關有請示後再驗。改第 11 施工關頁不在這個 PR。
- 理由：只做有真資料的請求；其餘回明確錯誤而不是假資料。已讀要存 DB（新表、保留規則），沒有第二個讀者時做了驗不到「共用」。
- 替代方案：本關把 G4 也做完（多一張表，第 12 施工關前沒有第二個讀者）；`terminal_input` 本關一起做（要先定操作者限制，會把第 9 施工關的權限提前）。
- 例子：`command {status}` → `error not_supported: agent commands arrive in gate 9 (agend status)`，連線不斷。
- [x] 使用者確認（2026-09-26）

### P7：agend-client 的 API、重試、錯誤訊息

- 問題：client 怎麼重試才能讓「命令執行中重啟 daemon」最後仍成功？什麼情況不重試？第 9 施工關 CLI 需要什麼？TUI 的重連跟它是什麼關係？
- 建議：
  - API（同步、不建 runtime）：`Client::connect(socket, caller)`：連線＋`hello`，連不上每 100 ms 重試，最多 10 秒（`RESTART_RETRY_WINDOW`）；`Client::connect_once`：只試一次，給有自己重連畫面的 TUI（第 11 施工關 T6 每 500 ms）；`request(…)`：送請求、依 `request_id` 等回應，最多等 10 秒；`events()`：阻塞讀下一個事件。
  - 什麼會重試：socket 不存在、連線被拒、請求送出前連線就斷。**請求送出後**斷線，只有呼叫端標明「可重做」的請求才重送（本關只有讀取類的 `get_fleet`；agent 命令哪些可重做，包括 `status`、`inbox`，由第 9 施工關逐一決定）；其他回 `daemon restarted during the request; check with agend status`。
  - 什麼不重試：版本不合（P3）、`forbidden`、其他 daemon 回的錯誤。
  - 10 秒到了的訊息：`cannot reach the AgEnD daemon at <path> after 10 s (<原因>). Is it running? Start it with: agend daemon`，exit 1。
  - socket 路徑由呼叫端給（`agend` 從 `AGEND_HOME` 算）；client 不讀環境變數與設定檔。
  - 本關的命令（`agend debug …`，唯讀）：`agend debug ping [--count N --interval MS]` 印協定版本與 instance 數；`agend debug watch` 印全貌摘要與之後的事件；連不上或斷線時跟 TUI 一樣用 `connect_once` 每 500 ms 重試、不放棄（印 `reconnecting…`），連上就重拿全貌。
  - 給第 9 施工關：錯誤型別分「連不上／版本不合／daemon 回錯誤（含錯誤碼）」三種，CLI 依此選訊息與 exit code；`caller` 由 `agend` 從 `AGEND_INSTANCE` 填。
- 理由：10 秒是規劃定的；只重送讀取類，避免 daemon 重啟時同一個命令被做兩次（v1 的訊息重複類問題，V1-LESSONS #1）。TUI 本來就有自己的斷線畫面與重連，不必吃 10 秒的阻塞。
- 替代方案：所有請求都重送（非冪等的命令會重複）；靠 `request_id` 讓 daemon 去重（要跨重啟記住，等於多一張表）；client 自己讀 `AGEND_HOME`（client 就依賴環境，測試難隔離）。
- 例子：daemon 沒在跑：`time agend debug ping` → 約 10 秒後印上面那段錯誤、exit 1。`agend debug ping --count 20 --interval 500` 跑到一半重啟 daemon：20 行都成功，中間一兩行是 `ok (retried 1.4 s)`。
- [x] 使用者確認（2026-09-26）

### P8：server 的 thread 模型、多個 client、慢的 client

- 問題：server 跑在 tokio 還是自己開 thread？同時很多 client 怎麼辦？有個 client 不讀（例如 TUI 卡在很慢的 ssh），會不會拖住 daemon？
- 建議：
  - 跟第 6 施工關一致：用 daemon 已有的 tokio multi-thread runtime，加 `net` feature；每個連線一個 task。命令處理在 `handlers`，`server` 只管 socket 與 JSON Lines（第 6 施工關 `server.rs` 的 `Must NOT` 已寫）。
  - 事件用一個 `tokio::sync::broadcast`（1024 筆）送給每個訂閱者。某個 client 落後到被覆蓋（`Lagged`）→ 送 `error event_gap`、關掉**它**的連線；它重連、重拿全貌（跟 daemon 重啟同一條路）。
  - 寫入加 5 秒逾時：完全不讀的 client 會把 socket buffer 塞滿（macOS 約 8 KB、Linux 約 200 KB），之後 server 的寫入卡住，5 秒後直接關掉它的連線。這種 client 收不到 `event_gap`（寫不進去），只會看到連線被關；兩種都走同一條「重連、重拿全貌」。
  - 終端串流：holder 長連線把 `PtyBytes` 轉進每個 instance 一個 broadcast（256 塊）；落後也是關連線，重連時先拿到當下畫面。
  - 不限制連線數；CLI 的連線是一次性的。
- 理由：一條「落後就斷、重連重拿」的規則，慢 client 不會拖住 daemon 或其他 client，也不用替每個 client 排無限長的隊（v1 曾因為死掉的 TUI 訂閱者沒被清掉而修過，#3682）。重連本來就要做，不多一條路。
- 替代方案：每個連線一條 std thread（像假 daemon；但 handler 要呼叫 async 的 store）；每個 client 無上限的佇列（慢 client 讓 daemon 記憶體一直長）；落後時跳過事件繼續送（client 畫面悄悄變錯）。
- 例子：三個 client 訂閱，daemon 連續發 2000 個事件：正常的全部收到、順序正確；**讀得很慢**的（每 10 ms 讀一筆）落後超過 1024 筆，收到 `event_gap` 後被關；**完全不讀**的在 5 秒寫入逾時後被關（收不到 `event_gap`）。
- [x] 使用者確認（2026-09-26）

### P9：client 協定契約：假 daemon 與真 daemon 跑同一套

- 問題：TUI、CLI 的測試都對 testkit 假 daemon 跑。怎麼保證假 daemon 跟真 daemon 行為一樣？
- 建議：
  - 在 [CONTRACTS.md](../../crates/agend-testkit/CONTRACTS.md) 加一張表 `CLP`（client protocol），每條一個編號；suite 放 testkit，對**兩個** server 跑：`FakeDaemon` 與真的 `agend daemon`（暫存 home、真 binary，放 `crates/agend/tests/`，比照第 6 施工關 H12）。
  - 規則草稿（開工時定稿）：`hello` 必須第一個；major 不合回 `version_mismatch` 並關閉；`get_fleet` 的 `as_of` 之後訂閱，事件不漏不重；舊游標回 `event_gap`；未知請求回錯誤、連線不斷；兩個 client 收到同樣的事件、同樣的順序；落後的 client 被關、其他不受影響（P8）；server 重啟後重連、重拿全貌；`not_supported` 的請求不改任何狀態。
  - 每條至少一個故意弄壞的 mutant（比照第 2 施工關的規則表）。
  - `agend-client` 的測試對假 daemon 跑；契約的驅動端用 testkit 的 `ProbeClient`，不用 `agend-client`，這樣 client 的 bug 不會被同一份程式碼蓋掉。
  - 假 daemon 補上本關新增的請求與規則（`get_fleet`、`resolve_attention`、事件 id 起點、游標規則、落後就斷；不帶游標照舊重播 backlog），讓第 9、11 施工關的測試對得上真 daemon。
- 理由：v1 #1483 的假綠就是測試餵了 production 從不送的格式；同一套規則跑兩邊，假 daemon 一偏離就失敗（#1493）。
- 替代方案：只對真 daemon 測 client（慢，而且 TUI／CLI 的測試仍然對假 daemon）；各寫各的測試（兩邊會慢慢漂移）。
- 例子：假 daemon 的事件 id 從 1 開始（沒照 P4）→ CLP 的「舊游標回 `event_gap`」對假 daemon 失敗、對真 daemon 通過，一眼看出是假的偏了。
- [x] 使用者確認（2026-09-26）

### P10：依賴規則

- 問題：`agend-client` 要保持輕，`check-deps` 要加什麼？
- 建議：
  - `agend-client` 保持現有禁止清單（async runtime、SQLite、`agend-daemon`）；一般依賴只有 `agend-core` 與 `serde_json`。
  - 新規則：`agend-daemon` 不能依賴 `agend-client`（server 與 client 各自編碼，契約才驗得到兩邊一致）。
  - `agend-tui` 的 lib 改走 `agend-client` 是第 11 施工關 B 段的事，本關不動 TUI。
- 理由：CLI 啟動要輕（D11 實測 p50 4.1 ms）；daemon 若借用 client 的程式碼，兩邊的 bug 會一起出現、互相蓋掉。
- 替代方案：不加 daemon 規則（靠 code review）。
- 例子：有人在 `agend-daemon` 的 `Cargo.toml` 加 `agend-client` → `cargo xtask check-deps` 失敗，訊息附 `cargo tree … -i agend-client`。
- [x] 使用者確認（2026-09-26）

### 本關不做（明確列出）

- cookie、token、peer credential（P2）；跨使用者存取。
- WebSocket、給外部 GUI 的 JSON schema 產生器（D1、D11 說「需要時」）。
- 已讀狀態（G4 → 第 12 施工關，P6）；`terminal_input`（第 11 施工關 B 段）；agent 命令與請示（第 9、10 施工關）。
- `dismiss`、暫停等 `retry` 以外的操作（P5）。
- 事件存 DB、跨 daemon 重啟接續事件（P4）。
- 心跳：daemon 卡住但沒死時，TUI 不會發現（daemon 死掉時 unix socket 會立刻 EOF）；CLI 有 10 秒回應逾時。
- 連線數上限；Windows。
- TUI 改接 `agend-client` 與 `agend app`（第 11 施工關 B 段）。

### 已知風險（開工時處理）

- 第 6 施工關已 merge（#125）：本關實作從最新的 `v2` 開 branch；本頁引用的 `boot.rs`、`link.rs` 行為以 merge 後的程式為準。
- bind 在開機計畫之後（P1）：instance 很多、每個 holder 起不來要等 5 秒時，開機可能超過 10 秒，CLI 會先放棄。本關要量開機時間；超過就改成先 bind、全貌標「開機中」。
- 事件 id 用開機時間當起點（P4）：系統時鐘往回調時，新的 id 可能比舊的小。實際影響是 client 多重拿一次全貌或少拿幾個事件；開工時測「時鐘往回」會怎樣，必要時改用 DB 裡遞增的開機序號。
- 終端串流要在 holder 長連線上同時處理 `PtyBytes` 與 `Snapshot` 回應（第 6 施工關的 link 目前丟掉 `PtyBytes`）：要確認「畫面＋之後的位元組」接得起來，不重不漏。
- 落後就斷（P8）：很慢的網路上 TUI 可能一直重連。先記 log（哪個 client、落後多少），實際遇到再調 buffer。
- 身分可以假冒（P2）：同 uid 的 agent 能假裝是操作者。跟 D6 同級，已知且接受；要更強只能換 peer credential。
- socket 路徑長度：macOS 的 `$TMPDIR` 很長，測試的 home 照第 6 施工關 H13 放 `/tmp/g8-<pid>-<n>`。

## 自動驗收（完成定義）

- [x] `~/.cargo/bin/cargo test -p agend-client`、`-p agend-daemon`、`-p agend-testkit`、`-p agend-core`、`-p agend` 單獨通過（`cargo xtask accept client` 逐一跑）。對應的測試：重試 10 秒後的訊息、版本不合立刻失敗、送出後斷線只重送可重做的請求（`agend-client/tests/client.rs`）；1.0 解得開 1.1、1.1 解得開 1.0（`xtask/tests/protocol_compat.rs`，凍結的 1.0 型別）；`failed` → `attention_required` → `retry` → `attention_resolved`，`retry` 依 `session_started` 帶 `--resume`／`--session-id`／不帶，codex 跑過的沒有 `retry`（`crates/agend/tests/client_protocol.rs::retry_resumes_or_starts_by_session_started`、`supervisor` 單元測試）；`0003` 的回填、`CHECK`、`schema-v3.sql`、golden（`agend-daemon/tests/store.rs`）；socket 0600、101 bytes 拒絕、停止時刪檔（`the_socket_is_private_replaced_after_a_crash_and_removed_on_stop`）；agent 送 `resolve_attention` 回 `forbidden`（CLP-11、`agend-client` 測試）
- [x] `CLP` 對假 daemon 12/12、對真 `agend daemon` 11/11 ＋ CLP-8 對 daemon 的 server 程式（見 C1）；每條有 mutant（14 個，`contract_teeth`）；反向檢查：事件 id 改回從 1 開始時 CLP-4 失敗（`client_protocol_fake_with_ids_from_one_fails_clp_4`）
- [x] 慢 client（P8）：CLP-8 — 正常的 2000 個全部收到且順序正確；每 10 ms 讀一行的收到 `event_gap` 後被關；完全不讀的在 5 秒寫入逾時後被關、沒有 `event_gap`
- [x] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [x] `~/.cargo/bin/cargo xtask check-deps` 最後一行 `check-deps: ok (6 rules, 8 crates checked for agend-testkit, agend-core metadata ok, no-std build ok)`；新規則 `agend-daemon` ↛ `agend-client`，故意加依賴時失敗：`check-deps: agend-daemon depends on agend-client (…); inspect with cargo tree -e normal,build -p agend-daemon -i agend-client`
- [x] `~/.cargo/bin/cargo xtask accept client` 通過，並印出「你親自驗收」步驟 1 的 demo
- [x] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新；想改的共用文件列在 PR 裡
- [x] 測試不留殘留：跑完 `pgrep -fl "agend (holder|daemon)"` 沒有輸出、`/tmp/g8-*` 沒有留下；只對自己起的 daemon 子程序送 SIGINT／`Child::kill`，holder 用 `Shutdown`
- [x] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」（r1 CONFIRMED `7147763`、4 LOW 已修；修正 delta 再驗 CONFIRMED `881c120`）

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。下面的「應該看到」是實作者 2026-09-26 在 macOS 照抄指令實跑的輸出（時間戳、pid、session id、`as_of` 每次不同）。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二、第三個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
unset AGEND_BIN               # 前幾關步驟留下的 export 可能指到已刪除的 worktree
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.0.0`。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

步驟 2 起用同一個暫存 home。第二、第三個終端也要貼上步驟 2 印出的那行 `export AGEND_HOME=…`。

1. 跑 demo。

   **這步在驗什麼**：同一套 client 協定規則對假 daemon 和真 daemon 都通過，版本不合、慢 client、socket、`retry`、終端、重啟這幾段也跑完（P3、P5、P8、P9）。錯了代表 TUI 與 CLI 的測試對的是一個跟真 daemon 不一樣的假東西。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo xtask accept client
   ```

   要跑幾分鐘（前面是各 crate 的 fmt／clippy／測試，最後約 1 分鐘是 demo）。應該看到：

   | 段落 | 要找的字 |
   |---|---|
   | `== contract` | `CLP-1 fake ok`、`CLP-1 real ok` … 到 `CLP-12`，每條兩行；`CLP-8 real ok (the daemon's server code in this process: 2000 events)`（見 C1）；`negative check (fake event ids from 1 again): CLP-4 FAIL: …` |
   | `== version` | `agend debug ping → exit 1 in 0.02 s: the daemon speaks client protocol 1.0; this agend needs 1.1 — restart the daemon with this binary` |
   | `== slow-client` | `normal reader: all 2000 events, in order`、`slow reader: event_gap after … events, then closed`、`no reader: closed after 5 s write timeout (… no event_gap)` |
   | `== socket` | `run/ is 700, run/daemon.sock is 600`、`Ctrl-C: run/daemon.sock removed`、`101-byte socket path: exit 1: agend daemon: socket path too long: …` |
   | `== retry` | `retry → …cr: start --resume <S1>`、`retry → …cn: start --session-id <S2>`、`retry → …xn: start`、`…xs (codex, ran before): retry → unknown_attention` |
   | `== terminal`、`== restart` | `terminal_snapshot …, then terminal_bytes`；`12/12 ok; ok 5/12 (retried 1.4 s)` 之類；watch 有兩行 `fleet:` |
   | 最後 | `client demo: all sections passed`，然後 `gate 8 (client): checks passed` |

   - [x] 通過

2. 故意弄壞：daemon 沒在跑時連線。

   **這步在驗什麼**：client 會等 daemon（重啟時要用），但 10 秒後一定放棄並說清楚怎麼辦（P7）。錯了的話 CLI 不是永遠卡住，就是一句看不懂的 `Connection refused`。

   ```bash
   export AGEND_HOME="$(mktemp -d /tmp/g8.XXXX)" && echo "export AGEND_HOME=$AGEND_HOME"
   time agend debug ping; echo "exit=$?"
   ```

   應該看到（約 10 秒後）：

   ```text
   cannot reach the AgEnD daemon at /tmp/g8.fCmu/run/daemon.sock after 10 s (No such file or directory (os error 2)). Is it running? Start it with: agend daemon
   agend debug ping  0.00s user 0.01s system 0% cpu 10.333 total
   exit=1
   ```

   - [x] 通過

3. 啟動 daemon，再連一次。

   **這步在驗什麼**：socket 在 daemon 好了之後才出現、只有你能連，連上馬上回協定版本（P1、P3）。錯了的話別的使用者能連，或 client 看到開機到一半的資料。

   第一個終端：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- add g8-1
   agend daemon
   ```

   應該看到（daemon 留在前景）：`added g8-1: claude session …`，然後 `g8-1: start --session-id …`、`listening on /tmp/g8.…/run/daemon.sock`、`agend daemon ready: instances=1 recovered=0 started=1 orphans=0`（`listening` 在 `ready` 前面）。

   第二個終端（先跑開頭那段、貼上 `export AGEND_HOME=…`）：

   ```bash
   agend debug ping
   ls -l "$AGEND_HOME/run/daemon.sock"
   ```

   應該看到：

   ```text
   agend daemon: client protocol 1.1, instances=1
   srw-------@ 1 suzuke  wheel  0 Sep 26 18:45 /tmp/g8.fCmu/run/daemon.sock
   ```

   關鍵：`client protocol 1.1, instances=1`，`ls` 那行開頭是 `srw-------`（結尾的 `@` 是 macOS 的延伸屬性，不影響）。

   - [x] 通過

4. 命令執行中重啟 daemon。

   **這步在驗什麼**：這關的主要驗收：daemon 重啟時命令會等它回來，最後照樣成功（P7）。錯了的話每次重啟 daemon，正在跑的 agent 命令都會失敗。

   第二個終端：

   ```bash
   agend debug ping --count 20 --interval 500
   ```

   它跑的 10 秒內，到第一個終端按 Ctrl-C，再馬上跑 `agend daemon`。

   應該看到：20 行都以 `ok` 開頭，其中一兩行帶 `(retried …)`，最後回到提示符號（exit 0）：

   ```text
   ok 7/20: client protocol 1.1, instances=1
   ok 8/20 (retried 1.6 s): client protocol 1.1, instances=1
   ok 9/20: client protocol 1.1, instances=1
   ```

   第一個終端的新 daemon 印 `g8-1: reconnected to holder pid=…; screen: counter=…`（同一個 holder，計數器沒歸零）。

   - [x] 通過

5. 兩個 client 看同一件事：agent 一直死，進「需要你」，再按重試。

   **這步在驗什麼**：事件同時送到每個 client；`failed` 的 instance 變成「需要你」，帶 id、等待時間、「不處理的話」與 `retry`；只有操作者按得了，按了才消失（P2、P5、P8）。錯了的話 TUI 看不到 agent 死掉，或看到了卻沒辦法處理。

   第一個終端 Ctrl-C 停 daemon，再加一個一起來就死的 instance（先不要啟動 daemon）：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- add g8-2 --dies
   ```

   **先開兩個 watch，再啟動 daemon**，才不會錯過一開始的事件：第二個終端跑 `agend debug watch`；再開第三個終端，先跑開頭那段設定（`cargo build` + `export PATH`）、貼上 `export AGEND_HOME=…`，也跑 `agend debug watch`。兩個 watch 這時每 0.5 秒印一行 `reconnecting… (cannot reach the AgEnD daemon at … (No such file or directory (os error 2)))`（daemon 還沒起來），這是正常的。然後在第一個終端：

   ```bash
   agend daemon
   ```

   應該看到：兩個 watch 都印一行 `fleet: …`，之後的事件兩邊相同、順序相同（第一次死掉可能落在 `fleet:` 全貌裡，所以只比 `fleet:` 之後的）。約 15 秒後：

   ```text
   fleet: instances=2 (g8-1 unknown, g8-2 starting) tasks=0 teams=1 attention=0 as_of=1790419525018005
   instance_changed g8-2 unknown: running (holder pid=93813)
   instance_changed g8-2 starting: died; restart 2/3 in 5 s
   instance_changed g8-2 unknown: running (holder pid=93841)
   instance_changed g8-2 starting: died; restart 3/3 in 5 s
   instance_changed g8-2 unknown: running (holder pid=93885)
   instance_changed g8-2 failed: restarted 3 times in 10m and it still died; not restarting
   attention_required instance-failed:g8-2 (unblocks 0, waiting since 1790419540451; if ignored: g8-2 stays stopped) actions: retry
   ```

   操作：第三個終端按 Ctrl-C 停掉它的 watch，然後：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-client --example client_probe -- resolve instance-failed:g8-2 retry
   AGEND_INSTANCE=g8-1 ~/.cargo/bin/cargo run -q -p agend-client --example client_probe -- resolve instance-failed:g8-2 retry; echo "exit=$?"
   ```

   應該看到：

   | 在哪 | 要找的字 |
   |---|---|
   | 第三個終端第一行（操作者） | `resolved` |
   | 第二個終端的 watch | `attention_resolved instance-failed:g8-2 retry`，接著 `instance_changed g8-2 starting: start --resume …` |
   | 第一個終端的 daemon | `g8-2: retry requested by the operator`、`g8-2: start --resume …`（先停掉留著的 holder：`holder g8-2 (pid …) exited`） |
   | 第三個終端第二行（假裝是 agent） | `forbidden: only the operator can resolve needs-you items; ask the operator with agend ask`、`exit=1` |

   `--dies` 的 agent 還是會死：約 15 秒後又回到「需要你」，這是正常的。

   - [x] 通過

6. 故意弄壞：watch 開著時重啟 daemon。

   **這步在驗什麼**：長時間連著的 client 在 daemon 重啟後自己重連、重拿全貌，不會用舊的事件游標漏事件（P4）。錯了的話 TUI 在 daemon 重啟後顯示過期的資料。

   操作：第二個終端的 watch 繼續開著；第一個終端 Ctrl-C，再跑 `agend daemon`。

   應該看到（第二個終端）：

   ```text
   disconnected: the daemon closed the connection
   reconnecting… (cannot reach the AgEnD daemon at /tmp/g8.fCmu/run/daemon.sock (No such file or directory (os error 2)))
   fleet: instances=2 (g8-1 unknown, g8-2 starting) tasks=0 teams=1 attention=0 as_of=1790419548311005
   ```

   關鍵：`disconnected: …`、幾行 `reconnecting…`、新的 `fleet:`，而且新的 `as_of` 比重啟前的大；之後的事件照常出現。

   - [x] 通過

7. 收尾。

   **這步在驗什麼**：daemon 停止時刪掉 socket；本關的 instance 在下次開機被收掉，什麼都不留（P1；第 6 施工關的孤兒巡查）。錯了的話會留下 holder 或舊 socket。

   操作：第二個終端 Ctrl-C 停 watch；第一個終端 Ctrl-C 停 daemon，然後：

   ```bash
   ls "$AGEND_HOME/run/"
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- remove g8-1
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- remove g8-2
   agend daemon
   ```

   應該看到：`ls` 只有 `holders`（沒有 `daemon.sock`）；`removed g8-1`、`removed g8-2`；daemon 印 `orphan g8-1: Shutdown sent`、`orphan g8-2: Shutdown sent`、`agend daemon ready: instances=0 recovered=0 started=0 orphans=2`。

   然後在第二個終端：

   ```bash
   pgrep -fl "agend holder g8-"; echo "pgrep exit=$?"
   ```

   應該看到只有 `pgrep exit=1`（什麼都沒找到）。最後在第一個終端按 Ctrl-C，再 `rm -rf "$AGEND_HOME"`。

   - [x] 通過

## 待你追認

實作時做了、提案沒寫到或與提案字面不同的選擇。確認前照目前的做法運作。每項：決定 · 理由 · 反悔的成本。

**追認結果**：使用者 2026-09-26 全部追認 C1–C14（C1 單獨明確決定：CLP-8 在測試程序裡跑同一份 server 程式碼，不在正式程式加測試開關）。

| # | 決定 | 理由 | 反悔成本 |
|---|---|---|---|
| C1 | **與 P9 字面不同，請明確決定**：CLP-8（2000 個事件的慢 client）的「真」不是真 binary，而是同一份 daemon server 程式碼（`agend_daemon::server` + `fleet`）在測試程序裡跑、直接發事件；其他 11 條都對真 `agend daemon` binary 跑 | 真 binary 裡每個事件都要一次真的 instance 狀態改變（最快的是對 `failed` 的 instance 按 `retry`，每次起一個 holder），2000 個做不到；替 binary 加「測試用發事件」的開關等於在正式程式裡放後門 | 加一個只在測試用的環境變數讓 daemon 發假事件，約 20 行＋一條「只測試用」的規則 |
| C2 | 假 daemon 的 `subscribe_terminal` 對任何 instance id 都回一張畫面；CLP-12 只釘「先回畫面」，`no_terminal`（沒有這個 instance）只在真 daemon 的測試裡驗 | 第 11 施工關 TUI 的測試用 catalog 裡的 agent id 訂閱終端，假 daemon 若只認識自己登記的 instance，TUI 的測試要先改（本關不動 TUI） | 假 daemon 只回登記過的 instance、TUI 測試先 `set_instance`：約 15 行 |
| C3 | mutant 與反向檢查（事件 id 改回從 1 開始）都是「假 daemon 前面加一個改行的 proxy」（`contract::client::proxy`），假 daemon 本身沒有「故意弄壞」的開關 | 開關會留在 production 的假實作裡，被別的測試誤用；proxy 只存在測試路徑 | 在 `FakeDaemon` 加開關，mutant 改用開關：約 40 行 |
| C4 | instance 的 `state`：啟動中與「死了、等 5 秒重起」都是 `starting`；holder 起來、`Spawn` 被確認後是 `unknown`（忙碌／閒置要 driver）；放棄是 `failed` | P4 只說本關給得出這三種；等重起時 agent 正要回來，最接近「啟動中」 | 等重起時改成別的狀態（例如新加 `restarting`）：core 一個 variant + supervisor 一行 |
| C5 | 全貌的 `tasks` 在 daemon 開機時從 DB 讀一次；關卡清單 `stages` 空、`current_stage` 無 | 第 10 施工關前 daemon 不寫 task，daemon 跑著時 `daemon_probe` 也開不了 DB（鎖），所以開機後不會變；每次 `get_fleet` 都讀 DB 要在 handler 裡多一次跨執行緒呼叫 | 改成每次 `get_fleet` 讀 DB：handler 多一個 `store.tasks().await`（store 要能從 handler 拿到） |
| C6 | `resolve_attention` 由 handler 當場檢查並拿掉項目、發 `attention_resolved`、回 `accepted`，**不等 supervisor**（它可能正在停別的 holder，最多 15 秒，會超過 client 的 10 秒）；`retry` 之後才由 supervisor 做。`retry` 開始不了（讀／寫 DB 失敗）就把原本的項目放回清單（再發一次 `attention_required`）；停舊 holder 失敗走 `failed`，會列出新的項目（verifier r1 LOW-3、LOW-4） | CLI 不必卡住；結果看事件（`instance_changed … starting` → `unknown`，或再死一次又回到「需要你」） | 等 `start` 做完才回：handler 改回等 supervisor 的回覆（約 20 行） |
| C7 | 終端串流：holder 那條長連線結束（agent 重起、instance 放棄）時送 `error no_terminal "the terminal of X ended; subscribe again"`，連線**不關**；落後 256 塊才關連線。游標接不上的 `event_gap` 也不關連線（可以在同一條連線重拿全貌）；只有「落後超過 1024 筆」的 `event_gap` 會關 | P8 只規定「落後就斷」；終端結束不是 client 的錯，關掉連線會連事件訂閱一起斷 | 改成一律關連線：server 兩行 |
| C8 | 同一條連線上再訂閱一次事件或終端：新的取代舊的 | 最簡單；TUI 切換 agent 時直接重訂終端 | 改成回 `invalid_request`：server 兩行 |
| C9 | `agend debug ping` 的輸出：單次 `agend daemon: client protocol 1.1, instances=N`；`--count` 時每行 `ok 3/20: client protocol 1.1, instances=N`，重試過的是 `ok 5/20 (retried 1.4 s): …`；中途有一次失敗就印錯誤、exit 1、不再繼續 | 頁面例子只寫 `ok (retried 1.4 s)`；加上第幾次比較好對照 | 改格式：`debug.rs` 一行 |
| C10 | `agend debug watch` 連不上時每 500 ms 印一行 `reconnecting… (<原因>)`（帶原因）；版本不合時印錯誤、exit 1，不一直重試 | 原因能看出是 daemon 沒開還是別的；版本不合重試也不會好 | 去掉原因或版本不合也重試：各一行 |
| C11 | 假 daemon：一個請求造成的事件，在它的回應**之後**才送到同一個 client（跟真 daemon 一樣）；原本的假 daemon 是事件先到，`tests/fake_daemon.rs` 那一條改成回應先到 | 真 daemon 處理請求時不讀事件，回應一定先寫出去；假 daemon 要跟真的一樣 | 無（改回去會讓假 daemon 跟真的不一樣） |
| C12 | `ClientResponse` 加 `#[allow(clippy::large_enum_variant)]`（`attention_required` 多了 6 個欄位後，事件那一格比其他大） | 回應一次只處理一行、不會大量存著；改成 `Box` 要改每個建構的地方 | 改成 `Event { data: Box<EventData> }`：約 20 處 |
| C13 | 為了編譯，`agend-tui` 改了最少的地方：3 個結構加新欄位（`None`／空）、`terminal::describe` 多一個 `attention_resolved` 的分支；行為不變，TUI 改接 `agend-client` 仍在第 11 施工關 B 段 | core 加欄位後，完整寫出欄位的地方不改就編不過 | 無 |
| C14 | 依賴：`agend-client` 一般依賴加 `serde_json`（P10 已寫）、dev 依賴 `agend-testkit`；`agend` 的 dev 依賴加 `tokio`（C1 在測試程序裡跑 server）；`xtask` 的 dev 依賴加 `serde`（`protocol_compat.rs` 凍結的 1.0 型別） | 都不是一般依賴，`check-deps` 規則不變 | 移除時對應的測試要換寫法 |

另記（事實）：
- 開機時間（P1 已知風險）：1–7 個 instance 時，從 `agend.db opened` 到 `listening on …`／`ready` 約 0.1 秒；CLI 的 10 秒不會先放棄。instance 很多且 holder 起不來的情況沒有量。
- 時鐘往回調（P4 已知風險）：`fleet::tests::a_clock_stepped_back_is_a_gap_unless_the_old_cursor_falls_in_range` 釘住現況：舊游標比新 daemon 最新的 id 還大 → `event_gap`；落在新範圍內會被接受（可能漏事件），所以 1.1 client 重連一律重拿全貌。
- 終端「畫面＋之後的位元組」（P1 已知風險）：畫面用 holder 長連線上的 `Snapshot` 請求拿，holder 在同一條連線依序回，所以畫面之後讀到的 `PtyBytes` 一定是畫面之後的（`the_terminal_streams_after_the_screen`）。
- macOS 在對方已關閉的 unix socket 上設讀取逾時會回 `EINVAL`；`ProbeClient::recv_within` 因此忽略設定逾時的錯誤（關閉前送的資料照樣讀得到）。
- 第 4 施工關的 `agend-holder/tests/server.rs::agent_environment_is_only_what_spawn_carries` 在全 workspace 平行跑時失敗過一次（`screen never matched`），單獨重跑 3 次都通過；本關沒有改 holder。

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
| 2026-09-26 | 通過 | 在 `feat/gate-08-client`（merge 前）由 agent 帶著走 7 步。步驟 1 第一次失敗：終端分頁裡殘留第 4 施工關的 `export AGEND_BIN=…/AgEnD-v2-gate04/…`（worktree 已刪），`unset` 後通過；已加 `unset AGEND_BIN` 到開頭設定，demo 對不存在的 `AGEND_BIN` 改成直接說明。步驟 2：10.006 s 放棄、exit=1。步驟 3：`listening` 在 `ready` 前、socket `srw-------`。步驟 4：20/20 ok，第 5 次 `retried 0.9 s`，同一個 holder 17414。步驟 5：兩個 watch 事件相同同序；操作者 `resolved`、agent `forbidden` exit=1；retry 先停舊 holder 再 `start --resume <同一個 session>`。步驟 6：重連重拿全貌，`as_of` 變大、`attention=1` 保留。步驟 7：socket 已刪、`orphans=2`、`pgrep exit=1`。 |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-26 使用者親自驗收 7 步通過（merge 前）；開頭設定加 `unset AGEND_BIN`，`client_demo` 對不存在的 `AGEND_BIN` 直接說明。
- 2026-09-26 使用者追認 C1–C14（C1 單獨明確決定）。
- 2026-09-26 fresh-context verifier r1：CONFIRMED（沒有 HIGH／MEDIUM），4 LOW + 1 INFO 已修：socket 段加「開機計畫卡 5 秒時，socket 出現的第一刻連上就看到開機計畫做完的全貌」（bind 移到開機前的 mutant 會失敗）；`retry` 段加「每個 instance 只起一次 holder、沒有 start failed、沒有 restart」（拿掉先停舊 holder 的 mutant 會失敗）；`resolve_attention` 不等 supervisor；`retry` 開始不了時把項目放回清單；假 daemon 游標 `+1` 改 saturating。另外全量測試抓到一個 client 的真 bug：連上之後、`hello` 回應之前 daemon 就關掉連線時，macOS 在設定逾時／寫入時回 `ENOTCONN`／`EINVAL`，沒被當成「請求送出前就斷」而不重試；現在這段的任何錯誤都重試（P7）（#131）。
- 2026-09-26 實作（draft PR #131，branch `feat/gate-08-client`）：core client protocol 1.1（`ClientHello`、`get_fleet`／全貌、`resolve_attention`、`attention_resolved`、錯誤碼 `client::error_code`、`order_attention`）；migration `0003`（`session_started`，`schema-v3.sql`、golden）；daemon 的 `run/daemon.sock` server、`fleet`、`handlers`、`failed` → 「需要你」→ `retry`、終端串流；`agend-client`；`agend debug ping|watch`、`client_probe`；假 daemon 1.1；CLP-1..12 契約（假 daemon、真 daemon、14 個 mutant、反向檢查）；`check-deps` 新規則；`client_demo` 與 `xtask accept client`；「你親自驗收」7 步改成確切指令與實跑輸出；「待你追認」C1–C14。fresh-context verifier 尚未跑。
- 2026-09-26 第 4 輪 review（1 MEDIUM：第一個事件 id＝起點＋1）後修正；使用者逐題確認 P1–P10，含 P4 改掉第 11 施工關 T6（重連一律重拿全貌）、P6 把第 11 施工關 G4 移到第 12 施工關。
- 2026-09-26 第 3 輪 review REFUTED（1 MEDIUM、數個 LOW）後修正：`0003` 把現有 `failed` 的 codex／opencode 設成已建立、加 CHECK 與 schema fixture；`retry` 先 `Shutdown` 留著的 holder；claude／0 的說明、log 字樣改 `start --resume`；游標規則寫成「最舊 − 1」；`actions: []` 的顯示；`get_fleet`／`resolve_attention` 的回應型別。
- 2026-09-26 第 2 輪 review REFUTED（2 MEDIUM、5 LOW）後修正：`retry` 靠 migration `0003` 的 `session_started` 決定 `--resume`／`--session-id`，codex／opencode 沒跑起來過才給 `retry`；不帶游標維持 1.0 的重播 backlog；`attention_required` 加 `instance_id`；錯誤碼加 `unknown_attention`、身分先於 id 檢查；`inbox` 不預先算可重做；步驟 5 的 `daemon_probe` 參數順序。
- 2026-09-26 fresh-context review REFUTED（4 MEDIUM、7 LOW）後修正：慢 client 分「讀得慢→`event_gap`」與「不讀→5 秒逾時關閉」；`failed` 目前沒有再試的方法（`plan_boot` 不碰）；游標比最新還新也回 `event_gap`；client 自己的 `ClientHello`；`failed` 的終端只回最後畫面；`waiting_since` 不加 migration；標出與已追認 T6／G4 不同之處；第 11 施工關 B 段步驟 2 的建議；步驟 5 先開 watch 再起 daemon。
- 2026-09-26 開工前提案 P1–P10 寫定（draft PR），待使用者確認；G4 建議移到第 12 施工關（P6）；「你親自驗收」改成 7 步；狀態改為提案中。

## 下一步

```bash
cat docs/gates/gate-08-client.md
~/.cargo/bin/cargo xtask accept client
```
