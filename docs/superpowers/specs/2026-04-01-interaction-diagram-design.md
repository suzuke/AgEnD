# Agent 互動視覺化儀表板

## 需求

使用者需要清楚地看到 fleet 中各個 Agent 之間的互動狀況，包括：

1. **全域視角**：顯示所有 instance 之間的訊息流，可按時間範圍篩選
2. **任務追蹤視角**：以 correlation_id 為起點，展開完整的任務委派鏈（誰委派了誰、誰回報了什麼）
3. **即時更新**：打開網頁就能看到最新狀態，前端 polling 定期刷新
4. **呈現方式**：Sequence Diagram，以時間軸展示訊息流

## 現有資源

### 可直接使用

| 資源 | 位置 | 說明 |
|------|------|------|
| HTTP Server | `fleet-manager.ts` port 19280 | 已有 `/health`、`/status` 路由，可直接擴展 |
| EventLog class | `src/event-log.ts` | SQLite WAL mode，通用 insert/query/prune，已有 index |
| 訊息攔截點 | `src/outbound-handlers.ts:sendToInstance` (L35-104) | **所有**跨 instance 訊息都經過此函數，包括 `request_information`、`delegate_task`、`report_result`（它們都 re-dispatch 回 `sendToInstance`） |
| 訊息 metadata | 同上 L67-81 | 已建構完整的 `ipcMeta`：`from_instance`、`correlation_id`、`request_kind`、`requires_reply`、`task_summary`、`ts` |
| Fleet config | `~/.agend/fleet.yaml` | instance 列表、description、tags |
| Instance 狀態 | `statusline.json` per instance | context%、cost、model |

### 已有的事件類型（events.db）

| event_type | payload | 觸發點 |
|---|---|---|
| `cost_snapshot` | `{session_cost_usd, accumulated_cents}` | 每日重置 |
| `context_rotation` | `{reason, context_pct, ...}` | context 溢出重啟 |
| `hang_detected` | `{}` | hang detector |
| `crash_loop` | `{}` | daemon 反覆 crash |
| `instance_paused` | `{reason, cost_cents}` | 觸及成本上限 |
| `schedule_deferred` | `{schedule_id, label, ...}` | 排程因 rate limit 延後 |
| `model_failover` / `model_recovered` | `{from, to, reason}` | 模型切換 |

## 缺少的部分

### 1. 互動訊息持久化（核心缺口）

**現狀**：IPC 跨 instance 訊息經過 `sendToInstance` 後直接送出，沒有寫入任何持久化儲存。訊息過了就消失。

**需要**：新增 `interactions` 表到 `events.db`（或獨立 db），記錄每一筆跨 instance 訊息。

```sql
CREATE TABLE interactions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  correlation_id TEXT NOT NULL,
  from_instance TEXT NOT NULL,
  to_instance TEXT NOT NULL,
  request_kind TEXT NOT NULL,        -- 'query' | 'task' | 'report' | 'update'
  requires_reply INTEGER DEFAULT 0,
  task_summary TEXT,
  message_preview TEXT,              -- 前 200 字
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_interactions_correlation ON interactions(correlation_id);
CREATE INDEX idx_interactions_time ON interactions(created_at);
CREATE INDEX idx_interactions_from ON interactions(from_instance, created_at);
CREATE INDEX idx_interactions_to ON interactions(to_instance, created_at);
```

**插入點**：`outbound-handlers.ts` 的 `sendToInstance` 函數，在 `targetIpc.send()` 之後加一行 insert。

### 2. 查詢 API

**現狀**：HTTP server 只有 `/health` 和 `/status`。

**需要新增路由**：

| 路由 | 說明 |
|------|------|
| `GET /api/interactions?since=&until=&limit=` | 全域互動列表，按時間倒序 |
| `GET /api/interactions/:correlationId` | 追蹤特定 correlation chain 的所有訊息 |
| `GET /api/interactions/topology?since=` | 彙總：from→to 配對的訊息數量，用於拓撲概覽 |
| `GET /dashboard` | 回傳儀表板 HTML |

### 3. 儀表板前端

**現狀**：不存在。

**需要**：單一 HTML 檔案（inline JS + CSS），內嵌到 HTTP server。

核心功能：
- **Sequence Diagram 渲染**：用 SVG 繪製，每個 instance 一條垂直生命線，訊息用水平箭頭連接
- **時間範圍選擇**：最近 1h / 6h / 24h / 7d
- **全域 → 聚焦切換**：點擊任一訊息箭頭，聚焦到該 correlation_id 的完整 chain
- **Polling 更新**：每 5 秒 fetch `/api/interactions`，差異更新 diagram
- **訊息類型色彩**：task=藍、query=綠、report=橙、update=灰
- **箭頭上的標籤**：顯示 task_summary（截斷）+ request_kind

### 4. InteractionLog class

**需要**：封裝 interactions 表的 CRUD，類似 EventLog 但 schema 更明確。

```typescript
export class InteractionLog {
  constructor(db: Database.Database);
  insert(interaction: {
    correlationId: string;
    from: string;
    to: string;
    kind: string;
    requiresReply: boolean;
    summary?: string;
    messagePreview?: string;
  }): void;
  query(opts: { since?: string; until?: string; limit?: number }): InteractionRow[];
  queryByCorrelation(correlationId: string): InteractionRow[];
  topology(since?: string): Array<{ from: string; to: string; count: number }>;
  prune(days: number): void;
}
```

## 設計決策

| 決策 | 選擇 | 理由 |
|------|------|------|
| 持久化方式 | 專用 `interactions` 表（非複用 events 表） | correlation chain 查詢效率高，schema 清晰 |
| 前端框架 | 無框架，單一 HTML + inline JS/CSS | KISS，零依賴，零 build step |
| Diagram 渲染 | 自繪 SVG（非 Mermaid） | 更輕量、可控，不依賴 CDN |
| 即時更新 | Polling 每 5 秒 | 最簡單，符合使用情境 |
| 部署方式 | 內嵌現有 HTTP server (port 19280) | 零額外服務，`agend fleet` 啟動即可用 |
| 資料保留 | 30 天自動 prune（同 events） | 與現有策略一致 |

## 改動範圍

| 檔案 | 改動 |
|------|------|
| `src/interaction-log.ts` | **新增** — InteractionLog class |
| `src/outbound-handlers.ts` | **修改** — sendToInstance 加入 interaction 記錄 |
| `src/fleet-manager.ts` | **修改** — 初始化 InteractionLog、註冊 API 路由、注入到 outbound context |
| `src/dashboard.ts` | **新增** — 儀表板 HTML 字串（或 inline template） |

共約 4 個檔案，預估 ~400 行新增程式碼。
