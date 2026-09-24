# 多 AI Coding Agent 協作／管理工具 — 市場調查（2026-09-24）

> 歸檔註：設計階段的原始紀錄，內容原樣保存（2026-09-24）。只把絕對路徑換成佔位符：`<scratchpad>` = 當時的暫存目錄、`<v1-repo>` = agend-terminal repo、`<tmp>` = 系統暫存目錄、`~` = 使用者家目錄。文中提到的腳本、log、schema dump 沒有一起歸檔。

調查目的：為 agend v2（本機常駐 daemon，管理多個 CLI agent 的「自主開發團隊」流水線：
派工 → worktree 隔離開發 → checks → agent 互相 review（綁 head SHA）→ daemon 自動 merge；
agent 互傳訊息；TUI+attach；Telegram 遙控；git shim 防護；daemon 熱升級不斷線）找差異化定位。

方法：WebSearch + `gh api` 查 GitHub 星數/授權/最近活躍度。查不到的一律標「未查證」，
不腦補。

---

## 比較表

圖例：✅有／❌無或未提及／⚠️部分／❓未查證

| 專案 | 形態 | 支援 agent | OSS+授權 | Star/Fork/最近活躍 | 多agent並行 | worktree隔離 | agent間通訊 | task board | agent互審 | 自動merge+門檻 | CI整合 | 手機遠端 | 卡住/usage-limit處理 | git防護 | daemon常駐 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **herdr** | TUI (單一Rust binary, agent-agnostic pane multiplexer) | 任何CLI agent(通用) | ✅ Apache-2.0(2026-07-22由AGPL轉) | 40,428★/3,082 fork，pushed 2026-09-23（極活躍）[gh api herdrdev/herdr] | ✅ pane並行 | ⚠️核心不內建，社群外掛(herdr-control)加 | ✅ **核心特色**：socket API讓agent互相spawn pane、讀輸出、互相等待 | ❌ | ❌ | ❌ | ❓未查證 | ⚠️僅社群fork(herdr-control) 2-way Slack bridge，非官方 | ❓未查證(有working/blocked/idle狀態偵測，但usage-limit未查證) | ❌未查證 | ✅核心賣點：背景runtime，reboot後session存活、任何終端/SSH重連 |
| **claude-squad** | TUI (tmux) | Claude Code, Codex, OpenCode, Amp, Gemini, Aider | ✅ AGPL-3.0 | 8,526★/622 fork，pushed 2026-08-20 [gh api smtg-ai/claude-squad]；但issue #250(2026-02)質疑專案是否已無人維護 | ✅ | ✅ tmux+worktree核心機制 | ❌ | ❌ | ❌ | ❌人工 | ❓ | ❌ | ❓ | ❌ | ⚠️tmux session持續，非真daemon |
| **Conductor** (conductor.build) | GUI (Mac app, 閉源) | Claude Code, Codex, Cursor | ❌閉源，App免費(需agent自己的訂閱) | 無公開repo；YC投資、$22M A輪 [rywalker.com/research/conductor] | ✅ | ✅ 核心機制，自動建worktree+branch | ❓未查證 | ❌(workspace清單非kanban) | ❌ | ❌人工diff review | ❓未查證 | ❌ | ❓ | ❓ | ❌桌面App，非headless daemon |
| **vibe-kanban** (BloopAI，sunsetting) / **easy-vibe-kanban**(社群fork) | Web UI + CLI | 10+ agents：Claude Code, Codex, Gemini CLI, Copilot等 | ✅ Apache-2.0 | 28,177★/3,023 fork [gh api BloopAI/vibe-kanban]；公司2026-04-10關門，轉社群維護 | ✅ | ✅ 每task自動branch+worktree | ⚠️fork加了「workflow canvas」序列串接agent，非雙向通訊 | ✅**核心賣點**kanban | ⚠️人工diff+inline comment，非agent互審 | ❌PR流程，人工merge | ❓未查證 | ❌無原生App | ❓ | ❓ | ❌web app，非常駐daemon |
| **Crystal→Nimbalyst** (stravu) | GUI桌面(Electron) | Claude Code, Codex, OpenCode | ✅ Nimbalyst桌面/iOS MIT；team server元件AGPL | Crystal 3,121★/197fork(deprecated 2026-02)→Nimbalyst 1,769★/260fork pushed 2026-09-23 [gh api] | ✅ | ✅ 核心機制 | ❌ | ⚠️任務追蹤，非kanban | ❌人工diff | ❌，有rebase/squash輔助但人工觸發 | ❓ | ✅ Nimbalyst有iOS/Android companion app | ❓ | ❌ | ❌桌面App |
| **uzi** | CLI (tmux+worktree) | Claude, Codex, Cursor, Aider等 | ✅ MIT | 583★/27fork，**pushed 2025-06-04**（一年多未更新，明顯停滯）[gh api devflowinc/uzi] | ✅ | ✅ 自動化核心機制 | ❌ | ❌ | ❌ | ⚠️一鍵checkpoint合併，但人工觸發非審核門檻 | ❌ | ❌ | ⚠️「自動處理agent prompt/confirmation」，未含usage-limit | ❌ | ❌ |
| **Sculptor** (Imbue) | GUI桌面App | 未明確限定(container包任何agent) | ✅ MIT | 232★/17fork，pushed 2026-09-23(當日)[gh api imbue-ai/sculptor] | ✅每agent自己container | ⚠️用container隔離而非git worktree | ❌ | ❌ | ❌ | ❌「pairing mode」人工帶回本機 | ⚠️會主動檢查程式碼問題(缺測試/race condition)，非CI串接 | ❌ | ❓ | ❌ | ❌桌面App非daemon |
| **Terragon**（terragon-labs，已關站） | Cloud | Claude Code, Codex等CLI | ✅ terragon-oss快照Apache-2.0（已停止維護） | 259★/43fork，pushed 2026-02-10，**公司已於2026-01-16關站**[gh api terragon-labs/terragon-oss] | ✅雲端沙盒並行 | ⚠️雲沙盒複本而非git worktree | ❌ | ❌ | ❓未查證 | ❌人工PR審核 | ❓ | ⚠️雲端網頁可用手機瀏覽器，無專用App | ❓ | ❓ | N/A(雲端服務關站=個案警示：純雲端商業模式風險) |
| **Omnara** | Web+Mobile+Desktop+Watch控制中心 | 多agent(經API接入，非worktree導向) | ✅ Apache-2.0 | 2,864★/228fork，pushed 2026-09-23(當日)[gh api omnara-ai/omnara] | ✅可同時跑多agent | ❓未查證(非其重點) | ❓未查證 | ❓未查證 | ❌ | ❌ | ❌ | ✅**核心賣點**：手機/Apple Watch/web全平台、語音優先 | ❓未查證 | ❌ | ⚠️伺服器端session持久化（雲端而非本機daemon） |
| **happy / happy-coder** (slopus) | CLI wrapper + Mobile/Web client, e2e加密 | Claude Code, Codex | ✅ MIT | slopus/happy 23,878★/2,037fork pushed 2026-09-22(極活躍)；舊repo happy-cli已archived 554★[gh api] | ✅可切換裝置操作多session | ❌非其重點 | ❌ | ❌ | ❌ | ❌ | ❌ | ✅**核心賣點**：手機/web、需要許可或出錯時push通知、一鍵裝置切換 | ⚠️「需要許可/出錯」推播，非專門usage-limit偵測 | ❌ | ⚠️server relay持久化，非本機daemon |
| **Claude Code官方 subagents/Agent Teams** | CLI內建功能(閉源) | 僅Claude Code自己 | ❌閉源 | N/A(產品功能非獨立repo) | ✅ Agent Teams(2026-02隨Opus 4.6推出，預設關閉、實驗性) | ❓未查證(非worktree為核心設計) | ✅**罕見**：teammate間mailbox系統+共享task list、peer-to-peer(而非僅回報給lead) | ⚠️共享task list，非kanban | ❌未見審核gate | ❌ | ❓ | ❓未查證 | ❓ | ❌ | ❌僅session內存活，關掉session team消失 |
| **OpenAI Codex cloud** | Cloud (ChatGPT/API) | 僅Codex自己 | ❌閉源 | N/A | ✅可同時派多雲端task | ⚠️雲sandbox隔離，非git worktree | ❌ | ❌(task/PR清單) | ❌人工審diff | ❌人工merge PR | ⚠️sandbox內可跑repo checks | ✅ChatGPT App可查看/派工 | ❓ | ❓ | N/A雲端服務 |
| **Cursor background agents / Origin** | GUI IDE+雲端+新Git forge(Origin, 2026-06公布) | Cursor自己(多模型) | ❌閉源 | N/A | ✅worktree或remote機器並行 | ✅ | ❌ | ⚠️Origin加「疊層PR+依賴圖」，非kanban | ⚠️人工，官方**明確建議不要**單靠審核agent自動merge | ❌**官方明確反對**auto-merge，堅持人工merge | ✅建議搭配CI跑測試 | ❓未查證 | ❓ | ❌ | ❌ |
| **GitHub Copilot coding agent / Copilot App** | Cloud+Desktop App(2026-06 Build公布) | 僅Copilot自己 | ❌閉源 | N/A | ✅Desktop App可並行多session，各自worktree | ✅ | ❌ | ❌(PR為中心) | ✅Copilot review bot審自己/他人PR，並處理change-request | ⚠️2026-09-01起可**opt-in**授權Copilot review自行批准PR（預設關閉，企業可控）；coding agent本身仍不能merge | ✅原生GitHub Checks整合 | ⚠️GitHub手機App可審/管理PR | ❓ | ❌ | ❌ |
| **ccmanager** | CLI/TUI | 8種：Claude Code, Gemini CLI, Codex CLI, Cursor Agent, Copilot CLI, Cline CLI, OpenCode, Kimi CLI(支援agent數最廣之一) | ✅ MIT | 1,248★/93fork，pushed 2026-09-13[gh api kbwo/ccmanager] | ✅ | ✅核心機制(各自worktree+branch) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ⚠️自動核准不需授權的prompt，減少人工介入，非usage-limit專用 | ⚠️僅止於「自動核准非敏感操作」，非攔截危險指令 | ❌需手動保持執行 |
| **agent-of-empires** | TUI + Web dashboard(可裝成PWA) | 14+種：Claude Code, Codex, Gemini CLI, Cursor, Copilot, OpenCode, Mistral Vibe, Antigravity, Pi.dev, Factory Droid, Hermes, Kiro, Qwen Code, Kimi Code(支援最廣) | ✅ MIT | 3,283★/361fork，pushed 2026-09-23(當日)，60+貢獻者[gh api agent-of-empires/agent-of-empires] | ✅ | ⚠️不同branch執行(隔離機制細節未查證是否為worktree)，可選Docker沙盒 | ❓未查證 | ❌ | ❌ | ❌ | ❌ | ✅**PWA可裝手機當App** | ❓ | ⚠️Docker沙盒為選項(圍堵而非攔截) | ⚠️tmux背景session持續存在，非可熱升級的daemon |
| **Google Jules** | Cloud(async) | 僅Jules自己(Gemini) | ❌閉源 | N/A | ✅中階可15個並行task | ⚠️雲VM隔離非worktree | ❌ | ⚠️task/PR清單 | ❌人工審PR | ❌人工merge | ❓未查證是否跑專案自己CI | ❓未查證是否有手機App(僅知CLI+web) | ❓ | ❓ | N/A雲端服務 |

**附註（satellite單一用途工具，證明「主流orchestrator沒做」的功能有專門小工具補位）：**
- usage-limit卡住偵測+自動resume：`unsnooze`、`stablyai/orca` PR #21036/#8132、`codeman`、claude-auto-retry類工具 — 全部是**獨立**於上述17個orchestrator之外的補丁型專案。
- git危險指令攔截/沙盒：`kintsugi`、`shellter`、`destructive_command_guard`、`agent-shell-gate`、`git-safety-guard`、`CC Safety Net` — 同樣是**獨立**小工具，沒有一個被整合進上述任一主流orchestrator核心。

---

## 歸納

### 已是標配（多數工具都有）
1. 多agent「並行執行」— 17個裡16個都支援（Claude Code官方subagent預設單一lead委派，Agent Teams才有並行）。
2. 某種形式的隔離（git worktree為主，雲端則用VM/container）— 本機工具幾乎全部靠git worktree；雲端工具靠沙盒複本。
3. 人工diff review + 人工按鈕merge — 幾乎所有工具的終點都停在「產出PR/diff讓人看」，這是目前業界共識做法（Cursor官方甚至明文建議**不要**自動merge）。

### 很少人做
1. **Agent之間直接通訊**：只有herdr(socket API)和Claude Code官方Agent Teams(mailbox peer-to-peer，但侷限單一session/單機、關掉session即消失)做到；其餘15個工具agent之間互不知道彼此存在。
2. **手機遠端＋雙向操作(不只是看)**：Omnara、happy/happy-coder、agent-of-empires(PWA)是僅有的三個把手機/遠端當一等公民設計的專案，且都是近6–12個月才竄起（happy 23.8k★、Omnara 2.9k★成長中）。
3. **daemon常駐+跨reboot存活**：目前只有herdr做到「daemon背景常駐、reboot後session還在、任何終端可重連」；其他工具多半是tmux session或桌面App，daemon「升級不中斷agent」這件事**沒有任何工具明確宣稱做到**（未查證＝目前市場空白）。

### 沒人做好／空白
1. **Agent互審＋綁head SHA的自動merge門檻**：業界共識反而是「不要讓agent自己批准自己merge」——WorkOS的官方部落格明確建議把review權限與merge權限分離、merge時要帶reviewed commit SHA防止stale approval；Factory.ai官方文件明講「resist the temptation to auto-merge based on review droid approval alone」；Cursor官方也堅持人工review。GitHub Copilot 2026-09-01才剛開放「review bot可opt-in自行批准PR」，但仍是review agent≠coding agent的分權設計，且預設關閉。**目前沒有任何調查到的工具把「另一個agent審核並綁定head SHA、通過後daemon自動merge」做成開箱即用的預設流水線**——這正是agend v2目前定位最独特、也最反業界慣性的一塊，需要想清楚安全論述（因為主流意見對全自動merge抱持謹慎/反對態度）。
2. **git操作防護內建於orchestrator本體**：目前是完全獨立的小工具生態(kintsugi/shellter/dcg/agent-shell-gate)，沒有一個被整合進任何一個17個orchestrator裡。
3. **同時把「task board派工＋worktree＋CI gate＋agent互審＋自動merge＋agent通訊＋手機遙控＋daemon常駐＋git防護」全部串成一條管線**的工具：每個現有工具只解1–3塊，沒有人做「全端自主開發團隊」的完整閉環。

## 使用者社群最常抱怨的前5名（依證據強度排序）

1. **平行agent互踩造成merge衝突，人工還是要收尾**：一份針對33,596個PR的分析顯示，57.6%的衝突是「兩個agent改到同一檔案重疊行」；worktree只解決執行期隔離，合併期的判斷仍落在人身上。[codex.danielvaughan.com/2026/07/28]
2. **協調負擔轉嫁給人，「自主」名不副實**：有實測案例5個平行agent做零衝突架構的任務，仍耗掉人類8小時做「偽裝成自主」的協調工作；人變成瓶頸不是因為慢，而是只有人能同時記住5條並行工作線的狀態。[engineeredintelligence.substack.com]
3. **Token/成本非線性暴增**：多agent pipeline的成本是「複合」而非線性成長，orchestrator的context會被下游agent輸出不斷灌爆(context rot)，10輪下來可能累積6萬+ tokens。
4. **這個賽道專案陣亡率高，選型有風險**：vibe-kanban公司2026-04關門(專案轉社群維護)、Terragon 2026-01整個關站、Crystal 2026-02deprecated轉Nimbalyst、claude-squad被使用者在issue #250質疑「專案看起來已無人維護」——短短半年多內好幾個知名工具下市或改名，使用者選型時明顯擔心「養套殺」或棄坑風險。
5. **Usage-limit卡住後沒人管**：這問題嚴重到催生一整群獨立補丁工具(unsnooze/codeman/orca/claude-auto-retry)，代表主流orchestrator沒有原生解決，使用者得自己另外裝東西補洞。

## 來源清單（節錄，完整URL見上表內文引用）
- gh api（GitHub官方API即時查詢，2026-09-24執行）：smtg-ai/claude-squad, stravu/crystal, devflowinc/uzi, imbue-ai/sculptor, terragon-labs/terragon-oss, kbwo/ccmanager, agent-of-empires/agent-of-empires, BloopAI/vibe-kanban, omnara-ai/omnara, slopus/happy, slopus/happy-cli, herdrdev/herdr(ogulcancelik/herdr), nimbalyst/nimbalyst
- https://github.com/herdrdev/herdr, https://www.opentechhub.io/herdr/, https://www.bitdoze.com/herdr-agent-multiplexer/
- https://github.com/smtg-ai/claude-squad, https://github.com/smtg-ai/claude-squad/issues/250, https://github.com/smtg-ai/claude-squad/issues/245
- https://www.conductor.build/docs/guides/parallel-agents/run-multiple-claude-code-sessions, https://rywalker.com/research/conductor
- https://github.com/BloopAI/vibe-kanban, https://ai.dosa.dev/tools/vibe-kanban, https://github.com/nagisasaka/easy-vibe-kanban
- https://github.com/stravu/crystal, https://nimbalyst.com/crystal/, https://github.com/nimbalyst/nimbalyst
- https://github.com/devflowinc/uzi, https://www.vibesparking.com/en/blog/ai/claude-code/uzi/2025-08-23-uzi-parallel-ai-coders-git-worktrees-tmux/
- https://imbue.com/product/sculptor, https://github.com/imbue-ai/sculptor
- https://github.com/terragon-labs/terragon-oss
- https://www.omnara.com/, https://github.com/omnara-ai/omnara
- https://github.com/slopus/happy, https://github.com/slopus/happy-cli
- https://www.tembo.io/blog/claude-code-multi-agent-orchestration, https://joseparreogarcia.substack.com/p/claude-code-agent-teams
- https://developers.openai.com/codex/cloud, https://developertoolkit.ai/en/codex/automate/multi-agent-workflows/
- https://madewithlove.com/blog/using-cursor-background-agents/, https://webdeveloper.com/news/cursor-origin-git-forge-parallel-agents/
- https://docs.github.com/en/copilot/responsible-use/agents, https://prlens.dev/guides/how-to-review-copilot-coding-agent-pull-requests, https://www.developersdigest.tech/blog/agent-pr-governance-github-copilot-review
- https://jules.google/, https://developers.googleblog.com/en/meet-jules-tools-a-command-line-companion-for-googles-async-coding-agent/
- https://github.com/agent-of-empires/agent-of-empires, http://www.agent-of-empires.com/
- https://codex.danielvaughan.com/2026/07/28/agent-pr-merge-conflicts-concurrent-coding-agents-codex-cli-worktree-isolation-coordination-defence/
- https://engineeredintelligence.substack.com/p/orchestrate-to-survive-the-speed
- https://gijs.substack.com/p/running-multiple-ai-agents-in-parallel
- https://workos.com/blog/ai-coding-agent-github-merge-policies
- https://factory.ai/, https://sidbharath.com/blog/factory-ai-guide/
- https://github.com/saaranshM/unsnooze, https://github.com/julian3xl/codeman, https://github.com/stablyai/orca/pull/21036
- https://github.com/arrowassassin/kintsugi, https://github.com/walangstudio/shellter, https://github.com/Dicklesworthstone/destructive_command_guard, https://github.com/Spundu/agent-shell-gate
