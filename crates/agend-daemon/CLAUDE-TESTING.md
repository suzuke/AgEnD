# Claude 接入與蒐證測試

> **TL;DR**
> - bridge／Driver 測試用真 daemon、holder、SQLite 與 native producer；不呼叫真 Claude 或模型。
> - startup capture 只驗蒐證工具，`startup=not_assessed`；正式 P5／P6／完整 12A 尚未完成。
> - 下一步：建置自有 `agend`，跑下方 native capture 與 daemon 測試。

## 第 12A bridge 基礎

`agend/tests/claude_bridge.rs` 執行真 daemon／holder／helper 與 SQLite，驗 idle channel、busy Stop、防迴圈、Sent／明確 ACK、四次開機、離線 helper 退出後自動 ingest、caller／版本／session 拒絕、壞 spool 不阻擋後方 ACK，以及真 Stop stdout 背壓後 unknown 不重送。native MCP 輸入使用 testkit producer；無真 Claude／模型。[重驗與限制](../../docs/gates/gate-12a-bridge.md)。

## 第 12A Driver 與設定（施工中）

`driver::claude::launch::tests` 驗原生 settings／MCP JSON、六種 hooks、ACK 指示、inbox 不寫檔或加旗標、外來與修改檔保留、SHA256 ownership 重開及發佈後 DB 失敗的拒絕恢復。`driver::claude::tests` 對真 SQLite 驗 failed instance 不自動放棄未送訊息、重開後仍可預約、未知結果只由明確 ACK 修復，以及超過 1024 筆不相關 hook／其他 instance 事件不遮蔽後續 Stop。原回歸先在修正前驗出 Failed／Queued 差異。

`agend/tests/claude_bridge.rs` 已接入完整 DRV 十個共用案例及 native Git pipeline 回歸，包含 busy Steer 的單一 Esc／立即 channel／不假造 ACK，以及人工 holder owner 保留控制、訊息仍 queued。結果不明的 40 則分頁、人的明確放棄、ACK 競爭及四次原生開機已通過；DRV 與 task／review 已分別通過；啟動提示及真 CLI 版本一致性仍未完成；詳見[本批進度](../../docs/gates/gate-12a-driver.md)。

`agend/tests/claude_process.rs` 對 native daemon／holder 驗 Claude 孤兒 group 的在線與下次開機清掃、failed 且 holder 存活時保護，以及外來 CLAUDE.md 的三次重試後 failed。shell agents 不呼叫模型，精確 argv 的正反例另在 `driver/claude/sweep.rs`。Driver 的 receipt 回歸核實實際 Written 才 sent、忙碌／其他 session 不等待，舊 idle 逾時仍 queued 且不開始投遞。

`actual_driver_routes_native_content_once_across_four_daemons_and_new_home_is_independent` 經真正的 agent send handler → BackendDriver → ClaudeDriver、native channel 與 ACK，核四次程序開機只一個投遞識別碼、sent 不代 confirmed、新 HOME 不共用冪等狀態。這是實際組合回歸，不宣稱完整 DRV-1–9 已執行。

`pipeline_store_ports` 另驗 Claude 回條只觀察 Driver/helper 狀態、task CAS 不冒充 ACK、晚到 ACK 與舊回條不倒退 confirmed；公開通用訊息狀態 API 仍拒絕 Claude 假確認。`claude_control_loss` 在真 PTY 消費 Esc 後丟原 holder 回覆，四次開機不重送鍵或內容；共用 DRV 的 restart 案例包括一個 seed 及四個獨立 composition child，新 HOME 的反向在 boot 2 因游標歷史遺失而失敗。

第 12A CI 前提修正：`tests/common/pipeline_process.rs` 的 approve helper 等同一真 FleetView 中的階段與 attention，避免讀到分次發布空窗；產品核准流程及 60 秒測試期限維持原值。Claude launch terminator 拒絕的單元斷言核真正 instance workspace 的 CLAUDE.md／.mcp.json，不只核 HOME。

`tests/claude_startup_capture.rs` 用自己的 shell producer 經真 daemon／holder 核兩種寬度、繁中字元、正式 argv、零輸入與成功／拒絕畫面後的清理；hash 不合與既有證據拒絕時不啟動 producer。這是蒐證工具回歸，不是真 Claude／P5 通過；[工具範圍與指令](../../docs/gates/gate-12a-startup-capture.md)。

啟動畫面蒐證另驗 native soft-wrap：電郵完整遮蔽、跨列 Bearer 前綴拒絕且清理。原逐列插 newline 的反例由 fresh verifier 用真 holder 重現；工具現依 cell.wrap／leading_spacer 還原 logical line 後才掃描，不把 physical row 邊界當作資料分隔。

受控 trust 蒐證另有 native producer：只對本次 workspace 的 No→Yes 送一次 Down／Enter；
未知、外來路徑、初始 Yes、No 未切換及未驗版本／尺寸的正反例核輸入 bytes 與清理。
兩寬另核外部路徑共用自有前綴、自有路徑出現在錯誤標頭以外的位置及重複標頭均零輸入；
workspace 必須是唯一 `Accessing workspace:` 標頭的下一個非空完整行。
選到 Yes／後續畫面是替身生成，不是真 CLI fixture；正式 P5 daemon-key／revision CAS、
P6 初始 idle 仍未認證。預設被動模式保持 0 輸入。

Development channels 蒐證用真提示文字經 native producer 重播，兩寬核三次輸入
`ESC[B CR CR`；外來前綴／額外 server、重複 Channels、Exit、缺完整 warning
及未完成 trust 前出現選單都不確認。預設只送兩鍵，缺 trust opt-in 在啟動前拒絕；
第三鍵後提示仍在也不重送。確認後畫面為 synthetic，不認證真 startup complete。

`foreign_frame_after_trust_completion_never_authorizes_development_input` 經自有 proxy 轉送真 daemon wire；確認 trust ACK 已轉送後才改 native frame 的 instance／view。兩寬均拒絕第三鍵並清理，proxy 不手造 frame。

`inconsistent_frame_before_resize_ack_stops_without_input` 用同一 native proxy，在真 resize ACK 前傳遞真 server 畫面的欄位不一致版本；兩寬 instance／view／generation／size 都必須失敗且零 Input。正常 resize 的原始與目標尺寸仍可過渡。

`trust_confirmation_rejects_extra_or_repeated_selections_at_both_widths` 以真 No fixture 經 native holder 產生兩寬畫面；No／Yes 各追加 selected Exit 或重複選項，須拒絕當次下一鍵且清理。trust 完整 fixture 只替換本次 canonical workspace，其他額外內容均 unknown。

完整提示穩定一秒後才送下一鍵，期間持續讀 frame／control；未知提示清除候選，不重送。
`prompt_stability_survives_delayed_receiver_and_resets_for_unknown` 用真 holder／兩寬 fixture，在畫面出現後 750ms 才啟用 raw receiver；原 consumer 先失敗，新 consumer 核恰好兩鍵；中途未知提示後，真 producer 核第一鍵仍等恢復 No 超過 900ms。
`private_cleanup_identity_matches_native_session_and_removed_workspace` 以 native argv 的 session 核私有 `cleanup-identity.json`，限定欄位與 0600，原生 workspace／home 已移除。

## 已核准的真蒐證重核（唯讀）

固定 `e0cedfb` 的 100×24／140×24 各完成三次 operator Input，保存 13／11 個真 frames 與主介面；另行版本查詢為 2.1.284。全新 verifier 只核保留證據一致性與指定殘留目前不存在，7 個記憶體 mutation 均拒絕；未啟動 backend 或模型。正式 P5／P6、歷史原始清理重演與首次 Down 無效原因未認證。完整限制見 [獨立核對](../../docs/gates/gate-12a-startup-capture-review.md)。

```bash
python3 -B /Users/suzuke/Documents/Hack/AgEnD-ops/g12a-native-checkpoint-20261005/true-ready-capture-fresh-verifier/audit.py --self-test
```

此指令不寫檔或送鍵，讀固定 git object、binary 與必要證據；缺檔或 hash 改變即拒絕。

## 下一步

```bash
export AGEND_BIN="$CARGO_TARGET_DIR/debug/agend"
cargo build -p agend --bins
cargo test -p agend-daemon --test claude_startup_capture
cargo test -p agend-daemon
cargo xtask check-deps
```
