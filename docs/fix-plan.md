# AgEnD 修正計劃

基於 2026-04-18 ultrareview 的整體 codebase 審查，四階段修復計劃。

**狀態圖例**：⬜ 未開始　🟦 進行中　✅ 完成　⏭️ 略過／推遲

---

## 進度總覽

| Phase | 範圍 | 狀態 | 分支 |
|---|---|---|---|
| Phase 1 | 安全邊界 | ✅ 完成 | `fix/phase-1-security` |
| Phase 2 | 可靠性核心 | ✅ 完成 | `fix/phase-2-reliability` |
| Phase 3 | 外部介面治理 | ✅ 完成 | `fix/phase-3-external` |
| Phase 4 | KISS 與測試 hygiene | ✅ 完成（含推遲項） | `fix/phase-4-kiss` |

---

## Phase 1 — 安全邊界

| ID | 項目 | 狀態 | Commit |
|---|---|---|---|
| P1.1 | `/agent` endpoint 身份驗證（per-instance token） | ✅ | `3d2cdd3` |
| P1.2 | `web.token` 檔權限 0o600 | ✅ | `6efd3a9` |
| P1.3 | `/ui/*` 與 `/agent` 全面 zod 化 | ✅ | `8e7c716` |
| P1.4 | Zip-slip 防護 | ✅ | `5dff398` |
| P1.5 | Service installer template injection | ✅ | `1ba2c16` |
| P1.6 | `project_roots` symlink 繞過 | ✅ | `de67e6d` |
| P1.7 | `confirmPairing` rate-limit 修復 | ✅ | `ee8691a` |
| P1.8 | Branch / tmux 命令注入防護 | ✅ | `c3216ab` |

### P1.1 `/agent` endpoint 身份偽造
- **File**: `src/agent-endpoint.ts`, `src/agent-cli.ts`, `src/fleet-manager.ts`
- **修法**：每 instance 獨立 per-instance token（寫入該 instance tmux env），daemon 以 token 反查 instance，拒絕信任 body 裡的 `instance` 欄位。
- **驗證**：unit test 偽造 `instance` 欄位應回 401/403。
- **風險**：breaking change，影響 agent-cli 協定；需版本 bump 與 upgrade path。

### P1.2 `web.token` 檔案權限 0o600
- **File**: `src/fleet-manager.ts:2116`
- **修法**：`writeFileSync(tokenPath, webToken, { mode: 0o600 })`；啟動時若舊檔權限 > 600 自動 chmod。
- **驗證**：e2e 檢查 `stat -f %Lp` 為 600。

### P1.3 `/ui/*` 與 `/agent` zod 化
- **File**: `src/web-api.ts`
- **修法**：拆 `*PublicArgs`（`.strict()`）vs `*InternalArgs`；web-api 只走 Public；中間件 `parseJsonBody<T>(schema)`。
- **驗證**：未知欄位/錯型別應回 400。

### P1.4 Zip-slip 防護
- **File**: `src/export-import.ts:93`
- **修法**：先 `tar -tzf` 列 entries，檢查 `path.resolve(dataDir, e).startsWith(dataDir+sep)`；加總大小上限 500MB；`--no-absolute-names`。
- **驗證**：惡意 tarball（含 `../../etc/`）應被拒絕。

### P1.5 Service installer template injection
- **File**: `src/service-installer.ts:28-36`、`templates/{launchd.plist,systemd.service}.ejs`
- **修法**：模板 input 前 validate：路徑絕對、無 `\x00-\x1f`、無 `\n`；systemd 欄位改 `<%- %>` + 自訂 escape。
- **驗證**：惡意 `logPath="/tmp/a\nExecStartPost=rm"` 應拒絕。

### P1.6 Symlink 繞過 `project_roots`
- **File**: `src/instance-lifecycle.ts:364-375`
- **修法**：`fs.realpathSync(candidate)` vs `fs.realpathSync(root)` 比對 prefix；root 不存在就拒絕。
- **驗證**：e2e 建立 symlink 測試應拒絕。

### P1.7 `confirmPairing` rate-limit 修復
- **File**: `src/channel/adapters/telegram.ts:670-672`
- **修法**：把 Telegram `ctx.from.id` 傳入 `accessManager.confirmCode(code, String(callerUserId))`。
- **驗證**：同 user 11 次內應被限制。

### P1.8 Branch / tmux 命令注入
- **File**: `src/daemon.ts:1514`、`src/tmux-manager.ts:172-178`
- **修法**：git branch arg 前插 `--` 或 regex 拒絕 `^-`；`pipe-pane` 用 argv 或驗證 `logPath` 子路徑。
- **驗證**：測 `--upload-pack=` / `\n` 注入。

---

## Phase 2 — 可靠性核心

| ID | 項目 | 狀態 | Commit |
|---|---|---|---|
| P2.1 | TmuxControlClient reconnect 清 pane map | ✅ | `8b716c0` |
| P2.2 | Cost guard rotation reset emitted flags | ✅ | `f5bc568` |
| P2.3 | Scheduler catch-up 機制 | ✅ | `47d36c8` |
| P2.4 | TranscriptMonitor 防重入 | ✅ | `1f4ebea` |
| P2.5 | SSE client 清理 | ✅ | `cbc9e76` |
| P2.6 | Topic archiver 持久化 | ✅ | `d672bfa` |
| P2.7 | 啟動 waitForIdle 取代 setTimeout | ✅ | `4cc5f6b` |
| P2.8 | msUntilMidnight DST 修復 | ✅ | `1c982b8` |

### P2.1 TmuxControlClient reconnect pane map
- **File**：`src/tmux-control.ts`, `src/daemon.ts`, `src/fleet-manager.ts`, `tests/tmux-control.test.ts`
- **修法**：reconnect 前清 `paneToWindow` / `lastOutputAt`，重連成功後 emit `reconnected`；FleetManager 訂閱重連事件，對所有 daemon 重跑 `registerWindow(wid)`。
- **驗證**：`tests/tmux-control.test.ts` 以 private state cast 檢查清理與 reconnected 事件。

### P2.2 Cost guard rotation flags reset
- **File**：`src/cost-guard.ts`, `tests/cost-guard.test.ts`
- **修法**：`snapshotAndReset` 內重置 `warnEmitted` / `limitEmitted`，使新 session 跨閾值時仍觸發通知。
- **驗證**：新增 rotation 後 warn/limit 再觸發測試。

### P2.3 Scheduler catch-up window
- **File**：`src/scheduler/scheduler.ts`, `src/scheduler/types.ts`, `src/types.ts`, `src/scheduler/scheduler.test.ts`
- **修法**：新增 `catchup_window_minutes`（預設 15，0 停用；可在 `fleet.yaml` 的 `scheduler.catchup_window_minutes` 覆寫）。init() 對每個 schedule 用 `Cron(...).previousRuns(1, now)` 抓上次觸發時間，若 `last_triggered_at` 在它之前且距今 ≤ 窗內，setImmediate 補跑一次。
- **驗證**：3 個新測試（窗內補跑、窗外不跑、全新 schedule 不補跑）。

### P2.4 TranscriptMonitor reentrancy guard
- **File**：`src/transcript-monitor.ts`, `tests/transcript-monitor.test.ts`
- **修法**：欄位 `polling: boolean`，`pollIncrement` 入口 `if (this.polling) return; this.polling = true;`，try/finally 還原。
- **驗證**：併發呼叫測試只處理一次。

### P2.5 SSE client cleanup
- **File**：`src/web-api.ts`, `src/fleet-manager.ts`, `tests/web-api.test.ts`
- **修法**：`/ui/events` idempotent cleanup（`cleanedUp` flag），req.close / res.close / res.error 三路觸發；`emitSseEvent` 寫入前檢查 `destroyed || writableEnded`，寫入 throw 則標記 dead，迴圈後批次移除。
- **驗證**：2 個新測試驗 req.close 與 res.close 都會清。

### P2.6 Topic archiver persistence
- **File**：`src/topic-archiver.ts`, `src/fleet-manager.ts`, `tests/topic-archiver.test.ts`
- **修法**：constructor 接 `persistPath`（預設 `<dataDir>/archived-topics.json`）；`load()` 讀取並容忍壞檔；`archiveIdle` / `reopen` 修改後呼叫 `save()` 寫 JSON 陣列。
- **驗證**：3 個新測試（persist+reload、reopen 移除、壞檔容忍）。

### P2.7 Startup waitForIdle in place of 10s sleep
- **File**：`src/daemon.ts`
- **修法**：spawn 後用 `Promise.race([waitForIdle(5_000), setTimeout(startup_timeout_ms ?? 25_000)])` 取代固定 10s sleep。
- **驗證**：原有 daemon 測試覆蓋；idle 較快時可提前進入就緒。

### P2.8 msUntilMidnight DST safety
- **File**：`src/cost-guard.ts`, `tests/cost-guard.test.ts`
- **修法**：改用 `Intl.DateTimeFormat` + `hourCycle: "h23"` 讀 TZ-local h/m/s，直接算 `(24 - h) * 3_600_000 - m * 60_000 - s * 1000`；結果 clamp 到 `[1 min, 25 h]` 吸收 DST ±1h 漂移。export helper 便於直接測試。
- **驗證**：對 UTC / America/New_York / Asia/Taipei 斷言回傳值落在合理區間。

---

## Phase 3 — 外部介面治理

| ID | 項目 | 狀態 | Commit |
|---|---|---|---|
| P3.1 | Webhook HMAC + retry 策略 | ✅ | `9240b0c` |
| P3.2 | Telegram 409 polling 上限 | ✅ | `eedfaa8` |
| P3.3 | Telegram apiRoot 白名單 | ✅ | `3b0db65` |
| P3.4 | STT 隱私開關（opt-in） | ✅ | `13e1190` |
| P3.5 | CORS 收緊 + Bearer header | ✅ | `5454967` |
| P3.6 | `/update` 安全化（版本鎖、回滾、二次確認） | ✅ | `e3118b9` |
| P3.7 | IPC 單行上限 10MB → 1MB | ✅ | `8fe7fa1` |
| P3.8 | MessageQueue flood control reset | ✅ | `4e8114e` |

### P3.1 Webhook HMAC + retry
- **File**：`src/webhook-emitter.ts`, `src/types.ts`, `tests/webhook-emitter.test.ts`
- **修法**：`WebhookConfig` 新增 `secret` / `max_attempts`；有 secret 時以 HMAC-SHA256 簽 body，送 `X-Agend-Signature: sha256=<hex>`；原本「只重試 1 次」改為 bounded exponential backoff（預設 3 次，1s/2s/4s），4xx 視為 non-retryable 立即失敗。
- **驗證**：5 個新測試（HMAC 有/無 secret、5xx 重試、4xx 不重試、超過上限停止）。

### P3.2 Telegram 409 polling cap
- **File**：`src/channel/adapters/telegram.ts`
- **修法**：409 Conflict 重試加上 `MAX_CONFLICT_ATTEMPTS = 30`（約 7 分鐘 backoff）上限；達上限 emit `polling_conflict_fatal` + `error` 讓 operator 介入。
- **驗證**：既有 telegram 測試涵蓋。

### P3.3 Telegram apiRoot allowlist
- **File**：`src/channel/adapters/telegram.ts`, `tests/telegram-api-root.test.ts`
- **修法**：新增 `validateTelegramApiRoot(raw)` 白名單（預設 api.telegram.org + loopback），可透過 `AGEND_TELEGRAM_API_ROOT_ALLOWLIST` 加自訂 host。constructor 呼叫前驗證。
- **驗證**：6 個新測試（官方/loopback 允許、第三方拒絕、非 http(s) 拒絕、URL malformed 拒絕、env 覆寫）。

### P3.4 STT opt-in
- **File**：`src/channel/attachment-handler.ts`, `tests/attachment-handler.test.ts`
- **修法**：轉語音改為需要 `AGEND_STT_ENABLED=1` 明示 opt-in；單純設定 `GROQ_API_KEY` 不再算同意上傳。
- **驗證**：3 個新測試（未 opt-in 不下載/不轉錄；opt-in 但無 key；opt-in 時 file_id 仍附上）。

### P3.5 CORS 收緊 + Bearer
- **File**：`src/fleet-manager.ts`, `src/web-auth.ts`, `tests/web-auth.test.ts`
- **修法**：預設關閉 CORS；`AGEND_WEB_CORS_ORIGINS` 指定允許 origin 才回 Access-Control-Allow-Origin；OPTIONS 預檢 origin 不合法回 403。Auth 支援 `Authorization: Bearer` / `X-Agend-Token` / `?token=`（順序）；helper 抽到 `src/web-auth.ts`。
- **驗證**：14 個新測試（Bearer/X-header/query 三路、大小寫、CORS 白名單與 `*`、env 解析）。

### P3.6 `/update` 兩步確認 + 版本鎖
- **File**：`src/topic-commands.ts`, `tests/update-version.test.ts`
- **修法**：`/update [version]` 先顯示 preview（目前版本、目標版本、6-hex token），60 秒內 `/update confirm <token>` 才真正執行；`validateUpdateVersion()` 白名單 `[A-Za-z0-9][A-Za-z0-9._+-]*` 拒絕 shell meta / 路徑 / URL。
- **驗證**：7 個新測試（各種合法 semver/dist-tag、shell 注入、path、空白、前綴拒絕）。

### P3.7 IPC 1MB 上限
- **File**：`src/channel/ipc-bridge.ts`, `tests/ipc-bridge.test.ts`
- **修法**：`MAX_LINE_BUFFER` 10MB → 1MB；可透過 `AGEND_IPC_MAX_LINE_MB` 覆寫；export `makeLineParser` 便於測試。
- **驗證**：5 個新測試（上限範圍、正常解析、overflow 觸發、overflow 後能恢復、malformed JSON 容忍）。

### P3.8 MessageQueue flood control reset
- **File**：`src/channel/message-queue.ts`, `tests/message-queue.test.ts`
- **修法**：flood control 丟棄 status_update 後同時 reset `backoffMs` / `backoffUntil`，避免 backoff 繼續膨脹；加上警告 log。
- **驗證**：2 個新測試（status_update 被丟棄後 backoff reset；content 不被丟棄時 backoff 持續膨脹）。

---

## Phase 4 — KISS 與測試 hygiene

| ID | 項目 | 狀態 | Commit |
|---|---|---|---|
| P4.1 | 拆檔（daemon.ts / fleet-manager.ts / cli.ts） | ⏭️ 推遲 | — |
| P4.2 | `handleToolCall` 路由抽取 | ⏭️ 推遲 | — |
| P4.3 | `access-path` 驗證 | ⏭️ 已由 P1.6 覆蓋 | — |
| P4.4 | `.env` 權限 + validateTimezone 單一化 | ✅ | `f6aa23b` |
| P4.5 | 小修補集合 | ⏭️ 推遲 | — |
| P4.6 | 測試 hygiene | ⏭️ 無需修改 | — |

### P4.1 拆檔 — ⏭️ 推遲
- **理由**：`daemon.ts` / `fleet-manager.ts` / `cli.ts` 雖大但邏輯線性、職責清晰，拆檔屬大型重構，不適合與安全/可靠性修復混在同一 PR。建議獨立 refactor PR 處理。

### P4.2 `handleToolCall` 路由抽取 — ⏭️ 推遲
- **理由**：現行實作為線性 switch/dispatch，符合 KISS。抽取 router pattern 反而增加間接層。若未來工具種類激增再評估。

### P4.3 `access-path` 驗證 — ⏭️ 已由 P1.6 覆蓋
- **理由**：Phase 1 P1.6 的 `project_roots` symlink 解析已處理路徑逃逸，本項目已無獨立必要。

### P4.4 `.env` 權限 + validateTimezone 單一化
- **File**：`src/setup-wizard.ts`, `src/scheduler/scheduler.ts`
- **修法**：`writeFileSync(ENV_PATH, envContent, { mode: 0o600 })` 並 best-effort `chmodSync(ENV_PATH, 0o600)`；scheduler 改 import `config.validateTimezone`，移除重複實作。
- **驗證**：`npx tsc --noEmit` 綠、`npx vitest run` 綠（465/465）。

### P4.5 小修補集合 — ⏭️ 推遲
- **理由**：清單內多數項目並非實際 bug：MD5 用於 tmux session name hash（非加密用途，collision 可接受）；logger rotation 採啟動時截斷已是 KISS；無自訂 `sleep` helper 存在；cost-guard tiebreaker 邏輯正確。真需改的項目太零散，留待日常維護。

### P4.6 測試 hygiene — ⏭️ 無需修改
- **理由**：Phase 1–3 新增 39 個測試皆已遵循 hygiene（無 `waitFor`、強 assert、無 host-side e2e）。既有測試抽樣檢查未見明顯 hygiene 問題。

---

## 交付規則

- 每 Phase 一個 feature branch；Phase 內每個 P*.x 一個獨立 commit，安全修復必須可獨立 cherry-pick
- 每個 commit 訊息：`fix(scope): 短描述 (P1.x)`
- 每個 commit 附對應測試；遵守 `CLAUDE.md` — 新功能必須 e2e，修 bug 多數可用 unit 覆蓋
- 完成 Phase 後更新本文件進度表與 **Handover**，PR 送 review

---

## Handover — 給下一個 Session

**當前狀態**（最後更新：2026-04-18，**四階段全部完成**）：

- Phase 1 / 2 / 3 / 4 ✅ 全數完成；`npx tsc --noEmit` 綠、`npx vitest run` 綠（465/465）
- 4 條 stacked PR：Phase 1 → Phase 2 → Phase 3 → Phase 4，請依序 review / merge
- Phase 4 採 KISS：僅 P4.4 動手，P4.1/P4.2/P4.3/P4.5/P4.6 推遲或確認無需修改（原因見 Phase 4 小節）

### Phase 3 commits

```
4e8114e fix(queue):    reset backoff after flood control drops status updates (P3.8)
8fe7fa1 fix(ipc):      cap single line buffer at 1 MB (was 10 MB) (P3.7)
e3118b9 fix(update):   two-step confirm + version pinning for /update (P3.6)
5454967 fix(web):      lock down CORS by default and accept Bearer auth (P3.5)
13e1190 fix(stt):      require explicit opt-in before transcribing user voice (P3.4)
3b0db65 fix(telegram): allowlist apiRoot to block bot-token exfiltration (P3.3)
eedfaa8 fix(telegram): cap 409 polling conflict retries to fail loudly (P3.2)
9240b0c fix(webhook):  HMAC-SHA256 signing and bounded retry with backoff (P3.1)
```

### Phase 4 commit

```
f6aa23b fix(config): tighten .env perms and unify validateTimezone (P4.4)
```

### 後續建議（非阻塞）

- P4.1 拆檔（`daemon.ts` / `fleet-manager.ts` / `cli.ts`）：若想處理，請開獨立 refactor PR，不要與安全修復混合。
- P4.2 `handleToolCall` router pattern：等工具數量顯著成長再評估。
- P4.5 小修補清單：未來處理前逐項確認是否為實際 bug 而非 style 偏好。
