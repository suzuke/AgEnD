# 決策索引（D1–D32）

> **TL;DR**
> - 這裡是已確認的設計決策；每條都經使用者確認。
> - 記住：**沒有新證據就不重開討論**；要推翻，先補證據再提新決策編號。
> - 下一步：找到相關決策，點進細節檔看理由、被否決的方案與證據。

來源：規劃 r4 §2（D1–D24）；D25 為使用者在設計討論中確認與 architecture 頁；D26–D32 是第 1 關開工前提案 P1–P7，使用者 2026-09-25 確認，細節在 [第 1 關頁面](gates/gate-01-core.md#開工前提案)。「規劃 §x」指 [research/REWRITE-PLAN.md](research/REWRITE-PLAN.md)；原始證據索引在 [research/README.md](research/README.md)。規劃本文與後來的決策衝突時，以後來的決策為準（見本頁底部）。

## 索引

| # | 一句話 | 細節 |
|---|---|---|
| D1 | daemon 對外只有一套公開、有版本的 protocol；TUI 是第一個 client | [d01-d08](decisions/d01-d08.md#d1) |
| D2 | daemon 常駐（launchd／systemd），可從 TUI 重啟；重啟前預檢新 binary | [d01-d08](decisions/d01-d08.md#d2) |
| D3 | 自有 holder，每個 instance 一個；daemon 重啟 agent 不斷線 | [d01-d08](decisions/d01-d08.md#d3) |
| D4 | 提交與 merge 透過 Forge（local、github）；驗證是 `command` 關卡 | [d01-d08](decisions/d01-d08.md#d4) |
| D5 | git shim 是一級元件、獨立 crate、同一 binary 以 argv[0] 分派 | [d01-d08](decisions/d01-d08.md#d5) |
| D6 | 不用 HMAC；binding 存 DB，daemon 寫唯讀快照給 shim | [d01-d08](decisions/d01-d08.md#d6) |
| D7 | agent 介面以 CLI 為主；需要時由同一份定義產生 MCP 轉接層 | [d01-d08](decisions/d01-d08.md#d7) |
| D8 | instance 全部存 DB，不再有 fleet.yaml | [d01-d08](decisions/d01-d08.md#d8) |
| D9 | 每個模組都能獨立驗證與測試 | [d09-d16](decisions/d09-d16.md#d9) |
| D10 | 切 crate 的四條準則；新增 agend-client、agend-testkit、xtask | [d09-d16](decisions/d09-d16.md#d10) |
| D11 | protocol 放 core；client 同步 I/O；driver／forge／store 為模組；狀態機在 core | [d09-d16](decisions/d09-d16.md#d11) |
| D12 | instance 只屬於一個 team；內建不可刪的 `general` | [d09-d16](decisions/d09-d16.md#d12) |
| D13 | team 共享目錄；Telegram「需要你」topic + 每 team 一個 topic | [d09-d16](decisions/d09-d16.md#d13) |
| D14 | main 前進：自動 rebase、重跑 checks；patch-id 不變才保留核准 | [d09-d16](decisions/d09-d16.md#d14) |
| D15 | team 有 0 或 1 個 repo；workflow 宣告需求 | [d09-d16](decisions/d09-d16.md#d15) |
| D16 | claude driver：互動式 TUI + hooks；中斷 = Esc 後立即經 channel 送 | [d09-d16](decisions/d09-d16.md#d16) |
| D17 | CLI 分 agent 命令與操作者命令；daemon 依身分限制權限 | [d17-d25](decisions/d17-d25.md#d17) |
| D18 | agent 不建 instance：開 task 指定角色，daemon 依角色範本分派 | [d17-d25](decisions/d17-d25.md#d18) |
| D19 | workflow 以 TOML 定義、存檔檢查、`agend workflow` 管版本 | [d17-d25](decisions/d17-d25.md#d19) |
| D20 | 人工核准 merge = 在 workflow 加 `approval(by = "human")` | [d17-d25](decisions/d17-d25.md#d20) |
| D21 | task 固定建立時的 workflow 版本 | [d17-d25](decisions/d17-d25.md#d21) |
| D22 | 施工依 crate 由下往上分成施工關，每個施工關使用者確認後才開下一個施工關（第 13 施工關見 D24） | [d17-d25](decisions/d17-d25.md#d22) |
| D23 | 文件繁中為主、程式輸出英文；每 crate 有 README／TESTING；AGENTS.md 唯一入口 | [d17-d25](decisions/d17-d25.md#d23) |
| D24 | 第 13 施工關「安裝與發布」；第 9 施工關只做 doctor、init；安裝規則在 core `setup` | [d17-d25](decisions/d17-d25.md#d24) |
| D25 | D18 分派規則是純邏輯，放 core `policy::assign`；daemon 只提供輸入 | [d17-d25](decisions/d17-d25.md#d25) |
| D26 | client／holder 協定用 unix socket 上的 JSON Lines（PTY 位元組 base64）；`hello` 協商版本，同 major 只加欄位；新 daemon 要能跟舊一個 major 的 holder 溝通（P1） | [gate-01 P1](gates/gate-01-core.md#p1wire-format-與版本協商) |
| D27 | 狀態機是純函式 `step(state, event) -> (state, actions)`，不呼叫 trait；trait 放 core、用 async、每個只放用到的最少方法（P2） | [gate-01 P2](gates/gate-01-core.md#p2trait-簽章) |
| D28 | `command` 關卡與 git adapter 跑程序都經 `Runner` trait：`run(cmd, dir, timeout)`（P3） | [gate-01 P3](gates/gate-01-core.md#p3runner-要不要-trait) |
| D29 | GitHub CI 用 `command` 關卡接（如 `gh pr checks {pr} --watch`），佔位符 `{pr}`／`{head}`／`{branch}`；forge 維持 3 個方法（P4） | [gate-01 P4](gates/gate-01-core.md#p4github-ci-怎麼進流水線) |
| D30 | 去抖動不對稱：轉 busy 立即生效、轉 idle 穩定 5 秒；第 7 關用真實資料校準（P5） | [gate-01 P5](gates/gate-01-core.md#p5去抖動) |
| D31 | 保留期限：task／workflow／decision 永久，訊息 30 天，事件與狀態轉換 14 天，WIP patch 30 天，DB 快照 7 份；第 5 關校準（P6） | [gate-01 P6](gates/gate-01-core.md#p6保留期限) |
| D32 | core 唯一依賴 `serde`（`default-features = false`，只開 `derive` + `alloc`）供型別 derive；JSON 編碼在 adapter（P7） | [gate-01 P7](gates/gate-01-core.md#p7core-依賴-allowlist) |

## 來源衝突與處理

| 衝突 | 處理 |
|---|---|
| 規劃 §4.5「每個 repo 選自動或人工 merge」、規劃 D4 的 Checks 介面 | D19／D20 取代：人工 = `approval(by = "human")`；checks = `command` 關卡 |
| 規劃 §4.5「main 前進後的政策（待定）」 | D14 已決定 |
| 規劃 §4.4 第一張表：claude 排隊 = channel、opencode 插入／中斷「未查證」 | 以 spike 表與 D16 為準（見 BACKEND-BEHAVIORS） |
| 規劃 §5／§5.1 的 `checks/{command,forge}` 模組與 Checks trait | 以 architecture 頁為準：daemon 用 `runner`，trait 清單無 Checks |
| runtime spike 建議用 herdr | D3 選自有 holder（見 D3 細節） |
| 規劃 §6 的功能階段 | D22 改為分成施工關；功能階段只當里程碑 |
| D22 原文寫 12 個施工關 | D24 加上第 13 施工關；以 ROADMAP 的 13 個施工關為準 |

## 下一步

```bash
cat docs/decisions/d01-d08.md
```
