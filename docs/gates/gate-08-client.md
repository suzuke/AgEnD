# 第 8 施工關：agend-client + protocol server（整合施工關）（`client`）

> **TL;DR**
> - 真 daemon 開 `run/daemon.sock` 講 client protocol；`agend-client` 同步連線、daemon 重啟時重試 10 秒、版本不合立刻說清楚；順便補上第 11 施工關的協定缺口 G1–G3（G4 移到第 12 施工關）。
> - 記住：**自動驗收全綠還不夠**；你親自跑完「你親自驗收」並填「驗收紀錄」，這個施工關才算完成。
> - 下一步：逐題確認下面的開工前提案 P1–P10（每題都有建議，可以只回「照建議」）；第 6 施工關 merge 後開工。

**先看這條**：這頁的步驟會用到 `agend`。每個新開的終端機分頁（包括第二個終端）都要先跑「你親自驗收」開頭的設定，否則會跑到舊的 Node 版 `agend` 1.24.0。

## 狀態

**提案中**（2026-09-26）：開工前提案 P1–P10 待你確認；依賴的第 6 施工關（PR #125）等 merge。

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
- [ ] 待你確認

### P2：怎麼知道是誰在連（身分、要不要 cookie）

- 問題：D17 說 daemon 依呼叫者身分限制權限。unix socket 上要不要 cookie？身分從哪來？
- 建議：
  - **不做 cookie、不做 token**。能連上 socket 的只有同一個使用者（P1 的 0700／0600）。
  - `hello` 多一個選填欄位 `caller`：agent 裡的 CLI 填 `AGEND_INSTANCE`（daemon 起 agent 時設的，第 6 施工關 H3）；沒填就是操作者。
  - 這是跟 binding 快照同級的安全帶（D6）：同 uid 的 agent 可以不填或亂填，擋的是「agent 照說明跑錯命令」，不是惡意 agent。
  - 本關只有一個地方用到身分：`resolve_attention`（P5）只接受操作者；agent 送來回 `forbidden`，訊息附正確做法。agent 命令的權限在第 9 施工關做。
- 理由：v1 為了 TCP loopback 做 `api.cookie`，後來又加 `api.operator` 第二把，註解自己承認同 uid 的 agent 讀得到檔案、隔離「沒有解決」（v1 `src/auth_cookie.rs`）。unix socket 靠檔案權限就擋掉其他使用者，cookie 在同 uid 下多擋不了什麼。
- 替代方案：peer credential（`SO_PEERCRED`／`LOCAL_PEERPID` 拿 pid，再往上找屬於哪個 holder）：比較難假冒，但兩個平台寫法不同，agent 用 double fork 脫離 holder 也能繞過；cookie（v1 的路，同 uid 一樣繞得過）。
- 例子：agent `g8-1` 裡 `agend debug ping` 送 `{"type":"hello","caller":"g8-1",…}`；它若送 `resolve_attention` 會拿到 `forbidden: only the operator can resolve needs-you items; ask the operator with agend ask`。
- [ ] 待你確認

### P3：協定版本與錯誤碼

- 問題：本關要在協定加東西（P4、P5）。版本號怎麼動？舊的 1.0 peer 怎麼辦？假 daemon 自己發明的錯誤碼要不要固定？
- 建議：
  - 本關所有新增算一次 minor：client protocol **1.1**，`SUPPORTED_VERSIONS = [1.1]`。只加選填欄位與新的請求／事件，1.0 的 peer 解得開（新的變成 `unknown`），D26 不變。
  - client 需要 1.1 的功能（`get_fleet`）卻協商到 1.0：立刻失敗，訊息 `the daemon speaks client protocol 1.0; this agend needs 1.1 — restart the daemon with this binary`，不重試。
  - 錯誤碼變成 core 的常數（`protocol::client::error_code`）：`hello_required`、`version_mismatch`、`invalid_request`、`unknown_request`、`unknown_ask`、`stale_result`（已有）、新增 `event_gap`、`not_supported`、`forbidden`。假 daemon 改用 core 的常數。
  - 唯一的 wire 格式：真 server、假 daemon、`agend-client`、testkit 的 `ProbeClient` 都用 `serde_json` 編 core 型別，沒有第二份手寫格式（#1493）。
- 理由：一次 minor 讓「協商到舊版要說清楚」這條路本關就被測到；錯誤碼只有一份，CLI（第 9 施工關）才能依錯誤碼決定訊息與 exit code。
- 替代方案：不升版本（v2 還沒發布，可以直接改）——但 D26 的相容規則就一直沒被真正用過；每加一項升一次 minor（版本號變多、沒有好處）。
- 例子：假 daemon `set_supported_versions(&[1.0])` → `agend debug ping` 印上面那行錯誤、exit 1，而且立刻結束（不等 10 秒）。
- [ ] 待你確認

### P4：G1 全貌與事件游標

- 問題：TUI 需要 team、task、agent 的清單與結構化狀態（G1）。要做很多個 list 請求，還是一個快照？重連時怎麼保證不漏、不重複事件？
- 建議：
  - **一個請求** `get_fleet`，回 `fleet`：`as_of_event_id`、`teams`、`tasks`（含關卡清單與目前關卡）、`instances`（含 backend 與 `state`）、`attention`（目前的「需要你」清單）。名稱用「全貌」（fleet view），避開名詞表裡已經很多的「快照」。
  - 接著 `subscribe_events { after_event_id: as_of_event_id }`，之後的變化靠事件；`instance_changed`、`task_changed` 各加一個選填欄位帶新的結構化內容（1.0 的文字 `summary` 保留）。
  - agent 的 `state`：`starting`／`working`／`idle`／`stuck`／`failed`／`unknown`。本關真 daemon 只給得出 `starting`、`failed`、`unknown`（忙碌／閒置要 driver，第 7、12 施工關）。「需要你」不放進 `state`：由 client 從 `attention` 算（第 11 施工關 T18 已經這樣做），只有一個真相。
  - 事件 id 從「開機時間（unix ms）× 1000」開始往上數，事件只放記憶體（最近 1024 筆）。`after_event_id` 比留著的還舊（包括上一次開機的 id）→ `event_gap`，client 重拿全貌。所以規則只有一條：**重連一律重拿全貌**。
  - 真 daemon 本關能填的：`teams` 固定一個 `general`（D12；team 表由之後的施工關加）；`tasks` 照 DB 現有的列，關卡清單第 10 施工關補；`instances` 來自 DB 與 supervisor。
- 理由：一個請求、一個 `as_of`，全貌和事件之間沒有縫；很多個 list 請求各自有時間差。事件不存 DB：全貌本來就能從 DB 重建，重啟後重拿比記錄一份跨重啟的事件日誌簡單；id 以開機時間為底，舊 id 一定比新的小，不用另外的欄位。
- 替代方案：每種東西一個 list 請求（G1 原本的寫法，要自己處理 list 與訂閱之間的事件）；事件存 DB 讓 client 跨重啟接續（多一張表、多一套保留規則）；每次開機 id 從 1 開始（舊 client 的游標會「剛好在範圍內」而悄悄漏事件）。
- 例子：TUI 連上 → `get_fleet` 回 `as_of_event_id=1790000000000000`、1 個 team、2 個 instance → 訂閱；daemon 重啟 → 連線斷 → 重連、重拿全貌（新的 `as_of` 比較大）；如果拿舊游標訂閱，回 `event_gap`。
- [ ] 待你確認

### P5：G2、G3「需要你」的欄位與操作

- 問題：`attention_required` 沒有 id、放行數、等待時間、「不處理的話」（G2、T16）；非請示的項目沒有操作，也不會消失（G3）。怎麼補？本關有真的來源嗎？
- 建議：
  - `attention_required` 加選填欄位：`attention_id`（請示用 ask id；其他是固定的字串，例如 `instance-failed:g8-2`）、`unblocks`、`waiting_since_unix_ms`、`if_ignored`、`actions`。欄位名對上 core 已有的 `policy::attention::AttentionItem`，排序照 D36。
  - 新請求 `resolve_attention { attention_id, action }`（只收操作者，P2）；新事件 `attention_resolved { attention_id, action }`。請示照舊用 `answer_ask` 與 `ask_updated`。
  - 本關的真來源：supervisor 放棄的 instance（第 6 施工關 P6「交給人」）。`actions` 只有 `retry`：清掉 `failed`、重算重起次數、帶 `--resume` 再起。沒有 session id 可接（codex、opencode）就不給 `retry`（P6 不全新啟動）。
  - 「需要你」清單不另存表：每次從 DB 的 `failed` instance 算。`waiting_since` 要跨重啟，所以 migration `0003` 在 `instances` 加一個可空的 `failed_at_unix_ms`。`unblocks` 是它手上的 task 數（第 10 施工關前一律 0）。
  - 其他操作（暫停、改派…）等有來源的施工關再加：`actions` 的 enum 有 `unknown`，舊 client 看到新操作不會壞。
- 理由：G3 有真來源才驗得到「操作 → daemon 決定 → 消失」這條路，不用發明假的項目；`failed` 本來就要人處理，現在只能重啟 daemon 才會再試。
- 替代方案：G3 整個移到第 10 施工關（本關就沒有任何非請示項目可驗，第 11 施工關 B 段會缺一步）；`dismiss`（只從清單拿掉、instance 還是 `failed`，容易忘記）；另開 `attention` 表（本關只有一種來源，從 instance 算就夠）。
- 例子：`g8-2` 一起來就死 → 3 次後 `failed` → 事件 `attention_required {attention_id:"instance-failed:g8-2", unblocks:0, waiting_since_unix_ms:…, if_ignored:"g8-2 stays stopped", actions:["retry"]}` → 操作者送 `retry` → `attention_resolved`，daemon log `restart 1/3 --resume …`。
- [ ] 待你確認

### P6：真 daemon 本關做哪些請求（G4 移走）

- 問題：client protocol 有一堆請求，但真 daemon 還沒有 task、請示、訊息（第 9、10 施工關）。本關做到哪？G4 已讀狀態做不做？
- 建議：

  | 請求 | 真 daemon 本關 |
  |---|---|
  | `hello`、`get_fleet`、`subscribe_events`、`resolve_attention` | 做 |
  | `subscribe_terminal` | 做：先回 holder 當下的畫面，再轉送之後的 `terminal_bytes`（daemon 的 holder 長連線轉出來，client 絕不直接連 holder） |
  | `terminal_input` | 回 `not_supported`；第 11 施工關 B 段做（要先確定只有操作者能打字） |
  | `answer_ask` | 沒有請示，回 `unknown_ask`；請示在第 9、10 施工關 |
  | `command`（agent 命令） | 回 `not_supported`，訊息寫在哪個施工關做；第 9、10 施工關 |

  - **G4 已讀狀態移到第 12 施工關**：要 daemon 記已讀，唯一的理由是跟 Telegram 共用，而 Telegram 在第 12 施工關。在那之前 TUI 用本機已讀（第 11 施工關 T4、T17）。
- 理由：只做有真資料的請求；其餘回明確錯誤而不是假資料。已讀要存 DB（新表、保留規則），沒有第二個讀者時做了驗不到「共用」。
- 替代方案：本關把 G4 也做完（多一張表，第 12 施工關前沒有第二個讀者）；`terminal_input` 本關一起做（要先定操作者限制，會把第 9 施工關的權限提前）。
- 例子：`command {status}` → `error not_supported: agent commands arrive in gate 9 (agend status)`，連線不斷。
- [ ] 待你確認

### P7：agend-client 的 API、重試、錯誤訊息

- 問題：client 怎麼重試才能讓「命令執行中重啟 daemon」最後仍成功？什麼情況不重試？第 9 施工關 CLI 需要什麼？TUI 的重連跟它是什麼關係？
- 建議：
  - API（同步、不建 runtime）：`Client::connect(socket, caller)`：連線＋`hello`，連不上每 100 ms 重試，最多 10 秒（`RESTART_RETRY_WINDOW`）；`Client::connect_once`：只試一次，給有自己重連畫面的 TUI（第 11 施工關 T6 每 500 ms）；`request(…)`：送請求、依 `request_id` 等回應，最多等 10 秒；`events()`：阻塞讀下一個事件。
  - 什麼會重試：socket 不存在、連線被拒、請求送出前連線就斷。**請求送出後**斷線，只有呼叫端標明「可重做」的請求才重送（讀取類：`get_fleet`、`status`、`inbox`）；其他回 `daemon restarted during the request; check with agend status`。哪些 agent 命令可重做由第 9 施工關逐一決定。
  - 什麼不重試：版本不合（P3）、`forbidden`、其他 daemon 回的錯誤。
  - 10 秒到了的訊息：`cannot reach the AgEnD daemon at <path> after 10 s (<原因>). Is it running? Start it with: agend daemon`，exit 1。
  - socket 路徑由呼叫端給（`agend` 從 `AGEND_HOME` 算）；client 不讀環境變數與設定檔。
  - 本關的命令（`agend debug …`，唯讀）：`agend debug ping [--count N --interval MS]` 印協定版本與 instance 數；`agend debug watch` 印全貌摘要與之後的事件，斷線時自己重連、重拿全貌。
  - 給第 9 施工關：錯誤型別分「連不上／版本不合／daemon 回錯誤（含錯誤碼）」三種，CLI 依此選訊息與 exit code；`caller` 由 `agend` 從 `AGEND_INSTANCE` 填。
- 理由：10 秒是規劃定的；只重送讀取類，避免 daemon 重啟時同一個命令被做兩次（v1 的訊息重複類問題，V1-LESSONS #1）。TUI 本來就有自己的斷線畫面與重連，不必吃 10 秒的阻塞。
- 替代方案：所有請求都重送（非冪等的命令會重複）；靠 `request_id` 讓 daemon 去重（要跨重啟記住，等於多一張表）；client 自己讀 `AGEND_HOME`（client 就依賴環境，測試難隔離）。
- 例子：daemon 沒在跑：`time agend debug ping` → 約 10 秒後印上面那段錯誤、exit 1。`agend debug ping --count 20 --interval 500` 跑到一半重啟 daemon：20 行都成功，中間一兩行是 `ok (retried 1.4 s)`。
- [ ] 待你確認

### P8：server 的 thread 模型、多個 client、慢的 client

- 問題：server 跑在 tokio 還是自己開 thread？同時很多 client 怎麼辦？有個 client 不讀（例如 TUI 卡在很慢的 ssh），會不會拖住 daemon？
- 建議：
  - 跟第 6 施工關一致：用 daemon 已有的 tokio multi-thread runtime，加 `net` feature；每個連線一個 task。命令處理在 `handlers`，`server` 只管 socket 與 JSON Lines（第 6 施工關 `server.rs` 的 `Must NOT` 已寫）。
  - 事件用一個 `tokio::sync::broadcast`（1024 筆）送給每個訂閱者。某個 client 落後到被覆蓋（`Lagged`）→ 送 `error event_gap`、關掉**它**的連線；它重連、重拿全貌（跟 daemon 重啟同一條路）。
  - 寫入加 5 秒逾時：完全不讀的 client 5 秒後被關掉。
  - 終端串流：holder 長連線把 `PtyBytes` 轉進每個 instance 一個 broadcast（256 塊）；落後也是關連線，重連時先拿到當下畫面。
  - 不限制連線數；CLI 的連線是一次性的。
- 理由：一條「落後就斷、重連重拿」的規則，慢 client 不會拖住 daemon 或其他 client，也不用替每個 client 排無限長的隊（v1 曾因為死掉的 TUI 訂閱者沒被清掉而修過，#3682）。重連本來就要做，不多一條路。
- 替代方案：每個連線一條 std thread（像假 daemon；但 handler 要呼叫 async 的 store）；每個 client 無上限的佇列（慢 client 讓 daemon 記憶體一直長）；落後時跳過事件繼續送（client 畫面悄悄變錯）。
- 例子：測試開一個 client 訂閱後不讀，daemon 連續發 2000 個事件：另一個正常 client 全部收到、順序正確；不讀的那個收到 `event_gap` 後被關。
- [ ] 待你確認

### P9：client 協定契約：假 daemon 與真 daemon 跑同一套

- 問題：TUI、CLI 的測試都對 testkit 假 daemon 跑。怎麼保證假 daemon 跟真 daemon 行為一樣？
- 建議：
  - 在 [CONTRACTS.md](../../crates/agend-testkit/CONTRACTS.md) 加一張表 `CLP`（client protocol），每條一個編號；suite 放 testkit，對**兩個** server 跑：`FakeDaemon` 與真的 `agend daemon`（暫存 home、真 binary，放 `crates/agend/tests/`，比照第 6 施工關 H12）。
  - 規則草稿（開工時定稿）：`hello` 必須第一個；major 不合回 `version_mismatch` 並關閉；`get_fleet` 的 `as_of` 之後訂閱，事件不漏不重；舊游標回 `event_gap`；未知請求回錯誤、連線不斷；兩個 client 收到同樣的事件、同樣的順序；落後的 client 被關、其他不受影響（P8）；server 重啟後重連、重拿全貌；`not_supported` 的請求不改任何狀態。
  - 每條至少一個故意弄壞的 mutant（比照第 2 施工關的規則表）。
  - `agend-client` 的測試對假 daemon 跑；契約的驅動端用 testkit 的 `ProbeClient`，不用 `agend-client`，這樣 client 的 bug 不會被同一份程式碼蓋掉。
  - 假 daemon 補上本關新增的請求與規則（`get_fleet`、`resolve_attention`、事件 id 起點、落後就斷），讓第 9、11 施工關的測試對得上真 daemon。
- 理由：v1 #1483 的假綠就是測試餵了 production 從不送的格式；同一套規則跑兩邊，假 daemon 一偏離就失敗（#1493）。
- 替代方案：只對真 daemon 測 client（慢，而且 TUI／CLI 的測試仍然對假 daemon）；各寫各的測試（兩邊會慢慢漂移）。
- 例子：假 daemon 的事件 id 從 1 開始（沒照 P4）→ CLP 的「舊游標回 `event_gap`」對假 daemon 失敗、對真 daemon 通過，一眼看出是假的偏了。
- [ ] 待你確認

### P10：依賴規則

- 問題：`agend-client` 要保持輕，`check-deps` 要加什麼？
- 建議：
  - `agend-client` 保持現有禁止清單（async runtime、SQLite、`agend-daemon`）；一般依賴只有 `agend-core` 與 `serde_json`。
  - 新規則：`agend-daemon` 不能依賴 `agend-client`（server 與 client 各自編碼，契約才驗得到兩邊一致）。
  - `agend-tui` 的 lib 改走 `agend-client` 是第 11 施工關 B 段的事，本關不動 TUI。
- 理由：CLI 啟動要輕（D11 實測 p50 4.1 ms）；daemon 若借用 client 的程式碼，兩邊的 bug 會一起出現、互相蓋掉。
- 替代方案：不加 daemon 規則（靠 code review）。
- 例子：有人在 `agend-daemon` 的 `Cargo.toml` 加 `agend-client` → `cargo xtask check-deps` 失敗，訊息附 `cargo tree … -i agend-client`。
- [ ] 待你確認

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

- 第 6 施工關（PR #125）還沒 merge：本關的 branch 要從它 merge 後的 `v2` 開。
- bind 在開機計畫之後（P1）：instance 很多、每個 holder 起不來要等 5 秒時，開機可能超過 10 秒，CLI 會先放棄。本關要量開機時間；超過就改成先 bind、全貌標「開機中」。
- 事件 id 用開機時間當起點（P4）：系統時鐘往回調時，新的 id 可能比舊的小。實際影響是 client 多重拿一次全貌或少拿幾個事件；開工時測「時鐘往回」會怎樣，必要時改用 DB 裡遞增的開機序號。
- 終端串流要在 holder 長連線上同時處理 `PtyBytes` 與 `Snapshot` 回應（第 6 施工關的 link 目前丟掉 `PtyBytes`）：要確認「畫面＋之後的位元組」接得起來，不重不漏。
- 落後就斷（P8）：很慢的網路上 TUI 可能一直重連。先記 log（哪個 client、落後多少），實際遇到再調 buffer。
- 身分可以假冒（P2）：同 uid 的 agent 能假裝是操作者。跟 D6 同級，已知且接受；要更強只能換 peer credential。
- socket 路徑長度：macOS 的 `$TMPDIR` 很長，測試的 home 照第 6 施工關 H13 放 `/tmp/g8-<pid>-<n>`。

## 自動驗收（完成定義）

- [ ] `~/.cargo/bin/cargo test -p agend-client`、`-p agend-daemon`、`-p agend-testkit`、`-p agend-core` 單獨通過，包括：重試 10 秒後的訊息、版本不合立刻失敗、送出後斷線只重送可重做的請求（P7）；1.0 的 peer 解得開 1.1 的訊息、1.1 解得開 1.0 的訊息（P3）；`failed` → `attention_required` → `retry` → `attention_resolved`，沒有 session id 就沒有 `retry`（P5）；socket 0600、路徑太長拒絕啟動、停止時刪檔（P1）；agent 送 `resolve_attention` 回 `forbidden`（P2）
- [ ] client 協定契約 `CLP` 對假 daemon 與真 `agend daemon` 都通過，每條有 mutant；反向檢查：假 daemon 的事件 id 改回從 1 開始時契約必須失敗（P9）
- [ ] 慢 client：一個不讀、一個正常，2000 個事件後正常的全部收到且順序正確，不讀的被關（P8）
- [ ] `~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings` 乾淨
- [ ] `~/.cargo/bin/cargo xtask check-deps` 最後一行是 `… no-std build ok)`（出現 `SKIPPED` 不算通過），並有新規則：`agend-daemon` 不能依賴 `agend-client`（P10；故意加依賴會失敗）
- [ ] `~/.cargo/bin/cargo xtask accept client` 通過，並印出下方「你親自驗收」用到的 demo
- [ ] 本施工關 crate 的 `README.md`／`TESTING.md` 已更新；想改的共用文件（名詞表、AGENTS.md、第 11、12 施工關頁）列在 PR 裡由你決定
- [ ] 測試不留殘留：結束時沒有 `g8-` 或測試 id 的 holder、沒有留下的 `daemon.sock`；kill 只對自己起的、大於 1 的 pid
- [ ] fresh-context verifier 重跑並嘗試推翻；結果寫進「進度紀錄」

## 你親自驗收

由 agent 帶著一步一步做（見 [AGENTS.md](../../AGENTS.md#帶使用者親自驗收)）。每一步：照抄指令 → 對照「應該看到」→ 對了就打勾。任何一步不符就停，記在「驗收紀錄」。標「開工時細化」的地方，開工時會改成確切指令與輸出。

**每個新開的終端機分頁都要先跑這段**（包括 daemon 在前景跑時開的第二個終端）。第 13 施工關之前沒有安裝程式，而你的 PATH 上有舊的 Node 版 `agend`（v1-ts 1.24.0）：

```bash
cd ~/Documents/Hack/AgEnD-v2    # 你的 AgEnD-v2 路徑
~/.cargo/bin/cargo build -p agend && export PATH="$PWD/target/debug:$PATH" && agend --version
```

應該看到 `agend 0.x.y`（目前是 `agend 0.0.0`）。如果印出 `1.24.0`，跑到的是舊的 Node CLI——在這個終端機重跑上面那段。

步驟 2 起用同一個暫存 home。第二個終端也要貼上步驟 2 印出的那行 `export AGEND_HOME=…`。

1. 跑 demo。

   **這步在驗什麼**：同一套 client 協定規則對假 daemon 和真 daemon 都通過，版本不合、慢 client、重啟這幾段也跑完（P3、P8、P9）。錯了代表 TUI 與 CLI 的測試對的是一個跟真 daemon 不一樣的假東西。

   ```bash
   cd ~/Documents/Hack/AgEnD-v2
   ~/.cargo/bin/cargo xtask accept client
   ```

   應該看到：`== contract` 段每條 `CLP-n` 印兩次（`fake ok`、`real ok`）；`== version` 段有 `needs 1.1`；`== slow-client` 段有 `event_gap`；最後一行 `gate 8 (client): checks passed`（開工時細化確切輸出）。

   - [ ] 通過

2. 故意弄壞：daemon 沒在跑時連線。

   **這步在驗什麼**：client 會等 daemon（重啟時要用），但 10 秒後一定放棄並說清楚怎麼辦（P7）。錯了的話 CLI 不是永遠卡住，就是一句看不懂的 `Connection refused`。

   ```bash
   export AGEND_HOME="$(mktemp -d /tmp/g8.XXXX)" && echo "export AGEND_HOME=$AGEND_HOME"
   time agend debug ping
   ```

   應該看到：約 10 秒後印 `cannot reach the AgEnD daemon at …/run/daemon.sock after 10 s (…). Is it running? Start it with: agend daemon`；`echo $?` 是 `1`；`time` 約 10 秒。

   - [ ] 通過

3. 啟動 daemon，再連一次。

   **這步在驗什麼**：socket 在 daemon 好了之後才出現、只有你能連，連上馬上回協定版本（P1、P3）。錯了的話別的使用者能連，或 client 看到開機到一半的資料。

   操作（開工時細化 `daemon_probe` 參數）：第一個終端：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- add g8-1
   agend daemon
   ```

   第二個終端（先跑開頭那段、貼上 `export AGEND_HOME=…`）：

   ```bash
   agend debug ping
   ls -l "$AGEND_HOME/run/daemon.sock"
   ```

   應該看到：daemon 印 `agend daemon ready: instances=1 …`；`ping` 馬上印 `agend daemon: client protocol 1.1, instances=1`、exit 0；`ls` 那行開頭是 `srw-------`。

   - [ ] 通過

4. 命令執行中重啟 daemon。

   **這步在驗什麼**：這關的主要驗收：daemon 重啟時命令會等它回來，最後照樣成功（P7）。錯了的話每次重啟 daemon，正在跑的 agent 命令都會失敗。

   操作：第二個終端：

   ```bash
   agend debug ping --count 20 --interval 500
   ```

   它跑的 10 秒內，到第一個終端按 Ctrl-C，再馬上跑 `agend daemon`。

   應該看到：20 行全部是 `ok`，其中一兩行是 `ok (retried 1.4 s)` 之類；exit 0。

   - [ ] 通過

5. 兩個 client 看同一件事：agent 一直死，進「需要你」，再按重試。

   **這步在驗什麼**：事件同時送到每個 client；`failed` 的 instance 變成「需要你」，帶 id、等待時間、「不處理的話」與 `retry`；只有操作者按得了，按了才消失（P2、P5、P8）。錯了的話 TUI 看不到 agent 死掉，或看到了卻沒辦法處理。

   操作（開工時細化）：第一個終端 Ctrl-C 停 daemon，然後：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- add --dies g8-2
   agend daemon
   ```

   第二個終端：`agend debug watch`。再開第三個終端（先跑開頭那段、貼上 `export AGEND_HOME=…`）也跑 `agend debug watch`。

   應該看到：兩個 watch 都先印 `fleet: instances=2 attention=0`，接著同樣順序的 `instance_changed g8-2 …`，約 15 秒後 `attention_required instance-failed:g8-2 (unblocks 0, waiting since …; if ignored: g8-2 stays stopped) actions: retry`。

   操作：第三個終端按 Ctrl-C 停掉它的 watch，然後：

   ```bash
   ~/.cargo/bin/cargo run -q -p agend-client --example client_probe -- resolve instance-failed:g8-2 retry
   AGEND_INSTANCE=g8-1 ~/.cargo/bin/cargo run -q -p agend-client --example client_probe -- resolve instance-failed:g8-2 retry
   ```

   應該看到：第一行 `resolved`，第二個終端的 watch 印 `attention_resolved instance-failed:g8-2 retry`，daemon log 有 `restart 1/3 --resume …`（`--dies` 的 agent 還是會死，約 15 秒後又回到「需要你」，這是正常的）；第二行（假裝是 agent）印 `forbidden: only the operator can resolve needs-you items …`、exit 1。

   - [ ] 通過

6. 故意弄壞：watch 開著時重啟 daemon。

   **這步在驗什麼**：長時間連著的 client 在 daemon 重啟後自己重連、重拿全貌，不會用舊的事件游標漏事件（P4）。錯了的話 TUI 在 daemon 重啟後顯示過期的資料。

   操作：第二個終端的 watch 繼續開著；第一個終端 Ctrl-C，再跑 `agend daemon`。

   應該看到：watch 印 `disconnected: the daemon closed the connection`、幾行 `reconnecting…`，daemon 回來後印 `fleet: instances=2 …`（重拿全貌），之後的事件照常出現。

   - [ ] 通過

7. 收尾。

   **這步在驗什麼**：daemon 停止時刪掉 socket；本關的 instance 在下次開機被收掉，什麼都不留（P1；第 6 施工關的孤兒巡查）。錯了的話會留下 holder 或舊 socket。

   操作：第二個終端 Ctrl-C 停 watch；第一個終端 Ctrl-C 停 daemon，然後：

   ```bash
   ls "$AGEND_HOME/run/"
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- remove g8-1
   ~/.cargo/bin/cargo run -q -p agend-daemon --example daemon_probe -- remove g8-2
   agend daemon
   ```

   應該看到：`ls` 只有 `holders`，沒有 `daemon.sock`；daemon 印 `agend daemon ready: … orphans=…`。在第二個終端 `pgrep -fl "agend holder g8-"` 什麼都不印。最後按 Ctrl-C，再 `rm -rf "$AGEND_HOME"`。

   - [ ] 通過

## 驗收紀錄

由你填寫。

| 日期 | 結果（通過／不通過） | 備註 |
|---|---|---|
|  |  |  |

## 進度紀錄

日期 + 一行 + commit／PR，新的在上面。

- 2026-09-26 開工前提案 P1–P10 寫定（draft PR），待你確認；G4 建議移到第 12 施工關（P6）；「你親自驗收」改成 7 步；狀態改為提案中。

## 下一步

```bash
cat docs/gates/gate-08-client.md
~/.cargo/bin/cargo xtask accept client
```
