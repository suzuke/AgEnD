# 第 12A：第三次 smoke 在 terminal 註冊前停止

> **TL;DR**
> - 固定6a666ba／observed v3 經另行授權執行一次，在 B 首次讀取回 no_terminal 時停止；完整 smoke FAILED。
> - 零工作訊息；兩個早期 scratch namespaces 補清，13 精確路徑／5 PID／兩 trust keys absent。
> - 下一步：首次註冊等待修正交全新 verifier；新真 CLI 計畫另取授權，#153 未 merge。

## 原始執行

計畫 SHA `f919718c696e7e15c899c02b4cfb888203d80851303d7ef863e7844675e98a46`。
一次版本查詢、兩次 instance add；A 取得一份空白100×24原始 frame，
B helper exit1：`startup_frame: instance has no live terminal`。
約一秒內停機，無 status、SessionStart、startup key 或 persisted messages。
B trace 的 evidence_file 只是預定名稱，實際沒有 B frame；不能當成兩份成功捕捉。
這次是 harness 在 terminal 註冊前訂閱的競態，沒有證據支持再次歸因 Ready literal。
命令只記完成時間；native messages 為零不代表精確 API 次數或費用為零。

## 修正與原生驗證

新 `scripts/claude_observed_ready_smoke.py` 保留已執行的舊 runner／plan／binary bytes。
首次每 instance 最多20次只讀觀察、10秒；精確 exit1、空 stdout 與已固定 helper 的
no_terminal stderr 才能繼續觀察。每次先核 status，attention／failed／restart 立即停止。
其他拒絕、timeout、後續 batch 的 terminal 消失均不重讀；每次 helper 最多5秒，
成功 frames最多28份，含首次未就緒觀察的 helper reads最多66次。
全新 verifier 曾以12秒 status 揭露原10秒註冊期限沒有涵蓋前置 status；
修正 status timeout 也使用註冊剩餘時間，保留原反例，不把期限改稱 helper-only。
這是首次畫面觀察，不重啟 instance，也不重送任何工作訊息；原七則工作訊息／900秒不變。

`verify_observed_ready_smoke.py` 使用真 daemon／holder／shell：先只新增 A，
訂閱 B 必須由 native producer 回 no_terminal，讀取紀錄出現後才延遲新增 B。
舊 runner 重現拒絕；新 runner 取得 A／B frames。
native DB 核零 messages／driver events，未執行真 Claude／模型。
另核未註冊讀取次數上限、10秒期限、helper timeout 不 replay、不同 stderr／exit／非空 stdout
與後續 terminal 消失拒絕，以及 attention 前不呼叫 helper。

## 清理與證據限制

runner 成功停止自有 daemon／holders 並刪 home，但因未收到 SessionStart，
原清理漏掉兩個早期 scratch namespaces。root 根據執行前 namespace freshness，
停止後核 uid／節點型別／symlink／hardlink，再私有保存空目錄拓樸並補清。
歷史節點檢查只有執行者紀錄，刪除後不能獨立重建；沒有歷史 PGID snapshot。
當下13精確路徑、5個記錄 PID、兩個 trust keys 均 absent，account settings 未改。
新 runner 在 native cleanup 成功後也掃描唯一 nonce scratch，保存刪除前節點 metadata；
有檔案時先保存必要證據，再移除。native fixture 核未有 SessionStart 的兩個目錄與檔案均清理。
未合併 author worktree、必要私有證據及歷史固定 binaries保留。

## CI 與下一步

6a666ba 的 PR 雙平台 CI 通過；push macOS 同一 outer PTY 回歸306.841042ms超過原300ms，
Ubuntu 通過。失敗 log 保留，首次註冊等待修正不宣稱修復該效能問題。

完成全新 verifier、清理及固定新計畫後，依 [D40](../decisions/d40.md) 取得新真 CLI 授權。
完整模型 smoke 尚未通過；merge 仍等使用者確認。
