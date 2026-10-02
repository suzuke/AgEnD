# 第 11 施工關 C 段：CI 反例修正

> **TL;DR**
> - CI 找到 retry attention 的舊快照重建，以及重新取得控制時使用舊視窗尺寸。
> - attention 的受控交錯已重現原錯誤並修正；draw 尺寸同步的缺失 mutant 被新回歸拒絕。
> - 下一步：核修正後的原生 App 與最新雙平台 CI；這些局部證據不代表 C 段完成。

## Retry 後重建失敗項目

`d8b65cc` 的 macOS PR CI（job 111016591768）在 CLP-11 失敗。raw events 顯示 Retry 已發 AttentionResolved，接著同一個「daemon 開始前失敗」的 reason／waiting_since 又被 AttentionRequired 加回，之後 instance 已啟動。

pipeline refresh 讀取 failure attention 快照，再 await held_task；handler 可在期間處理 Retry。原 blind upsert 把已移除的快照重建，也可能覆蓋同 id 的新失敗。

`pipeline/attention/tests.rs` 在第三次 view 捕捉真 Fleet 快照後執行真 resolve，才回傳舊快照。它注入排程交錯，不改 producer payload，也不靠 sleep：

| 情境 | 原程式 | 修正後 |
|---|---|---|
| operator Retry 移除 | 舊失敗復活，exit 101 | attention 保持移除，不重發舊 Required |
| Retry 後新失敗 | 新 reason／waiting_since 被舊快照覆蓋，exit 101 | 新 failure 原樣保留 |
| 未變更項目補 unblocks | — | 正常更新一次；相同值不重發，錯 id／舊值／已移除皆拒絕 |

core 的 `PipelineView::replace_attention_if` 規定 compare／replace／publish 原子完成；Fleet 以同一把鎖實作。DB await 之後只更新仍等於捕捉值的項目。handler 的 Retry 回覆路徑維持直接 resolve／交給 supervisor，不等待 DB enrichment。

完整 daemon unit 76 passed，真 client protocol 9 passed；`accept core` 156 passed／2 個既有 ignored、demo／workspace clippy／fmt／實際 no-std 通過。原負面 log 是 `attention-race-original.log`，修正證據為 `attention-race-fixed/lib/client/core.log`。

## Draw 與實際視窗尺寸

`7389b18` 的 macOS PR CI（job 111018614790）在真外層 PTY 情境失敗：舊視窗縮成 20×5 後重新取得控制，agent 仍是 31×8，而非 20×4。原 log 保留；CI 沒有記逐筆 Resize events，無法判定特定通知在何時延遲或遺失。

原 interactive loop 只靠啟動尺寸與 Resize events 更新 App；ratatui draw 卻另讀目前 backend 尺寸。現在 native draw 與 off-screen draw 共用 `render_frame`，先用實際 frame area 更新 App，再 render。延遲／合併的通知不能讓 control 使用和正在畫的視窗不同的尺寸。

新 parser-backed App 回歸刻意省略 Resize event，再注入舊 Resize event；核 Acquire／Resize 最終 frame 為 20×4，未確認前的鍵不送，確認後繁中文字逐 byte 到達 producer。移除 draw-size 同步的 mutant 在同一情境失敗 exit 101，finally 還原；正向通過。這是受控缺通知的證據，不宣稱已還原 CI 當下的確切 signal 排序。

原 CI log：`latency-ci-111018614790-failed.log`；本機：`resize-backend-positive/negative/tui/outer.log`。均在 `/private/tmp/g11c-implementation-logs`。

## 修正後本機檢查

完整 TUI 82 passed／0 ignored；真外層 PTY 6 passed／0 ignored，12 次最後 dirty 仍皆 ≤300 ms（最慢 207.166 ms）；U17 foundation 2 passed／0 ignored。最新 workspace clippy／fmt／實際 thumb no-std 通過。這次沒有重算完整 workspace；前批 CI 的成功不代替本批 head。

## 重跑

```bash
cd /Users/suzuke/AlphaCR-worktrees/AgEnD-v2-g11c-terminal
export CARGO_TARGET_DIR=/private/tmp/AgEnD-g11c-target
~/.cargo/bin/cargo test -p agend-daemon --lib pipeline::attention::tests
~/.cargo/bin/cargo test -p agend --test client_protocol
~/.cargo/bin/cargo test -p agend-tui --test full_app
~/.cargo/bin/cargo test -p agend --test tui_outer_pty -- --nocapture
```

## 下一步

等最新 head 的雙平台 CI，再補剩餘矩陣與 [Codex U17](gate-11c-u17-validation.md)。完整 C 段仍要全新 verifier、逐步人工驗收與使用者 merge 確認。
