# Crash Recovery & Instance Replacement — Design

> AgEnD — Context Management 設計
> 日期：2026-04-11（v4 rewrite — context rotation removed, replaced by crash recovery + replace_instance）

---

## 1. 設計演進

### v1（已淘汰）
- 40% 閾值 → 送 `/compact` → 失敗就 fresh start（刪 session-id）
- 問題：閾值太低、/compact 效果有限、脈絡全失

### v2（已淘汰）
- 60% 閾值 → 等 idle → 送 handover prompt → 驗證 `handover.md` → rotate
- 問題：**依賴 Claude 自覺寫交接報告**，本質上脆弱

### v3（已淘汰）
- 80% 閾值 → 5s idle barrier → daemon 收集 snapshot → kill → spawn with snapshot prompt
- 問題：所有 CLI 都有 auto-compact，threshold rotation 跟 auto-compact 衝突

### v4（現行）
- **移除所有主動 rotation**（threshold + max_age）
- Context 管理交給 CLI 內建的 auto-compact（Claude Code、Codex、Gemini CLI、OpenCode、Kiro CLI 都有）
- AgEnD 只負責：crash recovery（health check + respawn + snapshot）和 replace_instance（跨 instance 交接）

---

## 2. v2 → v3 的核心洞察

v2 嘗試讓 Claude 完成一個 meta-task（自我報告狀態），但這個鏈條每一步都脆弱：

| v2 步驟 | 失敗模式 |
|---------|---------|
| 送 handover prompt | Claude 可能忽略 |
| 等 Claude 寫 `handover.md` | Claude 可能寫了但格式錯 |
| 15 秒靜默偵測 = 「完成」 | 可能只是在思考 |
| Markdown section 驗證 | 形式正確 ≠ 內容有用 |
| 重試一次 | 重試後仍失敗就直接 proceed |

**根本問題**：daemon 永遠只是在「猜」Claude 有沒有完成 handover。

**v3 解法**：不猜。daemon 自己收集資訊，自己注入。

---

## 3. v3 設計

### 3.1 狀態機

```
┌──────────────┐
│   NORMAL     │ ◄──── grace 期滿
└──────────────┘
        │
        │ ≥ 80% or max_age_hours
        v
┌──────────────┐
│  RESTARTING  │ ── 5s idle barrier → snapshot → kill → spawn
└──────────────┘
        │
        │ respawn done
        v
┌──────────────┐
│   GRACE      │ ── 10 分鐘冷卻
└──────────────┘
```

從 5 個狀態簡化到 3 個。移除了 PENDING（等 idle）和 HANDING_OVER（等 Claude 寫報告）。

### 3.2 觸發條件

| 條件 | 行為 |
|------|------|
| `context_window.used_percentage >= restart_threshold_pct` | 進入 RESTARTING |
| `max_age_hours` 到期 | 進入 RESTARTING |
| tmux window crash | 走既有 health check respawn（不經過 guardian） |

### 3.3 重啟流程

```
1. guardian emit restart_requested(reason)
2. daemon: waitForTranscriptIdle(5000)    ← 5 秒 best-effort
3. daemon: writeRotationSnapshot(reason)  ← 寫 rotation-state.json
4. daemon: saveSessionId()
5. daemon: killWindow()
6. daemon: spawnClaudeWindow()            ← system prompt 含 snapshot
7. guardian: markRestartComplete()         ← 進入 GRACE
```

### 3.4 Snapshot

Daemon 在重啟前收集本地資訊，寫入 `<instanceDir>/rotation-state.json`：

```json
{
  "instance": "general",
  "reason": "context_full",
  "created_at": "2026-03-30T10:00:00.000Z",
  "session_id": "abc123",
  "context_pct": 82,
  "working_directory": "/path/to/repo",
  "recent_user_messages": [
    { "text": "Check the latest branch-instance spec", "ts": "..." }
  ],
  "recent_events": [
    { "type": "tool_use", "name": "Read", "preview": "docs/spec.md" },
    { "type": "assistant_text", "preview": "I found the spec..." }
  ],
  "recent_tool_activity": [
    "Read docs/spec.md",
    "Edit src/fleet-manager.ts"
  ],
  "last_statusline": {
    "model": "opus",
    "cost_usd": 3.2,
    "five_hour_pct": 71
  }
}
```

Ring buffer 規則：
- `recent_user_messages`：最多 10 筆，每筆截斷 200 字元，過濾 cross-instance 訊息
- `recent_events`：最多 15 筆，preview 截斷 100 字元
- `recent_tool_activity`：最多 10 筆

### 3.5 Prompt 注入

新 session 的 system prompt 結構：

```
1. fleet system prompt          ← 靜態模板
2. user/system-configured prompt ← fleet.yaml 設定
3. previous session snapshot     ← 從 rotation-state.json 產生
```

Snapshot 注入預算：**≤ 2000 字元**。超過則依此優先順序截斷：

1. 永遠保留：reason, context_pct, session_id, working_directory
2. 盡量保留：recent_user_messages
3. 盡量保留：recent_events
4. 最先捨棄：tool activity

---

## 4. 設定

```yaml
context_guardian:
  restart_threshold_pct: 80    # v3 新欄位
  max_age_hours: 8
  grace_period_ms: 600000
  enabled: true                # 選用
```

已淘汰（向後相容但被忽略）：
- `threshold_percentage`（fallback 到 `restart_threshold_pct`）
- `max_idle_wait_ms`
- `completion_timeout_ms`

---

## 5. 觀測性

### 事件

| 事件 | 欄位 |
|------|------|
| `restart_requested` | reason, context_pct |
| `restart_complete` | reason, restart_duration_ms, pre_restart_context_pct, snapshot_user_message_count, snapshot_event_count |

### 每日報告

```
proj-a: $8.20, 2 restarts
proj-b: $2.10
proj-c: $0.00 ⚠️ 1 hang
```

不再追蹤：`handover_status`、`missing_sections`、`word_count`。

---

## 6. 風險與緩解

| 風險 | 緩解 |
|------|------|
| 5 秒 idle barrier 仍是 best-effort | 接受。比 v2 的 5 分鐘 idle 等待 + 60 秒 handover 簡單得多 |
| Snapshot 可能遺漏 in-flight context | 可接受。Claude auto memory (MEMORY.md) 作為補充 |
| 注入 snapshot prompt 占 context budget | 硬限 2000 字元，啟動時僅占 ~0.2% context |
| 連續重啟 | 10 分鐘 grace period 防止循環 |

---

## 7. 成功指標

- [x] 不再出現 `handover_status: empty/timeout/complete`
- [x] 重啟邏輯從日誌就能完整理解
- [x] 狀態機複雜度大幅降低（5 → 3 狀態）
- [x] 新 session 透過 daemon snapshot 獲得有用的延續 context

---

## 8. 相關參考

| 資料 | 重點 |
|------|------|
| [Vincent van Deth](https://vincentvandeth.nl/blog/context-rot-claude-code-automatic-rotation) | 60% 時 handover 品質最高——但 v3 不依賴 Claude 寫 handover，所以可以提高到 80% |
| [JetBrains NeurIPS 2025](https://github.com/JetBrains-Research/the-complexity-trap) | 簡單方法 ≈ 複雜策略 |
| [Anthropic harness](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents) | 狀態外部化 > 壓縮 context |
