# 第 12A：startup capture 輸入時機與清理身分

> **TL;DR**
> - 首次三鍵真蒐證在 100 欄只完成 Down，未觀察到 Yes；已停止並清理。
> - 完整提示須穩定一秒才送鍵，未知畫面重新計時；這不證明真失敗原因。
> - 下一步：全新 verifier 核對新工具與固定計畫後，新真執行另取授權。

## 三鍵真蒐證首個失敗（2026-10-05）

使用者授權固定計畫後查版本成功，100×24 只送一次 Down 並收到 completion；
revision 6／8 仍為完整 No 畫面，沒有觀察到 Yes。按首個失敗即停的條件，
未重送、未送 Enter、未執行 140 欄；0 模型／訊息。自有 native lab 與程序清理已核。
外部 DB observer 沒取得 session；未建立 trust entry，也未宣稱真按鍵被 CLI 消費或啟動完成。

新增保護等完整已知提示穩定一秒，期間仍讀 frames／控制權變化；未知提示清除候選，
不以時間代替完整選單、身分、ACK 或啟動完成。延遲啟用 raw input 的 native 替身先重現
原 consumer 過早送鍵後未切換，新 consumer 的兩寬回歸通過；真失敗原因仍未證明。
`cleanup-identity.json` 在啟動前保存本次 canonical home／workspace／session，0600，
只供自有資源清理，不含帳號、內容或程式參數，與遮蔽 transcript 分離。原始失敗證據保留。
新工具／計畫重驗後，任何新真執行均須新計畫授權；本次執行沒有續用未花完的鍵數。

19 個原生 capture 測試、整個 daemon、fmt、workspace clippy 與實際 no-std 已通過；新全新 verifier 待核。兩寬延遲 receiver 與未知畫面插入回歸核對等待與重新計時；清理資訊核對 native argv 的 session，以及 home／workspace 已移除。

## 下一步

保留首個真失敗證據。新真蒐證須綁定新提交與 binary hash、使用全新 output，不能沿用已停止計畫的剩餘鍵數。原生重驗指令見 [startup capture](gate-12a-startup-capture.md)。
