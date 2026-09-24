# 研究原始紀錄

> **TL;DR**
> - 原始證據，平常不用讀；要質疑或重新驗證某個決定時才查。
> - 記住：這些檔案**原樣保存**，不適用 150 行與 ADHD 格式規則；語言也維持原樣（中英混合）。
> - 下一步：從 [../DECISIONS.md](../DECISIONS.md) 找到決策，再從下表找到支持它的檔案。

全部整理於 2026-09-24，設計階段。只把絕對路徑換成佔位符（每個檔案開頭有註明）；文中提到的腳本、log、schema dump 沒有歸檔。

## 檔案

| 檔案 | 是什麼 | 支持的決策／文件 |
|---|---|---|
| [REWRITE-PLAN.md](REWRITE-PLAN.md) | 設計階段的正式計畫（規劃 r4）與 D1–D24 原文 | 全部；後來的決策優先於規劃本文 |
| [spike-codex.md](spike-codex.md) | codex 0.156.1 app-server 實測（S1–S7） | D3、D4、BACKEND-BEHAVIORS |
| [spike-claude.md](spike-claude.md) | Claude Code 2.1.281 實測（C1–C7） | D16、BACKEND-BEHAVIORS |
| [spike-claude-f.md](spike-claude-f.md) | claude 追加實測（F1–F5：來源說明 0/3 → 3/3、Stop hook 排隊） | D16 |
| [spike-opencode.md](spike-opencode.md) | opencode 1.18.31 實測（O1–O6） | BACKEND-BEHAVIORS、送達模型 |
| [runtime-spike.md](runtime-spike.md) | 自有 holder vs tmux vs herdr 實測 | D3 |
| [injection.md](injection.md) | v1 訊息注入機制與不穩定紀錄 | D7、D16、送達模型、V1-LESSONS |
| [inventory.md](inventory.md) | v1 功能盤點（約 18.3 萬行）與文件／程式不一致處 | D1、D5、D6、D7、V1-LESSONS |
| [history.md](history.md) | v1 git 歷史：bug 熱區、功能擺盪、invariant 測試清單 | D2、D9、V1-LESSONS |
| [usage.md](usage.md) | v1 實際使用率（MCP 呼叫次數、log、磁碟） | D8、D17、範圍（規劃 §3） |
| [workflow-validation.md](workflow-validation.md) | 用 v1 8,347 個真實 task 驗證關卡模型 | D15、D18、D19 |
| [competitors.md](competitors.md) | 17 個同類工具的市場調查 | 定位、D5、D20 |
| [v1-architecture-rfc.md](v1-architecture-rfc.md) | v1 未合併的架構簡化 RFC：被推翻的前提與方法論教訓 | D10、方法論（先證據後解法） |

## 下一步

```bash
cat docs/DECISIONS.md
```
