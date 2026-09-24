# agend-terminal git 歷史分析：bug 熱區與功能演變

> 歸檔註：設計階段的原始紀錄，內容原樣保存（2026-09-24）。只把絕對路徑換成佔位符：`<scratchpad>` = 當時的暫存目錄、`<v1-repo>` = agend-terminal repo、`<tmp>` = 系統暫存目錄、`~` = 使用者家目錄。文中提到的腳本、log、schema dump 沒有一起歸檔。

分析範圍：`main` 分支，2026-04-09 ~ 2026-09-23，非 merge commit 3215 個，
排除 `build(deps)` dependabot commit 62 個後剩 3153 個。全程唯讀（僅
`git log`/`git show`/`grep`/`sed` 讀取，未 checkout、未改檔）。

**方法論限制（先講清楚，避免誤讀數字）**：
- module 歸屬規則：`src/<sub>/...` 記為 `src/<sub>`；`src/<file>.rs`（無同名子目錄）
  記為它自己（例如 `src/daemon.rs` 與 `src/daemon/` 是兩個不同 bucket）。
  這會把同一邏輯子系統（例如 worktree：`worktree.rs`+`worktree_pool.rs`+
  `worktree_pool/`+`worktree_cleanup.rs`+`worktree_cleanup/`+`binding.rs`+
  `binding/`）拆成 7 個 bucket，報告內有給合併數字。
- fix 判定：commit subject 符合正則 `(^fix\(|\bfix\b|\bfixe[sd]\b|\bfixing\b|
  \bbug\b|\bregression\b|\brevert\b|\bhotfix\b)`（大小寫不敏感，`\b` 排除
  `prefix`/`suffix` 誤判）。抽樣核對見下方，未發現誤判。
- 「近 30/60 天」以今日 2026-09-24 為基準（30 天 = ≥2026-08-25，60 天 =
  ≥2026-07-26）。
- **文件/commit 訊息只是線索**：所有「仍在出現」「已移除」的結論都用
  `grep`/`Read` 對照當下 HEAD 原始碼重新核對，並附 file:line；未核對到的
  一律標「未驗證」。

可重跑指令都列在對應章節，原始資料留在 scratchpad：
`log_with_files.txt`（3215 commit 的 `--name-only` 全量 dump）、
`parse_log.py` / `parse_last60.py`（分類腳本）、`module_totals_all.tsv`、
`module_fix_commits.json`、`last60_commits.json`、`invariant_doc_comments.txt`。

---

## 1. 各模組 fix-commit 排行（Top 20）

```bash
git log --no-merges --name-only --format='@@COMMIT@@%H@@%ad@@%s' --date=short \
  > log_with_files.txt
python3 parse_log.py   # 見 scratchpad/parse_log.py，含 fix 正則與 module_of()
```

| module | fix | total | fix ratio |
|---|---:|---:|---:|
| src/daemon | 429 | 857 | 50.1% |
| src/mcp | 343 | 685 | 50.1% |
| tests/ | 170 | 407 | 41.8% |
| src/app | 151 | 309 | 48.9% |
| src/api | 147 | 295 | 49.8% |
| src/agent | 91 | 150 | 60.7% |
| src/tasks | 76 | 124 | 61.3% |
| src/inbox | 67 | 110 | 60.9% |
| src/main.rs | 64 | 234 | 27.4% |
| docs/ | 62 | 421 | 14.7% |
| src/channel | 61 | 160 | 38.1% |
| src/backend.rs | 60 | 114 | 52.6% |
| src/agent.rs | 56 | 109 | 51.4% |
| src/worktree_pool.rs | 53 | 94 | 56.4% |
| src/state | 48 | 89 | 53.9% |
| src/bootstrap | 45 | 121 | 37.2% |
| src/render | 41 | 77 | 53.3% |
| src/state.rs | 41 | 87 | 47.1% |
| src/daemon.rs | 41 | 82 | 50.0% |
| src/inbox.rs | 37 | 88 | 42.1% |

**合併後的 worktree/binding 生態系**（`worktree.rs`+`worktree_pool.rs`+
`worktree_pool/`+`worktree_cleanup.rs`+`worktree_cleanup/`+`binding.rs`+
`binding/`+`worktree/`+`git_worktree.rs`）：fix=212 / total=356 / **59.6%**，
若視為單一子系統可排到第 3 名（次於 daemon、mcp，高於 app）。

抽樣核對（`module_fix_commits.json`）：`src/daemon` 樣本 8 筆全部是真的
bug-fix PR（例：`247cc0f4 fix: fence manual sweep by repository scope (#3584)`,
`a2eeefa8 fix: prune dead TUI broadcast subscribers on liveness, not PTY
output (#3682)`）；`src/mcp` 同樣（`803bbcd1 Fix sweep owner admission
fail-closed (#3703)`）。分類正則未見明顯誤判。

**解讀**：`daemon`／`mcp`（daemon 對外的 RPC 介面）是絕對熱區，兩者合計
fix commit 佔全庫 fix 總數約 4 分之 1。`agent`/`tasks`/`inbox` 雖然總量較小，
但 fix ratio 超過 60%，代表「這個模組一旦被碰就有六成機率是在補洞」，
是重寫時要優先重新設計而非直接搬的候選。`docs/` fix ratio 最低（14.7%）
符合預期（文件變更多半是新增/調整，不是「修 bug」）。

---

## 2. 反覆出現的 bug 類別（同類 ≥3 次修復）

```bash
git log --no-merges --format='%H|%ad|%s' --date=short \
  | grep -v '^[^|]*|[^|]*|build(deps)' > all_nondep.tsv
grep -Ei "<keyword>" all_nondep.tsv        # 逐類關鍵字
awk -F'|' '$2>="2026-08-25"' all_nondep.tsv > last30.tsv   # 近 30 天子集
```

| 類別 | 修復次數 | 代表 commit | 近 30 天是否仍在修 | HEAD 現況核對（已驗證） |
|---|---:|---|---|---|
| **worktree 清理／orphan／leak** | ≥19（含 leak 子類另計 13 筆，多有重疊） | `1dfe971e` fix(worktree-leak): unified release invariant (#1805, 06-05); `09e45de2` cleanup_working_dir teardown worktree-aware (#2259, 06-16); `3c23f210` fix(worktree_cleanup) (#2900, 07-21) | **是**：`a9c9880e`(09-01) `540e736f`(09-02) `058f2787`(09-11) `373e942f`(09-15) `5755542d`+`461cf053`(09-20) | **仍在演變，非已解決**。`src/worktree_pool.rs:685` 現有 `TODO(W1.2): audit whether the empty-source_repo branch is still …`；worktree 生態系當前 LOC：`worktree.rs`+`worktree_cleanup.rs`+`worktree_pool.rs`=5881 行，加上 `worktree_pool/`+`worktree_cleanup/`+`binding/` 目錄共 18161 行——這是全庫唯一「近 30 天仍持續產生新 fix」且規模巨大的類別，最值得在重寫時重新設計（而非移植）。 |
| **state/PTY 輸出誤判（misclassification，非僅 #919 red-anchor）** | 7（#919 red-anchor 家族）+ 近60天另有 9 筆新誤判修復 | `25bb9859` fix(state): cut stall-detection false positives (#122, 04-23); `b73b0318` feat(#919 Phase A): red-SGR anchor (05 中); `ef6fe71c` fix(#1757) exempt hard network errors (#1765) | **#919 red-anchor 機制本身**近 30/60 天無新 fix；但**同類「新 banner/新版本誤判」持續發生**：`c02de096`(08-19) fix(state) 誤判 dev-channel modal 為 permission prompt；`2fb635be`(09-07) fix(state) usage-limit banner 誤判；`027d0936`(07-31) fix(grok) 重新校準 0.2.117 state anchors | 已驗證：`src/backend.rs:47` 函式已從文件提到的 `has_red_anchor()` 改名為 `should_anchor_on_red()`（`grep has_red_anchor` 全庫 0 筆命中，證明命名文件已過時，須以程式碼為準）；HIGH_FP anchor 機制現況見 `src/state/patterns.rs`、`src/vterm.rs:250`。**結論：紅色 anchor 這個「機制」穩定，但「每個 CLI 版本升級就要重新校準」這個根本問題沒解決**——本質是靠螢幕文字/顏色猜狀態，換一版 CLI banner 就要修一次，重寫時應考慮結構性方案（如結構化 hook 事件，`should_anchor_on_red` 註解已提到 claude/agy 有 hook 可用，但 Shell/Raw/其餘仍全靠螢幕猜測）。 |
| **block_on 巢狀 runtime panic** | 6 | `92f1d4dd`(04-22) `c3b1733f`(04-26 PR-AK URGENT) `b4a15ff3`/`e849a273`/`48affb04`(05-30 telegram+discord 同一 bug 複製貼上) `c3a59f9d`(06-02 dedup 成 `block_on_value`) | **否**：近 30/60 天 0 筆 | 已驗證修復落地且**結構性防退化**：`src/channel/telegram/state.rs:32-46` 的 `spawn_or_block_on` 用 `Handle::try_current()` 守衛，`:56-62` `block_on_value` 委派給 `src/channel/shared_async.rs:30` 共用實作；`src/channel/discord/adapter.rs:202` 同樣。`tests/block_on_runtime_guard_invariant.rs` 仍在（CLAUDE.md 命名的 hard rule）。**這類已經是「用測試+共用 helper 鎖死」的成功案例，可直接搬。** |
| **deadlock（self-IPC / lock-across-flock）** | 8 | `a612f5f4`(04-09) wait_exit_code 持鎖阻塞; `037ffaef`(05-12) PTY write timeout; `57492760`/`6f1403d2`(05-30, 06-02) registry-lock 相關; `4812adc8`(06-02) 擴展到 fs4 flock 層 | **否**：近 30/60 天 0 筆 | 已驗證：`src/sync_audit.rs:171-218` `FLOCK_DEPTH`/`CoreMutex` 仍是唯一鎖追蹤點；結構性守衛 `tests/core_mutex_invariant.rs`、`tests/flock_depth_invariant.rs`、`tests/self_ipc_guard_always_on_invariant_1492.rs` 均存在且描述「always-on、不可再 `#[cfg(debug_assertions)]` 化」。屬已收斂類別。 |
| **zombie process** | 5 | `1231d3ab`(04-20) supervisor reap zombies; `4b102f39`(05-19) boot-time zombie sweep #933; `1815a8db`(06-15) TOCTOU PID 重用 fix | **否**：近 30/60 天 0 筆 | 已驗證：`src/admin/cleanup_zombies.rs`、`src/daemon/boot_sweep.rs` 現存於 HEAD，機制仍在。屬已收斂類別。 |
| **flaky test（硬 sleep→poll）** | 7 | `d2cefef6`(05-13) `ccaec106`(05-13) poll 取代 hard sleep; `4945dbf0`(05-19) pid_file race; `10a94cd2`(06-15) flaky→deterministic | **否**：近 30/60 天 0 筆 | 未逐一重新跑測試驗證「零 flaky」，僅確認關鍵字近 60 天無新 fix commit（**此點標未驗證到「測試層面」，只驗證到「commit 訊息層面」**）。 |
| **duplicate/dedup 相關** | 66（樣本檢視為混合多主題：catalog OrderKey 重複、tab 重複、通知重複去重，非單一根因） | `6a74c28d`(09-23) Fix duplicate catalog OrderKey under concurrent task mutations (#3717) | 是（見上，09-23 仍有新的一筆） | 未細分逐一驗證每個子案；`6a74c28d` 是本次分析當下最新一筆非 dependabot commit，顯示**併發下的重複/排序問題到今天都還在修**，值得列為觀察但未做逐案 code-verify（時間所限，標「部分驗證」）。 |

**未列入「反覆類別」但抓到 0 筆的關鍵字**：`race condition`（0，需用 `\brace\b`
才抓到 34 筆，多為 test fixture race 而非產品邏輯 race，未逐一分類）、
`hot.restart`（0，`restart` 本身 55 筆但多為功能性 commit 非同一 bug 類）。

---

## 3. 加入又移除/revert/deprecate 的功能（功能擺盪）

```bash
grep -Ei '^\S+\|\S+\|(revert)' all_nondep.tsv
git log --oneline --all --grep="<feature>" -i
```

全庫 `Revert "..."` / `revert:` commit 共 17 筆。三個經 HEAD 驗證、影響較大的
案例：

1. **rate-limit 自動恢復提示（`process_rate_limit_recovery_nudges`）**——
   `#841` 引入 → `#846` hotfix → `#849`(05-16, operator-authorized) revert →
   `#886` 重新引入 → 再 revert → 再 revert-of-revert → 再 revert，三層巢狀
   `Revert "Revert "Revert ...""` commit（`0f4d63e2`/`d3c7273d`/`fb8ad4d1`
   等，05-18）。**已驗證現況**：`grep -rn process_rate_limit_recovery_nudges
   src/ tests/` 只在 `tests/no_rate_limit_recovery_nudge_invariant.rs:1,15,
   36,43` 出現，production `src/` 內 0 筆命中——**功能已被結構性測試鎖死為
   「必須維持未接線狀態」，屬永久移除**。分類器一改就震盪三次以上，是
   典型「需求本身不穩定，靠 revert 拉扯」的設計信號。

2. **Backend::Gemini 退役**——`25703f63`(06-03) 引入退役 → 同日
   `4c9770b0` revert（恢復 Gemini）→ `3a3fbfba`(06-11) 最終正式退役
   （`feat(#1580): retire Backend::Gemini — completes #8`）。**已驗證現況**：
   `src/backend.rs:10-22` 目前 `enum Backend` 無 `Gemini` 變體，只剩註解
   `// #1580: Gemini retired (gemini-cli sunset 2026-06-18)`，繼任者
   `Agy`。確認為真的永久移除（非文件過時）。

3. **App 永遠 Attached + 自動 spawn detached daemon（#879 系列）**——
   4 次嘗試全部被 revert：`8b5d7db6`(#881)→`470c251b` revert；
   `0fd89e80`(#882)→`720c38c2` revert；`55e32e51`(#903 v3)→`fe528c1` revert；
   最終 `1583370e`(#906, v4) 換了完全不同的根本手法解決——不做「app 永遠
   attach」的框架，而是讓 `daemon::run_core` 的 `.port` 發布與 agent 生成
   順序做同步化（`871f73ec` #908 佐證）。**已驗證現況**：
   `grep -rln "AlwaysAttached\|auto_spawn_detached" src/` 0 筆命中，
   `tests/attached_path_mcp_invariants.rs` 開頭明確記載「#879v4 RED tests
   — pin the two pre-existing bugs unmasked by PR #903 (then reverted via
   fe528c1)」。**這是最強的「同一個架構方向試了 4 次都失敗，換手法才成功」
   案例，重寫時應直接放棄「app-daemon 誰擁有誰」這個框架，改用它最終落地
   的「同步發布 + 有序 spawn」模型。**

4. **Sprint 49「channel discipline」（router 層自動注入提醒）**——
   `b18da141`(#424) 引入 → `eeb86108`(#425, 05-04) revert，理由明講
   「daemon deadlock + design issues」。**已驗證現況**：
   `grep -rn "channel.discipline" src/` 只剩 `src/instructions.rs:304,309`
   的文件字串「Response channel discipline」，語意是「依輸入來源選回覆
   管道」的說明文字，**不是**當年 router 層自動注入的實作。Sprint 52
   有規劃文件（`569715a5`）但未見對應 feat commit 落地，判斷為**規劃後
   未重新實作**（此點只查了 commit 訊息與現況程式碼，未逐一排除是否藏在
   其他檔名下，標「大致驗證，非窮盡」）。

---

## 4. 近 60 天（2026-07-26 起）開發心力分布

```bash
python3 parse_last60.py   # 見 scratchpad/parse_last60.py
```

非 dependabot commit 共 408 筆。

**按 conventional-commit 類型**：

| 類型 | 筆數 | 佔比 |
|---|---:|---:|
| fix | 276 | 67.6% |
| test | 44 | 10.8% |
| feat | 40 | 9.8% |
| docs | 17 | 4.2% |
| other（無法辨識 prefix） | 17 | 4.2% |
| ci | 4 | 1.0% |
| perf | 4 | 1.0% |
| refactor | 2 | 0.5% |
| chore | 2 | 0.5% |
| revert | 1 | 0.2% |
| build | 1 | 0.2% |

**治理機制類**（commit 訊息含 protocol/decision/governance/RCA/Sprint/invariant）
僅 6 筆（`e75a392c`(09-12) decision supersedes；`d4da139c`(09-09) decisions
fix；`ed19c9ce`(09-08) decision projections；`8acf4ece`(09-04) harness
invariant test；`5687a1a6`(08-19) protocol identity parity；`bad70621`
(08-15) decisions batch archival）——絕大多數治理相關工作已經被算進上面
fix/test/feat 桶，並未自成大宗。

**按子系統（commit 觸及該 module 至少一次，前 12）**：

| module | 筆數 | 型態分布 |
|---|---:|---|
| src/mcp | 121 | fix=90, feat=14, other=8, test=5 |
| src/daemon | 113 | fix=79, test=12, feat=11, other=7 |
| tests/ | 64 | fix=38, feat=9, other=7, test=6 |
| src/agent | 57 | fix=40, test=8, feat=5 |
| src/app | 48 | fix=35, feat=7 |
| src/tasks | 44 | fix=36, feat=6 |
| src/api | 43 | fix=29, feat=7 |
| src/transport | 42 | fix=30, feat=5, test=4 |
| docs/ | 35 | fix=13, docs=10, feat=9 |
| src/task_events | 31 | feat=19, fix=9 |
| src/main.rs | 22 | fix=12, feat=6 |
| src/agent_ops | 21 | fix=12, feat=4 |

**解讀**：近 60 天實際心力壓倒性花在 **修 daemon/mcp/agent/tasks 的既有
bug**（67.6% 是 fix），新功能開發（feat 9.8%）幾乎全部集中在
`src/task_events`（19 筆 feat，遠高於其他模組）——這是近期唯一還在「長」
的子系統，其餘都在「穩」。這與第 1 節的長期 fix ratio 排行一致：daemon/
mcp 長期都是熱區，近 60 天沒有緩解跡象。

---

## 5. tests/ 下 invariant/guard/audit 測試清單（結構補洞清單）

```bash
ls tests/*invariant*.rs tests/*guard*.rs tests/*audit*.rs
grep -m8 "^//!" tests/<file>.rs   # 逐檔取模組說明
```

tests/ 目錄共 168 個檔案，其中 48 個檔名含 invariant/guard/audit。逐一讀取
`//!` 開頭的模組說明後摘要如下（原文見
`scratchpad/invariant_doc_comments.txt`）；這些是「本該是型別系統/架構保證，
現在只能靠 grep-CI 補」的清單，重寫時應優先評估能否改用型別/所有權消除：

| 測試檔 | 防守的 bug 類別（一句話） |
|---|---|
| `agent_path_colon_split_invariant_1504.rs` | PATH 用硬編 `:` split 在 Windows 上炸掉，導致 git shim 遞迴 spawn 風暴（#1504） |
| `agentic_git_bin_target_invariant.rs` | agentic-git shim 沒有正確被聲明為 `[[bin]]`，導致 `cargo build` 不再產出它 |
| `anti_pattern_invariant.rs` | 測試函式與 production 函式同名，重構掉 production 函式卻不會讓測試變紅（假綠測試） |
| `atomic_write_invariant.rs` | 用非原子 `fs::write` 寫狀態檔，並發讀者讀到半寫壞的檔案 |
| `attached_path_mcp_invariants.rs` | always-Attached 模式移除了原本遮住的 daemon 端啟動順序 race（#879v4） |
| `audit_append_single_sink_invariant.rs` | 多個 sink 各自 unbuffered append 同一份 audit log，並發下互相交錯寫壞 |
| `audit_workflow_invariant.rs` | CI workflow 裡的 `audit`/`coverage` job 結構被意外改掉（pin CI YAML 形狀） |
| `block_on_runtime_guard_invariant.rs` | 在 tokio runtime 內對同一 shared runtime 呼叫 `block_on` 導致 panic（#1474/#1476） |
| `cargo_include_invariant.rs` | `include_str!`/`include_bytes!` 指到的路徑沒列進 `Cargo.toml` 的 `include`，`cargo publish` 建置失敗 |
| `ci_trigger_invariant.rs` | CI workflow 意外少了 `workflow_dispatch` 手動觸發 |
| `core_mutex_invariant.rs` | 出現裸 `Mutex<AgentCore>`，繞過 `CoreMutex` 的鎖深度追蹤，重開 self-IPC 持鎖死鎖破口（#1492/#1535） |
| `coverage_workflow_invariant.rs` | CI coverage job 結構被意外改掉 |
| `daemon_boot_gate_invariant.rs` | 新增會啟動真daemon的測試卻沒被列進 boot-flake-gate 清單，逃過每次 20 次重跑的 race 偵測 |
| `daemon_git_helper_invariant.rs` | daemon 端操作 git 忘記繞過 `agend-git` shim，污染 fleet 管理的 repo（#821/#1463） |
| `dev_modal_dismiss_reachability_invariant.rs` | dev-channel 啟動 modal 卡住，且沒有任何 transport 模式能把它清掉 |
| `docs_bilingual_invariant.rs` | 文件資訊架構（雙語、扁平放置）被意外破壞 |
| `enqueue_drop_invariant.rs` | inbox/notification 的 `enqueue` 回傳值被 `let _ =` 吃掉，訊息靜默丟失（#1614/#1618/#1622） |
| `env_isolation_invariant.rs` | `cmd.env_clear()` 沒排在 `cmd.env(...)` 之前，環境變數隔離被靜默破壞（#1440） |
| `env_mutation_serialization_invariant.rs` | 測試間全域環境變數互相污染，在同進程 test runner 下產生競態 |
| `file_size_invariant.rs` | `src/mcp/handlers` 底下檔案重新長回巨石檔案（>750 LOC） |
| `flock_depth_invariant.rs` | 出現不受追蹤的裸 fs4 flock，繞過 `FLOCK_DEPTH`，重開 flock-while-blocking 死鎖破口（#1617/#1342/#1340/#1624） |
| `git_subprocess_invariant.rs` | 測試直接 `Command::new("git")` 沒繞過 shim，污染 host repo 的 `.git`（#821） |
| `git_test_bypass_invariant.rs` | 測試對 scratch repo 做 mutating git 操作沒設 `AGEND_GIT_BYPASS=1`，被 shim 誤導到 bound worktree（#1463） |
| `health_blocked_reason_no_self_ipc_invariant_2454.rs` | health 的 blocked-reason 寫入 handler 又繞回 self-IPC loopback（#2454） |
| `heartbeat_pair_atomicity_audit.rs` | heartbeat pair 鎖定順序被破壞（F6 lock-around-pair） |
| `instrument_never_blocks_invariant.rs` | audit/instrument 的側寫程式碼意外影響到被觀察操作的控制流或 exit code |
| `issue_548_phase2_invariants.rs` | #548 Phase2 實作偏離其 Phase1 RCA 定案的行為契約 |
| `mcp_retired_arg_keys_invariant.rs` | MCP 工具參數改名後，某個 handler（常是共用 helper）仍讀舊參數名，靜默拿到 `null`（schema/handler drift） |
| `metadata_resolver_invariant.rs` | 手刻 metadata 路徑繞過統一 resolver，讀寫到過時的 legacy 檔名（#1680/#1682） |
| `model_flag_chokepoint_invariant.rs` | 多個 spawn 路徑各自手刻 `--model` 參數組裝，繞過統一能力閘（重複旗標/wrapper 誤判） |
| `no_local_mcp_mode_invariant.rs` | release 包漏帶 `agend-mcp-bridge`，退回到會炸 Windows daemon 的舊路徑（#531） |
| `no_per_task_json_probe_invariant.rs` | 探測不存在的 per-task JSON 檔（task board 已是 event-sourced，此檔案永遠不存在）（#1608b/#1614） |
| `no_rate_limit_recovery_nudge_invariant.rs` | rate-limit 自動恢復提示功能被 pin 死在「已 revert、未接線」狀態（見第 3 節案例 1） |
| `notify_undriven_runtime_invariant_channel.rs` | 對 `current_thread` runtime `spawn` 後丟棄 `JoinHandle`，任務永遠不會被推進（telegram 通知靜默不發） |
| `now_field_header_invariant_1509.rs` | 某個 `[AGEND-MSG` header builder 漏加 `now=` 時區欄位（#1487/#1509） |
| `prepush_hook_invariant.rs` | pre-push hook 的 CI-parity 檢查被靜默移除或跟 CI 定義漂移（#1734/#1735） |
| `provisioning_home_isolation_invariant.rs` | provisioning API 偷偷讀環境變數決定 home，而非要求呼叫端顯式傳入 |
| `ready_marker_invariants.rs` | `.ready` marker 存在，但 agent spawn loop 其實還沒跑完（#922） |
| `self_ipc_guard_always_on_invariant_1492.rs` | self-IPC 持鎖死鎖守衛只在 debug build 生效，release build 完全沒保護（#1492/#1535） |
| `snapshot_failopen_invariant.rs` | `snapshot.json` 的讀者沒有 fail-open，過期/缺失快照造成不可逆動作 |
| `spawn_rationale_audit.rs` | 新的 `thread::spawn`/`tokio::spawn` 站點沒有標註存活理由或 join 責任 |
| `src_file_size_invariant.rs` | production 檔案重新長成 4k-6k 行巨石檔（2026-06 拆過的 worktree_pool/supervisor/task_events 等） |
| `state_current_mutation_invariant.rs` | `StateTracker::current` 被繞過 `record_set` 直接賦值，狀態轉換漏記進 `state-transitions.jsonl`（#1527） |
| `stringly_classification_invariant.rs` | 用 `.contains("...")` 字串子串判斷錯誤類型，取代型別化欄位，訊息措辭一變就誤判（#1024/#1833） |
| `task_events_invariant.rs` | `task_events` 模組外的程式碼直接引用 `task_events.jsonl` 檔名，繞過統一 append 介面 |
| `tty_leak_invariant.rs` | app 模式下子行程繼承了 TUI 的控制終端（TTY 洩漏）（#2071） |
| `tui_reconnect_no_respawn_invariant.rs` | DISCONNECTED 復原路徑意外呼叫到會重新 spawn instance 的函式 |
| `typed_review_receipt_invariant.rs` | code-review 攝取邊界長出第二條未受控的 PR-effect 路徑（#2760） |
| `windows_native_symlink_failclosed_invariant.rs` | Windows 原生 symlink 权限不足時沒有 fail-closed |
| `windows_symlink_canary_invariant.rs` | CI Windows job 的 symlink canary 檢查步驟被意外拿掉 |
| `write_actor_platform_shim_invariant.rs` | `write_actor`（持有 unix-only 裸 PTY fd）被非 `#[cfg(unix)]` 的呼叫路徑引用，Windows build 失敗（#3315） |

**觀察**：這 48 個測試裡，至少 15 個是「同一種根因换了個 issue 號碼再犯一次」
才補的（`#1617`→`#1624`→`#1629` 死鎖鏈；`#1614`→`#1618`→`#1622` enqueue
丟失鏈等），代表現有設計把大量本可用型別系統消除的不變量
（單一寫入路徑、鎖深度追蹤、原子寫、no-bare-mutex）放到測試層補洞。重寫時
的具體建議：
- `CoreMutex`/`FLOCK_DEPTH` 的「唯一鎖入口」規則 → 用 newtype + 私有欄位
  在編譯期禁止裸 `Mutex<AgentCore>` 出現，而非用 AST 掃描抓。
- `task_events`/`audit_append_single_sink` 的「唯一寫入路徑」規則 →
  用 crate-private writer + `pub` 只暴露 append 函式，物理上做不到繞過。
- `stringly_classification`（字串 contains 判型別）→ 從一開始就用 enum/
  typed error 而非事後補測試禁止 `.contains`。
- worktree/binding 的一大坨 review_* 測試（`review_worktree_git_3/4/5/8`、
  `review_mcp_ci_worktree_2/3`）顯示這個子系統連「哪裡該加鎖、哪裡該
  fail-closed」都要靠事後 code review 逐條 pin 測試，是本次分析中最強烈
  建議重新設計（而非搬）的子系統。
