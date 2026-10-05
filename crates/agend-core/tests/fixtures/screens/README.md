# Screen fixtures

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
