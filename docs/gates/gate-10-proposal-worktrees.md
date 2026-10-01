# Gate 10 提案：worktree、Git 與 checks

> **TL;DR**
> - P4–P6 定義 binding、WIP 保存與 checks 沙箱。
> - 狀態與驗證證據見 [Gate 10 入口](gate-10-pipeline.md)。
> - 下一步：閱讀 [merge／attention 提案](gate-10-proposal-merge.md)。

### P4：worktree、binding、hook、binding 快照

- 問題：worktree 什麼時候建、建在哪？hook 什麼時候裝、怎麼裝（daemon 不能依賴 `agend-shim`）？binding 快照誰寫、什麼時候寫？審查的 agent 在哪看程式？task 結束時留著的修改怎麼辦？
- 建議：
  - 綁定（work 關卡第一次派給持有者時）：
    1. DB 的 `bindings` 表寫一列 `pending`（instance、task、kind、worktree、branch）。
    2. 在 canonical checkout 跑 `git worktree add -b agend/<task>/<slug> $AGEND_HOME/worktrees/<task>/ <main 的 SHA>`。
    3. 裝 hook：daemon 跑 `agend hooks install <worktree>`（`agend` 的內部子命令，呼叫 `agend_shim::install_hooks`；不在 `--help` 列出）。
    4. 寫 binding 快照（先寫暫存檔再 rename、0444）。
    5. `bindings` 標 `ready`，然後才送派工訊息。
  - 每一步都可重做：死在中間，P9 從 `pending` 接著做；worktree 已存在就檢查它是不是這個 branch。
  - binding 快照的型別從 `agend_shim::binding::Snapshot` 搬到 core（第 3 施工關 T1 已建議）：路徑改成 `String`（core 是 no_std），shim 與 daemon 共用同一個型別；golden JSON 測試證明格式沒變。這也是 D32 的延伸，跟 P2 一起編進 [D38](../decisions/d38.md#d38)。`bindings` 表的列在釋放時刪除。
  - 審查（`approval(by = <角色>)`）：reviewer 拿到 detached 的審查 worktree `$AGEND_HOME/worktrees/<task>-review/`（在審的 head），binding kind `review`，一樣裝 hook（審查 binding 不能寫任何 branch）。核准或要求修改後就拆掉。`approval(by = "human")` 沒有 worktree，走「需要你」（P8）。
  - 釋放（task done、失敗、取消；或審查結束）：
    1. binding 快照先改成沒有 binding（shim 從這一刻起拒絕寫入）。
    2. 從 blob／mode 重建沒有 stat cache 與隱藏旗標的私有 index，分別讀取 staged、unstaged 修改與未追蹤檔（index 專有內容也須保存；未解衝突時保留原 worktree／index 並回報 Failed）→ 存成 `archive/<task>-<unix-ms>.patch`；task 沒 merge（失敗、取消）時，branch 的 first-parent commits 也存進同一個檔：一般 commit 用完整 binary `format-patch`；merge commit 保存 metadata 與對第一個 parent 的 diff（`format-patch` 會省略 merge，單用它會漏掉衝突解法）。保留 30 天（D31 的 WIP patch）。 所有封存 diff 固定無色、a/／b/ prefix 與完整 repo 路徑，不受顯示設定影響；保存前走訪 filesystem，nested Git metadata（含缺 HEAD 的不完整狀態）／gitlink／特殊檔案無法由 patch 完整保存時，保留原 worktree／index 並回報 Failed。若有 content filter、working-tree-encoding、ident、text／eol／crlf attributes 或 core.autocrlf=true／input，先保留原 worktree／index 並回報 Failed，交由操作者處理轉換設定後重試。
    3. `agend hooks uninstall <worktree>` → `git worktree remove --force` → `git branch -D`。
    4. 刪掉 `bindings` 那一列；patch 路徑記在 task 事件，`agend status` 與 TUI 看得到。
- 理由：hook 是 protected ref 的硬保證（第 3 施工關威脅模型），一定要在 agent 拿到 worktree **之前**裝好；經子命令呼叫，daemon 就不必連結 shim（第 6 施工關 P9 的依賴規則）。一個型別兩邊用，格式不會漂移（#1493）。審查者看的是固定的 head，不是持有者正在改的 worktree。
- 替代方案：daemon 直接呼叫 `install_hooks`（違反依賴規則）；hook 在 agent 第一次跑 git 時由 shim 自己裝（agent 可以繞過 shim）；失敗或取消的 branch 留著不刪（v1 的 137 個 branch）；審查者直接讀持有者的 worktree（審的不一定是那個 head）；本關不做 agent 審查、只有人工核准（`code` workflow 跑不了，`review` 命令沒有用處）。
- 例子：task `t-3` 派給 `g10-dev` → `worktrees/t-3/` 出現、branch `agend/t-3/hello`、`bindings/g10-dev.json` 有 `{"kind":"work","task_id":"t-3",…}`；在那個 worktree 跑 `git update-ref refs/heads/main HEAD` → `agend-shim: refused … (agend reference-transaction hook)`。task merge 後三樣都不見，`bindings/g10-dev.json` 只剩 instance 與 repo。
- [x] 使用者確認（2026-09-26）

### P5：daemon 自己跑的 git

- 問題：daemon 也要跑 git（建 worktree、算 patch-id、merge、看 head）。會不會被 shim 或 hook 擋？會不會觸發專案自己的 hook？卡住怎麼辦？daemon 會不會改 agent 的 worktree？
- 建議：
  - daemon 開機時找一次真的 git：`PATH` 上第一個**不在** `$AGEND_HOME/bin` 的 `git`，記住絕對路徑；版本低於 2.38（`merge-tree --write-tree` 需要）就不處理有 repo 的 team，log 指出原因（`agend doctor` 由第 9 施工關檢查）。
  - 每個 git 呼叫：`env_clear` 後只給 `HOME`、`PATH`、`LANG=C`、`GIT_TERMINAL_PROMPT=0`（沒有 `AGEND_*`、沒有 daemon 的 `GIT_*`）；一律加 `-c core.hooksPath=/dev/null`（不跑 agend hook，也不跑專案的 hook）；碰到 checks worktree 的（建立、`worktree remove --force` 清掉）再加 `-c core.fsmonitor=false`，所以 checks 留下的設定不會讓 daemon 執行任何東西（P6）；經 `Runner`，timeout 60 秒。
  - 在 canonical checkout 跑：`worktree add/remove/list`、`branch -D`、`rev-parse`、`merge-base`、`diff`／`patch-id`、`log`、`merge-tree`、`commit-tree`、`update-ref`。
  - 在 agent 的 worktree 裡：讀（`status`、`diff`，給 P4 的 WIP patch 用）；以及 P7 的 rebase——**這會在持有者的 worktree 裡產生新的 commit、改寫它的 branch**，所以只在 task 不在 work 關卡、worktree 乾淨時做，不乾淨就不做、當成衝突。除此之外 daemon 不在 agent 的 worktree 裡寫任何東西。
  - 什麼時候看 branch 的 head：沒有輪詢。只在「收到任何結果事件時」與「送出 `Merge` 前」讀一次；和狀態裡的 head 不同就先餵 `CommitCreated`（附新的 patch-id），再處理原本的事件。
- 理由：第 3 施工關已定「可信的呼叫者用 `-c core.hooksPath=/dev/null`」；daemon 沒有 agent 身分，不關 hook 會被自己的 hook 擋。專案 hook（例如 husky 的 `post-checkout` 跑 `npm install`）在 `worktree add` 時跑，會又慢又不可預期。每個外部指令都有 timeout（V1-LESSONS #10）。
- 替代方案：用 `AGEND_SHIM_BYPASS=1` 跑 PATH 上的 git（只跳 shim，hook 照樣擋）；保留專案 hook（`worktree add` 可能跑幾分鐘）；定時輪詢每個 branch 的 head（多數時間白跑）。
- 例子：agent `done` 之後又 commit 了一次才被 checks 跑到：checks 結果回來時 daemon 發現 head 變了 → log `t-3: head moved a1b2… -> c3d4…; checks run again`，舊的結果不算。
- [x] 使用者確認（2026-09-26）

### P6：`command` 關卡（runner）

- 問題：checks 在哪跑？用什麼環境？跑多久算逾時？輸出存哪？daemon 被硬殺時正在跑的 check 怎麼辦？
- 建議：
  - **每次跑一個新的** detached worktree：`$AGEND_HOME/checks/<task>-<stage>-<attempt>-<unix 毫秒>/`，在要測的 head；跑完（不管結果）就刪。不裝 hook、沒有 binding。
  - 指令：`sh -c <已展開的指令>`，自己一個 process group（RUN-8），stdin 是 `/dev/null`。環境：第 6 施工關的 agent 白名單，但**沒有** `AGEND_*`，`PATH` 也拿掉 `$AGEND_HOME/bin`。
  - timeout：`RunCommand` 帶來的 `timeout_ms`（關卡沒寫就是 core 的預設 5 分鐘，`DEFAULT_STAGE_TIMEOUT_MS`）。逾時 → 停掉整個 process group → 餵 `CommandFinished{exit_code: None}`：core 當成 checks 失敗、退回 work，持有者會收到「checks timed out after N s」。**不餵 `StageTimedOut`**：預設逾時動作是「通知」，只記一筆，task 會一直停在 checks、「需要你」也沒有。所以 `command` 關卡的 `on_timeout` 沒有作用：寫了 `on_timeout` 的 `command` 關卡在建立 task 時被拒（跟 `reassign` 一樣），訊息寫「command 逾時一律當失敗、退回 work」。
  - 每次都是冷的 worktree，5 分鐘可能不夠：workflow 要自己寫 `timeout_ms`。本關的 `demo` workflow 寫 60 秒（checks 是 `test -f`），`slow` 寫 120 秒；Rust 專案每次都要重新下載依賴、從頭編譯（沒有共用快取，見下），建議至少 30 分鐘。
  - 輸出：完整 stdout／stderr 存 `logs/checks/<task>/<stage>-<attempt>.log`（每個檔最多 10 MiB，超過截斷並註明）；task 事件只放最後 20 行。保留 14 天（跟事件一樣，列進第 5 施工關 P8 的規則表）。
  - 同時最多跑 1 個 check，其他排隊（log 寫出在等誰）。
  - daemon 被硬殺時正在跑的 check 會變孤兒、跑到自己結束，結果沒人收；重開後 P9 對同一個 attempt 在**另一個新目錄**重跑，兩者不共用檔案；舊目錄由對帳刪掉。
  - **寫入沙箱**（使用者 2026-09-26 決定）。checks 跑的是 agent 寫的程式碼：指令是你寫進 workflow 的（例如 `cargo test`），但它會執行 agent 改過的測試、`build.rs`、腳本。所以整個指令在沙箱裡跑，**IP 網路（TCP、UDP、HTTPS）照常可用**，限制兩件事：寫入、unix socket 連線。
    - 可寫的只有兩個地方：這次的 checks worktree（但它的 `.git` 檔案本身唯讀）；這次的暫存目錄 `$AGEND_HOME/checks/<run>.tmp/`。再加 `/dev/null`、`/dev/tty` 這類裝置。
    - 暫存目錄設成 `TMPDIR`，也放每次自己的快取：`CARGO_HOME`、`CARGO_TARGET_DIR`、`XDG_CACHE_HOME` 都指到它底下，跑完一起刪。**沒有跨 checks 共用的快取**：每次重新下載依賴、從頭編譯，比較慢；換來的是一次 check 留下的東西（例如在 `CARGO_HOME/config.toml` 設 `runner = "true"`）不可能讓之後別的 task 的 check 假綠。
    - **不能連任何 unix socket（預設拒絕）**：check 沒有 `AGEND_INSTANCE`，照第 8 施工關 P2 連上 `run/daemon.sock` 會被當成操作者（能核准 merge、`task_cancel`、重啟 daemon）；連上 holder 的 socket 能在 agent 的 PTY 打字；連上 codex app-server（第 7 施工關，`run/holders/<id>.codex.sock` 是指到 `/private/tmp/codex-daemon-<uid>/…` 的 symlink）能讓 codex 不經沙箱執行命令。列路徑擋已經漏了三輪，所以改成**一律不准連 unix socket**，只有一個例外：macOS 的 DNS（`/private/var/run/mDNSResponder`，不開就查不到網址、HTTPS 也失敗；2026-09-26 在本機實測）。
    - 結果：需要 unix socket 的 checks（docker、ssh-agent、gpg-agent 之類）在本關跑不了，是已知的限制。
    - 每次 check 結束（不管結果、不只逾時）都停掉它的整個 process group。Linux 另有 pid namespace，沙箱裡的程序全部跟著結束；macOS 上用 `setsid` 另開 process group 的子程序停不到，但它仍在沙箱裡：寫不了外面、連不了任何 unix socket（見已知風險）。
    - 其他全部唯讀：你的 home（含 `~/.ssh`）、其他 repo、canonical repo 的 `.git`（refs、objects、`.git/worktrees/<run>/` 這個 worktree 的 git 目錄都唯讀：所以測試改不了 main，也改不了 `commondir` 把 daemon 之後跑的 git 導到假的 repo）、`$AGEND_HOME` 的其他地方。
    - 因此 checks 裡的 `git status`、`git diff` 照常可用；`git commit`、`git stash` 這類要寫 objects 或 index 的會失敗——checks 本來就不該改 repo，接受。
    - macOS 的 `mktemp` 不帶 `-t` 時不看 `TMPDIR`、寫到 `/var/folders/…/T`（`DARWIN_USER_TEMP_DIR`）：**不開放**那裡（別的程式也用它）。`mktemp -t <名字>`、`mktemp "$TMPDIR/x.XXXX"`、Rust 的 `std::env::temp_dir()` 都照 `TMPDIR`，可以用；不帶 `-t` 的 `mktemp` 會得到 `Operation not permitted`（寫進已知風險）。
    - macOS：`/usr/bin/sandbox-exec -f <profile>`，profile 由 daemon 每次產生：`(allow default)`、`(deny file-write*)`，再對上面兩個目錄 `(allow file-write* (subpath …))`、對 `<worktree>/.git` `(deny file-write* (literal …))`，再加 `(deny network-outbound (remote unix-socket))`（不帶路徑＝全部）與唯一的例外 `(allow network-outbound (remote unix-socket (path-literal "/private/var/run/mDNSResponder")))`。這個寫法 2026-09-26 在本機實測：不帶路徑的形式系統內建的 profile 裡沒有範例（它們只用 `path-literal`），但 `sandbox-exec` 接受；直接連與經 symlink 連 `/private/tmp` 下的 socket 都得到 `Operation not permitted`，`curl https://example.com` 回 200（沒有 mDNSResponder 例外時 curl 查不到網址）。profile 裡的路徑一律先轉成真實路徑（`/tmp` 是 `/private/tmp`），否則規則對不上。
    - Linux：`bwrap --ro-bind / / --dev /dev --proc /proc --bind <worktree> <worktree> --ro-bind <worktree>/.git <worktree>/.git --tmpfs /tmp --bind <tmp> <tmp> --tmpfs /run/user/<uid> --tmpfs <AGEND_HOME>/run --unshare-pid --die-with-parent -- sh -c <指令>`（`<tmp>` 在 `$AGEND_HOME/checks/` 底下，不受 `/tmp` 換掉影響）。Linux 沒有「擋所有 unix socket 連線、保留 IP 網路」的簡單開關（`--unshare-net` 會連 HTTPS 一起擋掉），所以用 `--tmpfs` 把放 socket 的目錄換成空的：`/tmp`（含 `/tmp/codex-daemon-*`）、`/run/user/<uid>`（使用者的 ssh-agent、gpg-agent、dbus）、`$AGEND_HOME/run`。**其他看得到的 socket 仍連得上**（例如 `/var/run/docker.sock`、`/run/systemd/…`），列為已知風險；abstract namespace 的 socket（不在檔案系統上）也一樣。
    - **fail closed，兩層**：
      1. 開機時與每次 `retry` 時：找沙箱工具並試跑一次 `true`；找不到或試跑失敗 → checks 不跑。
      2. 每一次 check：沙箱裡第一件事是寫一個標記檔 `$TMPDIR/.agend-sandbox-started`，再 `exec` 真的指令。結束後 daemon 只 `lstat` 這個檔（必須是一般檔案；**絕不打開它**，check 可能把它換成 FIFO 讓 daemon 卡住）。沒有這個標記檔時，daemon 當場再試跑一次 `true`：試跑失敗 → 是沙箱自己沒起來（profile 錯、工具出錯）；試跑成功 → 是 check 自己刪掉或換掉了標記檔，算 **check 失敗**（退回 work），不算 `sandbox-missing`，也就不會一直 `retry` 迴圈。
      沙箱沒起來的兩種情況一樣處理：task 留在 checks 關卡、不餵任何結果，出現「需要你」`sandbox-missing:<task>`（P8），按 `retry` 再試。`agend doctor` 的沙箱工具那一列由本關加（見上面的分工表）。不管哪一種，check 都不可能因此變成通過。
- 理由：agent 寫的程式碼不該能改你的 home、其他 repo 或 main；只擋寫入、不擋網路，一般的 `cargo test`、`npm test` 照常能下載依賴。新的 worktree 就在要測的那個 head 上，沒有上一次留下的檔案，所以「merge 出來的樹＝測過的樹」（P7）；沒有共用快取，一次 check 影響不到下一次；跟 agent 的 worktree 分開（`runner.rs` 的 Must NOT、[pipeline](../architecture/pipeline.md#6-種關卡)「在 head 的臨時 detached worktree 執行」）；孤兒 check 不會跟重跑的撞在同一個目錄。一次一個最簡單，也不會讓兩個 `cargo test` 搶 CPU 互相逾時。
- 替代方案：每個 task 重複用一個 checks worktree、保留 ignored 檔（增量編譯快，但上一次的產物可能讓測試假綠，也會跟孤兒撞目錄）；在 agent 的 worktree 跑（agent 可能正在改）；同時跑多個（要設上限與排序）；不做沙箱（原本的建議，使用者改成沙箱）；容器（要裝 Docker，太重）；沒有沙箱工具時照跑、只警告（等於沒有保護）；沒有沙箱工具時讓 checks 失敗（會退回 work，agent 白做一輪，所以改成「需要你」）；每個 team 一個可寫的共用快取（快很多，但 review 在 macOS 重現了：一次 check 在 `CARGO_HOME/config.toml` 設 `runner = "true"`，之後別的 task 失敗的 `cargo test` 變成通過）；共用的下載快取在沙箱裡唯讀、由沙箱外另一步先下載（`cargo fetch` 本身會跑依賴的程式碼，要再想一層，先不做）；開放 `.git/worktrees/<run>/` 讓 checks 能 commit（review 重現了改 `commondir` 讓 daemon 清理時執行 `core.fsmonitor` 的逃逸）；checks worktree 也裝 agend hook（沙箱已經讓 refs 唯讀，不需要）。
- 例子：checks 是 `test -f hello.txt`，第一次 agent 忘了加檔 → exit 1 → `t-3: checks t-3/checks/1 failed (exit 1); back to work (g10-dev)`；agent 補上再 `done` → `checks t-3/checks/2 passed`。
- [x] 使用者確認（2026-09-26；改成寫入沙箱，見上）

## 下一步

閱讀 [merge／attention 提案](gate-10-proposal-merge.md)。
