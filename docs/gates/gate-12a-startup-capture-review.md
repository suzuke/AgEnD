# 第 12A：startup capture 獨立核對紀錄

> **TL;DR**
> - 本頁保留各固定提交的原生核對與反例；只認證蒐證工具。
> - `dabb35e` 已通過全新 verifier、使用者重驗及雙平台 CI。
> - 下一步：新增穩定等待與清理身分後，重新取得全新 verifier 核對。

## 獨立核對

固定 `356fbef` 獲全新無相關 context verifier 局部 CONFIRMED。原 `4511e21` 的真 holder 跨列 synthetic Bearer 反例會寫入且回成功；修正後同一 producer 重播回失敗並在寫入前拒絕。另核 20／100／140／200 欄與 5／24／100 列的 native 寬字 spacer、多列電郵、零 stdin 與清理；5 個工具回歸、workspace clippy／fmt／實際 no-std 通過。這份結果只驗工具，不證明真 Claude 的版本、提示或 startup complete。

固定 `b0d8074` 的全新 verifier 用真 daemon／holder 找到兩個誤確認反例：外部路徑以自有
workspace 開頭、或錯誤 workspace 畫面在別處提及自有路徑，皆收到 Down／Enter。
原 REFUTED 證據保留；作者改成唯一標頭與完整路徑相等，補兩寬原生拒絕回歸，
修正 `7abb646` 獲同一驗證者局部 CONFIRMED，六個兩寬路徑反例零輸入；
回條故障不重送，整個 daemon／clippy／fmt／實際 no-std 通過並清理。
固定 head 的 push／PR 雙平台 CI 均成功。這份核對不認證正式 P5／P6。

固定 `386d083` 的全新 verifier 局部 CONFIRMED：四個 fixture 逐 byte 對齊來源；
四種 classifier mutation 均被抓到；core／fmt／clippy／實際 no-std、獨立 agend
及 10 個 native capture tests 通過。自有 worktree／branch／target 與程序已清理。
140 欄的 live DB identity 觀察缺口仍保留；不認證正式 P5／P6 或完整 12A。

固定 `28d6341` 的全新 verifier 找到三個原生反例：selected Exit 加 local decoy、
重複 selected local、或追加 selected Exit，原工具均送第三鍵並成功。原 REFUTED 證據保留；
改為整份版本化畫面只忽略空白後相等，新增兩寬拒絕回歸；修正獨立核對待完成。
未執行真三鍵蒐證。

固定 `8d605bf` 的另一位全新 verifier 找到 outer loop 的身分缺口：
trust 回條後的 native frame 改成外來 instance／view 仍送第三鍵。原 REFUTED 證據保留；
新回歸在原 consumer 失敗，補 subscribe／acquire／outer frame 的 instance／view／generation 核對。
修正後 15 native cases／整個 daemon／fmt／clippy／實際 no-std 通過，新獨立驗證待完成，真三鍵未執行。

固定 `01f438e` 的全新 verifier 又重現 resize 等待迴圈略過不一致 frame 後仍送三鍵；
原 REFUTED 保留。新增兩寬 instance／view／generation／size 回歸先核原 consumer 失敗；
resize ACK 前只接受本次身分，以及原始或目標尺寸兩種合法過渡。修正後 16 native cases／整個 daemon／fmt／clippy／實際 no-std 通過；新全新 verifier 待核，真三鍵未執行。

固定 `433d2a8` 的全新 verifier 另重現 trust 選單追加 selected Exit 仍確認，原 REFUTED 保留。
trust 改為完整已錄製 No／Yes fixture，只替換本次 canonical path；新回歸先核原 consumer 失敗；17 native cases／整個 daemon／fmt／clippy／實際 no-std 通過，新全新 verifier 待核。

固定 `dabb35e` 的全新 verifier 局部 CONFIRMED：17 native cases、整個 daemon、fmt、workspace clippy、實際 no-std，以及獨立 38 個選單／30 個回條故障案例通過。使用者以固定提交重驗相同範圍通過；push／PR 的 macOS、Ubuntu CI 均成功。自有 worktree、branch、target、程序與重複暫存已清理。此結果不認證真 CLI、正式 P5／P6 或完整 12A。

## 穩定等待與真兩寬 ready capture

固定 `e0cedfb` 的全新 verifier 局部 CONFIRMED：19 個 capture／199 個 daemon 測試、獨立 32 native cases、fmt／workspace clippy／實際 no-std 通過；push／PR 雙平台 CI 通過。自有 worktree／branch／target／runtime 均清理。

另獲 ready 計畫授權後，固定 Claude 2.1.284 的 100×24／140×24 各完成 Down、trust Enter、development Enter，保存 13／11 個真 frames 與主介面；沒有模型 prompt／團隊訊息或追加鍵。另一位全新 verifier 只讀審核授權、來源／binary hash、完整選單、intent／completed／result 一致性與指定殘留目前不存在，有限範圍 CONFIRMED。7 個記憶體 mutation（追加輸入、缺 completion、改 generation／尺寸、未知選單、P5 過度宣稱、secret）均拒絕。自有 verifier worktree／branch／cache 已移除，沒有重跑真 CLI。

原始 ACK wire 與 monotonic dwell 時戳未保留，不能重演完整身分核對或一秒等待；raw cleanup identity 已按計畫刪除，hash 不能還原 SID／canonical path 綁定或歷史 atomic config rewrite。保留 manifest 與現場 absence 支持有限結論，不認證正式 P5／P6、完整 12A、初始 idle 或先前 Down 無效原因。兩寬均有更新已安裝 banner；固定 executable hash 未變，共用更新未擅自清除。

必要證據與唯讀 `audit.py --self-test` 留在 `AgEnD-ops/g12a-native-checkpoint-20261005/true-ready-capture-fresh-verifier/`；[重核指令](../../crates/agend-daemon/CLAUDE-TESTING.md#已核准的真蒐證重核唯讀)不啟動 backend／模型、不寫檔／送鍵。

## 下一步

目前工具與真執行限制見 [startup capture](gate-12a-startup-capture.md)，失敗後的修正見 [輸入時機](gate-12a-startup-input-timing.md)。
