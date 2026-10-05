# Screen fixtures

## Claude 2.1.284 啟動診斷（2026-10-06）

`claude-2.1.284-main-100x24-3.txt` 來自使用者另行授權的一次 production 啟動診斷；
固定計畫 SHA-256 `4cda8e7e23b249dc8cac0aacf9b0edf3b947e9d0f87da251a79e5b068c993360`。
一個 instance、90 秒期限、四份只讀 frame、零工作訊息；正式啟動寫三個已知鍵，沒有人工鍵。
四份原始 frame 文字相同；採第一份 revision 25、100×24、normal live viewport，
來源 JSON SHA-256 `d29cf2380e7aea4336723b12eb5b59ecef55142fa61b911b7a4ca010066b0a8a`。
逐列串接非 leading spacer 的 cell text，只遮自有 canonical workspace；保留 24 列、空白與 NBSP。
fixture SHA-256 `e91471a74104aaff892c0e37d42cfc65effbf41fe5b03db32f1f8b1444b2f165`。

此完整畫面相對既有 `100x24-2` 的 token 差異只有 `Try "create a util logging.py that..."`。
全新 verifier 在原 classifier 核四份皆 None，只替提示為舊值才 Ready；footer 已由舊 fixture 涵蓋。
新增的是一份完整 literal，沒有把 Try 提示或未知內容放寬成 wildcard；其他未錄製提示仍拒絕。
結果是 **CAPTURED，非 smoke PASS**；沒有 transcript／usage 的留存證據，不能推斷 API 次數。
必要 raw frames、export manifest、執行前授權紀錄及獨立覆核保存於
`/Users/suzuke/Documents/Hack/AgEnD-ops/g12a-live-smoke-20261006/`。

## Claude 2.1.284 主畫面（既有授權蒐證，2026-10-05）

`claude-2.1.284-main-100x24-{0,1,2}.txt` 是 JSONL line 18／19／20 的原 `text`，revision 22／24／25；
來源 SHA-256 `9e1cd43f943704ea99ef3aca66fe1391881d65fb7151e48896c976bbc9975178`。
`claude-2.1.284-main-140x24-{0,1}.txt` 是 line 17／18，revision 25／26；
來源 SHA-256 `0b47ca7d77a94c93f33b9e26e911d33788115bdf05b60c2790a6af74ddc9ca3e`。
五份直接匯出、不改寫畫面；兩次各三個 operator key，零模型 prompt／團隊訊息。
正式啟動 classifier 比對完整 token、獨立核 canonical workspace；規則與驗證邊界見
[正式啟動處理](../../../../../docs/gates/gate-12a-startup-runtime.md)。本批沒有新增真 CLI 執行。

舊 Claude 檔案僅含版本化 spike 的 prompt 片段；Codex 檔案來自 holder PTY。
新增的 Claude 2.1.284 檔案來自實際 daemon／holder 保存的完整 24 列 frame 文字，
保留兩種寬度的換行與空白；路徑經蒐證工具遮蔽。

| Fixture | Evidence |
|---|---|
| `codex-folder-access.txt` | PTY capture from Codex CLI 0.156.1; `docs/research/spike-codex.md`, S6 |
| `claude-workspace-trust.txt` | Prompt excerpt from Claude Code 2.1.281 spike notes; default selection is described in the notes, not screen text |
| `claude-mcp-trust.txt` | Prompt excerpt from Claude Code 2.1.281 spike notes |

The classifier currently implements captured startup trust and development-channel prompts. Usage limits,
permission, rate-limit, authentication, and context-full patterns are deferred
until holder captures provide exact screen text. Add a rule only with a
corresponding fixture and backend/version evidence.

## Claude 2.1.284 被動蒐證（2026-10-05）

| Fixture | Evidence |
|---|---|
| `claude-2.1.284-workspace-trust-100x24.txt` | 真 Claude 2.1.284／holder，100×24；JSONL SHA-256 `1c4fbe2ac69b4ce91d5367f726a340c43d7166aad0472a21acc037e83172ea68`；預設 `No, exit`，未送鍵 |
| `claude-2.1.284-workspace-trust-140x24.txt` | 真 Claude 2.1.284／holder，140×24；JSONL SHA-256 `520eec42d6e64933dbe611bce14016ab1797589d53fa9356e01cfde065f8ed58`；預設 `No, exit`，未送鍵 |

固定 executable SHA-256：`50a14c2f50f56668380fdda490167f1d3630d5cc18fb8aed3073c2c7ea7314fe`。
同一檔案另查 `--version` 得 `2.1.284 (Claude Code)`；蒐證器本身未查版本，
JSONL 的 `version_was_queried=false` 保留原值。兩次各 20 秒、0 模型回合、0 輸入及訊息投遞。
原始遮蔽 JSONL、查版本結果與完整命令保存於
`/Users/suzuke/Documents/Hack/AgEnD-ops/g12a-native-checkpoint-20261005/`。

這些 fixture 只證明信任畫面的文字與既有 hard-gate 規則；未接受信任選項，
沒有 development channels／啟動完成畫面，不認證 P5 自動按鍵、P6 初始 idle 或 conformance。


## Claude 2.1.284 受控蒐證（2026-10-05）

使用者另行授權後，核同一 executable 版本／SHA-256，以 100×24、140×24 各蒐證 20 秒。
每次只送一次 Down、一次 Enter 接受本次自有 workspace；0 模型回合、0 訊息。
下列檔案逐 byte 匯出遮蔽 JSONL 的 `msg.text`，沒有改寫提示或空白。

| Fixture | Evidence |
|---|---|
| `claude-2.1.284-workspace-trust-selected-yes-100x24.txt` | 真 holder revision 7；來源 JSONL SHA-256 `8a1aab792cebc1e4086bb99a106ff20c42119061bfe3f8776509b8f544da3553` |
| `claude-2.1.284-development-channels-100x24.txt` | 真 holder revision 16；來源 JSONL SHA-256 `8a1aab792cebc1e4086bb99a106ff20c42119061bfe3f8776509b8f544da3553` |
| `claude-2.1.284-workspace-trust-selected-yes-140x24.txt` | 真 holder revision 7；來源 JSONL SHA-256 `520fca6a6d3a85376e5b0bee65d486bcc8a9688ac3e2c269c2acf7889b716041` |
| `claude-2.1.284-development-channels-140x24.txt` | 真 holder revision 14；來源 JSONL SHA-256 `520fca6a6d3a85376e5b0bee65d486bcc8a9688ac3e2c269c2acf7889b716041` |

兩次均停在 development channels 提示，沒有確認該提示；`startup=not_assessed`。
新規則只將這個已錄製提示辨識為 StartupMenu，沒有 suggested key。
這批使用 operator Input，未認證正式 P5 DaemonKey／revision CAS、P6 初始 idle 或完整 conformance。
自有程序／workspace 與兩筆暫存 workspace 的個人信任紀錄已清理，其他設定值保持原值。
必要 JSONL、命令、版本／清理及匯出 manifest 保存於上述 AgEnD-ops。

下一步：見 [蒐證頁](../../../../../docs/gates/gate-12a-startup-capture.md)。
