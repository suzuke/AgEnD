# 第 12A：observed v8 的 gh audit 範圍失敗

> **TL;DR**
> - 固定 `81804dc`／v8 經使用者授權，在四個 CI checks 全通後執行一次；完整 smoke **FAILED**。
> - 五個外部 shim 路徑、exit 1 與唯一工作 gh_merge 拒絕正確；四個 startup token 拒絕被 verifier 誤算進工作次數。
> - 下一步：修正 INITIAL 前後的 append-only audit 邊界，獨立覆核及新計畫另核；不自動重跑。

## 原始執行

計畫 SHA `61d63c51518bde0ea9c15e0c23d04ef0bd7e2366a153429b8679fd05b8477af2`；Claude 2.1.284、兩個 Haiku 4.5 low 計畫，100×24、七則工作／900 秒／零重跑。900 秒不是 API 次數或金額硬上限。兩個初始 idle、各三個 production startup keys 與四份原始 frames 已保存。

實際一則 harness INITIAL confirmed/channel、一則模型 A→B peer sent/channel；B 的後續本機 ACK 待同步，不能改稱 native confirmed。往返、Stop queue 與 Interrupt 沒有完成。A 將指定 INITIAL 整段放在一個 foreground Bash call，之後另做一次未要求的唯讀 ls／cat，這項額外工作也列入原始證據，不宣稱全部 prompt 限制已遵守。

`gh-guard-observation.json` 在斷言前保存五個正確 PATH、拒絕文字、exit 1 與完整 native audit。audit 有 A／B 各兩個 `gh_token`／`auth token` 拒絕，發生於 INITIAL 之前，另有一次 A 精確 cwd／`gh_merge`／`pr merge` 拒絕。沒有真 merge 動作；錯誤 `native gh refusal identity/count differs` 來自 verifier 要求整份 audit 只能有一個 gh row，不能據此聲稱防護繞過。

## 修正與零模型驗證

兩個初始 idle 後、第一則 harness send 前先保存 audit 原始 prefix。prefix gh 只允許 A／B 各自精確 cwd 的 `refuse`／`gh_token`／`auth token`；後續 audit 必須保留完全相同 prefix，suffix 只允許一個 A gh_merge。未知啟動 gh、prefix 改寫／截短、工作期間 token／其他 gh／重複 merge、錯誤身分一律失敗；不靠 timestamp 判定歸屬。

Bash／zsh 分別跑真正 shim 產生的四個 startup refusals 與 INITIAL，核空 prefix 正例、缺 baseline 負例、各二十個 mutations、完整 peer quoting 與七段語法。fixtures 自動移除，前後實際 check-deps/no-std 與 fmt 通過。七個 prompt、budget、driver、startup classifier、ACK／route／session 與 gh 政策不改；歷史 native binary pins 未重建，Rust source tree 未改。

全新 verifier 對固定 `a85a91a`／v9 零模型覆核判 REFUTED：`read_text()` 把 CRLF 正規化成 LF，使 byte 改寫仍被接受。v9 沒有執行，也不覆寫原覆核；修正版使用 raw bytes／base64 保存及逐 byte prefix 比對，追加換行改寫與無效 UTF-8 反例，兩個 shell 各二十三個反例通過。新計畫待另一次全新覆核。

## CI、清理與覆核

`81804dc` push Ubuntu 原輪在既有 TUI application-cursor 按鍵測試逾時；相同 head PR Ubuntu 通過，本機原 case 六次通過。同一 head 只重跑失敗 job 一次後四 checks 全通，原始 log 保留，沒有宣稱修復時序問題。

native cleanup 回報兩個 holders Gone／owned holders absent，home 與 session／scratch 暫存已清理；使用者要求的 trust entries 保留，cleanup 不寫共用 account、不停止外來 session。必要私有 raw transcripts／events／audit／plan／authorization／CI 與失敗證據保留；全新無相關 context verifier 已核固定 Git818、四個 CI jobs、raw transcript／hook／audit 及八個證據反例；五個 PID、兩個 agent PGID 與十四個精確路徑目前不存在。這是有限失敗／清理覆核，沒有完整歷史 OS 副作用 telemetry，不能補稱歷史外來程序／資料零影響。

## 下一步

新固定計畫完成獨立覆核後，依 [D40](../decisions/d40.md) 另取真模型執行授權；完整第 12A 仍未驗收，本次未 merge。
