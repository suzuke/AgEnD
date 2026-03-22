# Context Rotation v2 — 單閾值設計

> claude-channel-daemon — Context Guardian 重新設計
> 日期：2026-03-22

---

## 1. 現況與問題

```
watchFile (2s interval)
    → 讀 statusline.json
    → used% > 40% ?
        → 送 /compact，等 60 秒
        → 降到 40% 以下 → 完成
        → 沒降夠 → kill window + fresh start（刪 session-id）
```

**檔案**：`context-guardian.ts` (104 行) + `daemon.ts` rotate handler (209-258 行)

| # | 問題 | 影響 |
|---|------|------|
| 1 | 40% 閾值太低 | Opus 1M 只用到 400K 就觸發 |
| 2 | /compact 效果有限 | 只降 5-10%，很容易「沒降夠」就被殺 |
| 3 | Fresh start 太激進 | 刪 session-id = 對話脈絡全失 |
| 4 | 沒有通知、沒有 handover | 用戶不知道發生了什麼，新 session 不知道之前在做什麼 |
| 5 | 沒有 idle 偵測 | 可能在工作中途強制中斷 |

---

## 2. 業界調研

| 方案 | 作者/組織 | 核心思路 |
|------|----------|---------|
| Hook-based Rotation | Vincent van Deth | 60-65% 觸發，寫 handover doc 再換 session |
| Ralph Wiggum Loop | Geoffrey Huntley / Anthropic | 進度存檔案/git，context 用完就丟 |
| Context Virtualization | mksglu/context-mode | SQLite + FTS5，BM25 檢索 |
| Progress Files | Anthropic 官方 | progress.txt + git checkpoint |
| Anchored Summarization | Factory AI | 結構化摘要，評分 3.70/5 |
| Observation Masking | JetBrains Research (NeurIPS 2025) | 舊 tool output 換成 placeholder，簡單 ≈ 複雜 |

**共識**：狀態外部化 > 壓縮 context。Context 是消耗品，外部狀態才是永久的。

**關鍵數據**：
- Vincent van Deth：**60% 時 handover 品質最高，65% 已開始退化**
- JetBrains：簡單方法 ≈ 複雜策略，不需要過度設計

**參考資料**：
[Vincent van Deth](https://vincentvandeth.nl/blog/context-rot-claude-code-automatic-rotation) |
[Anthropic harness](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents) |
[Factory AI](https://factory.ai/news/evaluating-compression) |
[JetBrains](https://github.com/JetBrains-Research/the-complexity-trap) |
[context-mode](https://github.com/mksglu/context-mode) |
[MemGPT](https://arxiv.org/abs/2310.08560) |
[LangChain](https://blog.langchain.com/context-management-for-deepagents/) |
[Ralph Wiggum](https://github.com/anthropics/claude-code/tree/main/plugins/ralph-wiggum)

---

## 3. 設計方案

### 3.1 核心洞察

既然 60% 時 handover 品質最高，為什麼不直接在 60% rotate？

- 不需要 /compact（效果有限，Claude Code 自己會 auto-compact）
- 不需要多層閾值
- 不需要獨立通知系統（handover prompt 讓 Claude 自己告知用戶，天生 channel-agnostic）
- 新 session 拿到高品質 handover + 100% 乾淨 context

### 3.2 Handover 機制

透過 tmux sendKeys 打字進 Claude prompt（語意上是系統指令，不是用戶訊息）：

```
你的 context 已使用 {pct}%，即將進行 rotation。請：
1. 簡短告知用戶你正在保存工作狀態
2. 將目前工作狀態寫入 memory/handover.md，包含：正在進行的任務、已完成的部分、下一步計劃、重要決策
```

Claude 收到後會：
1. 透過當前 channel 回覆用戶 — **通知**
2. 寫 `memory/handover.md` 並更新 MEMORY.md — **交接**

檔案策略：永遠覆寫同一個 `memory/handover.md`。歷史由 memory-layer 自動備份到 SQLite。新 session 透過 Claude auto memory 載入 MEMORY.md 自然看到 handover。

### 3.3 完成偵測

三個信號競爭，先到先贏：

| 信號 | 來源 |
|------|------|
| `handover.md` change event | memory-layer (chokidar) |
| Claude 回到 idle | tmux-prompt-detector |
| 60 秒 timeout | fallback timer |

### 3.4 安全閥

| 安全閥 | 用途 | 預設值 |
|--------|------|--------|
| **Idle gate** | 不在工作中途中斷。超時放棄本輪，下次 poll 重試 | 5 分鐘 |
| **Completion timeout** | Handover 寫不完也不 block | 60 秒 |
| **Grace period** | rotation 後新 session 可能馬上超 60%，防止循環 | 10 分鐘 |

---

## 4. 狀態機

```
    ┌───────────────────────────────────────────┐
    │                                           │
    v                                           │
┌──────────────┐                                │
│   NORMAL     │ <── 5min timeout ──┐           │
│   (< 60%)    │                    │           │
└──────────────┘                    │           │
        │                           │           │
        │ ≥ 60% or max_age_hours    │           │
        v                           │           │
┌──────────────┐                    │           │
│   PENDING    │ ───────────────────┘           │
│ (等待 idle)  │                                │
└──────────────┘                                │
        │                                       │
        │ idle detected                         │
        v                                       │
┌──────────────┐                                │
│  HANDING     │                                │
│   OVER       │                                │
└──────────────┘                                │
        │                                       │
        │ done or 60s timeout                   │
        v                                       │
┌──────────────┐                                │
│  ROTATING    │                                │
│ (kill+spawn) │                                │
└──────────────┘                                │
        │                                       │
        │ respawn done                          │
        v                                       │
┌──────────────┐                                │
│   GRACE      │ ───────────────────────────────┘
│  (10 min)    │
└──────────────┘
```

---

## 5. 實作範圍

### 修改檔案

| 檔案 | 修改 |
|------|------|
| `context-guardian.ts` | 重寫為上述狀態機 |
| `daemon.ts` | rotate handler 改為 idle 等待 + handover + respawn |
| `types.ts` | GuardianConfig 改為單閾值 + 安全閥 |

### 利用的現有基礎設施（零改動）

| 元件 | 用途 |
|------|------|
| `tmux-prompt-detector.ts` | idle 偵測 |
| `tmux-manager.ts` sendKeys | 送 handover prompt |
| `memory-layer.ts` chokidar | 偵測 handover.md 寫入完成 |
| `memory-layer.ts` SQLite | handover 歷史備份 |
| Claude auto memory (MEMORY.md) | 新 session 恢復 |
| `daemon.ts` spawnClaudeWindow | 重啟 Claude |

### Configuration

```yaml
context_guardian:
  threshold_percentage: 60
  max_idle_wait_ms: 300000
  completion_timeout_ms: 60000
  grace_period_ms: 600000
  max_age_hours: 8
```

---

## 6. 風險與緩解

| 風險 | 緩解 |
|------|------|
| Claude 忽略 handover prompt | 60 秒 timeout 後繼續 rotate |
| Claude 長時間不 idle | 5 分鐘後放棄本輪，下次 poll 重試 |
| Rotation 後馬上又觸發 | 10 分鐘 grace period |
| 新 session 沒讀到 handover | Claude auto memory 保證載入 MEMORY.md |

---

## 7. 成功指標

- [ ] handover.md 成功寫入率 > 90%
- [ ] 零次工作中途中斷
- [ ] 零次 rotation 循環
- [ ] 用戶每次 rotation 都收到 Claude 的通知
- [ ] 新 session 能延續工作
