# 第 11 施工關 C 段：Codex U17 基礎驗證

> **TL;DR**
> - 真 holder／wrapper／PTY、fake Codex app-server、driver 與 SQLite 已驗人工 turn 和 daemon 訊息分開對帳。
> - 兩個原生情境通過；曾重現未嘗試送出的 queued 訊息被人工文字誤判 confirmed，修正後同一情境通過。
> - 下一步：補真 daemon／client／App 的完整 U17，再做明確 opt-in 的真 Codex smoke；目前仍 not_supported。

## 實際路徑與範圍

`crates/agend/tests/codex_u17.rs` 啟動真 holder 和 production Codex shell wrapper。明確設定 `-c agend_fake_manual_tui=true` 的 fake frontend 才進 raw PTY 模式；它把收到的 bracketed paste／Enter 送到同一 remote thread 的 `turn/start`，不帶 daemon clientUserMessageId。busy／idle、user items 和 queue 都由實際 fake app-server 產生，不由測試造事件。fake Codex 的預設 frontend 保持原行為。

driver／runtime／SQLite 是真實作，但在測試程序內組裝。這不是整個 daemon 程序重啟，也未經 client 權限或完整 App；不能把它當完整 U17。沒有啟動真 Codex 或 LLM。

| 情境 | 核對結果 |
|---|---|
| 人工 turn 與 queue | 繁中多行人工輸入出現在同一 thread；driver 觀察 busy／idle，人工輸入不新增 daemon message row。busy 時 Queue 訊息先是 Sent、turn id 空，再以自己的 clientId／turn 確認；一個 daemon row、一個 receipt |
| 相同文字與 component restart | driver 斷線時留下 Queued、attempted_at 空的訊息；PTY 人工輸入完全相同的 From 標頭與 body。runtime／driver／SQLite 關閉重接後 holder pid、generation、thread 不變，舊 attach 拒絕；daemon row 待自己的 queue turn，不能把人工 turn 當 receipt |

## 原反例與修正

原 history 匹配允許任何 no-turn Queued row 用相同文字對帳。第二個原生情境在原程式 exit 101：沒有實際送出嘗試，重接後仍被標 confirmed，人工 turn 取得了 daemon 訊息歸屬。原 log `u17-foundation-original.log` 與原 history source 保留。

修正 `driver/codex/history.rs`：

- 明確外來 clientId 不退回文字匹配；自己的 clientId 仍優先。
- 沒有 clientId 的 no-turn Queued row，必須已有 attempted_at 才能走舊 crash-window 文字對帳。
- 同 turn／text 的舊相容路徑保留；既有 crash fixture 補上實際 attempted_at。

這未消除「真正嘗試後、缺 clientId、人工又輸入相同文字」的所有歧義。完整 U17 與實際版本 smoke 還須證明 receipt／busy 不會污染；目前沒有放寬 Codex 終端輸入政策。

## 本機證據

| 檢查 | 結果 |
|---|---|
| 原生 U17 foundation | 2 passed／0 ignored |
| history 匹配 | 4 passed，含兩個新回歸 |
| 既有 Codex driver 契約 | 15 passed，包含忙碌三級、lost reply、冪等與四次重啟 |
| fake／錄製檔形狀一致性 | 18 passed |
| 完整 daemon＋testkit（attention 修正前） | 260 主 suite passed／0 ignored |
| workspace clippy、fmt、實際 thumb no-std | 通過，沒有 allow-skip |

原 logs 位於 `/private/tmp/g11c-implementation-logs/u17-*.log`。filtered re-exec child probe 不重複加到主 suite 計數。後續 attention／draw 修正另外重驗，不把較早全套結果當最新 head 的結果。

## 重跑

```bash
cd /Users/suzuke/AlphaCR-worktrees/AgEnD-v2-g11c-terminal
export CARGO_TARGET_DIR=/private/tmp/AgEnD-g11c-target
~/.cargo/bin/cargo build -p agend-testkit --example fake_codex
~/.cargo/bin/cargo test -p agend --test codex_u17 -- --nocapture
~/.cargo/bin/cargo test -p agend-daemon --test codex_driver
```

## 下一步

接通完整 U17 fixture，準備可檢視的 opt-in live smoke；再核已測版本、全新無 context verifier 與使用者確認。C 段仍在 [draft PR #145](https://github.com/suzuke/AgEnD/pull/145)，尚未驗收或 merge。
