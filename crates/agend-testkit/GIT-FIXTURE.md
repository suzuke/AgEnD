# Git fixture

> **TL;DR**
> - GitFixture 建立獨立 canonical repo、bare origin 與 linked worktree。
> - 指令只允許 fixture 自己的 repo，清除繼承的 git／AgEnD 環境；這是測試衛生，不是沙箱。
> - 下一步：用 `GitFixture::new` 建 fixture，指令走 `git` 或 `command`。

## git fixture

`GitFixture::new(label)` 在 `<tmp>/agend-test-git-<label>-*` 建出（`label` 含 `:` 或 `;`、或暫存目錄含 `:`（Windows 是 `;`）時回 `InvalidInput`，什麼都不建：git 拿這個字元切 `GIT_CEILING_DIRECTORIES`，路徑含了就等於沒有 ceiling）：

| 路徑 | 內容 |
|---|---|
| `canonical()` | repo，`main` 上有一個初始 commit，`origin` 指向下面的 bare repo，`main` 已 push |
| `origin()` | bare 的 team origin（`origin.git`） |
| `add_worktree(name, branch, from)` | `worktrees/<name>`：canonical 的 linked worktree，在新 branch 上；`name` 是 `.git`（不分大小寫）就 panic，不建任何東西 |

其他：`branch(name, from)`、`commit(dir, file, message)`（回新的 head）、`rev_parse`、`is_ancestor`、`git(dir, args)`（失敗就 panic 並印 stderr）、`command(program, dir)`（給要自己跑程式的測試，例如第 3 施工關的 shim）。drop 時整個目錄刪掉。

威脅模型：fixture 防的是**善意但會出錯**的測試程式碼：路徑寫錯或是空的、`cd` 失敗、繼承到外面的環境變數。它**不是沙箱**，不防故意改 git 內部檔案來逃出去的測試程式碼（例如透過 `command` 自己寫 `.git/commondir` 或 `core.worktree`）；這不在範圍內。理由跟第 3 施工關的 shim 一樣：要擋的是會發生的失誤，不是對抗性的程式碼；每補一個洞就冒出下一個，擋不完，只會讓 fixture 越來越複雜。只支援 unix（專案的平台）；Windows 的檔名規則（結尾的 `.`、8.3 短檔名）不處理。

衛生規則（每個 command 都套用）：

- **只在自己建的 repo 裡跑**：`dir` 必須是絕對路徑，解析後落在 `canonical()`、`origin()` 或 `add_worktree` 建的 worktree（含子目錄）裡，否則 panic；解析後任何一段是 `.git`（不分大小寫，例如 `canonical/.git`、`canonical/.git/worktrees/<w>`）也 panic，在那裡跑 git 或寫檔等於改 repo 自己的 metadata。`root()`、`root/worktrees` 與 `root()` 下其他目錄都拒絕：它們不是 repo，git 從那裡找 repo 會往上走，所以 discovery 一律從 fixture 的 repo 開始。`GIT_CEILING_DIRECTORIES=<root>` 是第二道防線（repo 的 `.git` 被刪掉時擋住往上找）。
- `git(dir, args)` 的第一個參數必須是子指令；`-C`、`--git-dir`、`--work-tree`、`--namespace`、`-c` 等全域選項一律 panic（repo 用 `dir` 指定）。
- `commit` 的 `dir` 只能是 canonical 或 linked worktree（不收 bare origin）；檔名只能是單純的相對路徑，任何一段是 `.git`（不分大小寫）或 symlink 就 panic，檔案已存在且 hard link 數大於 1 也 panic（unix），寫檔前確認上層目錄解析後仍在 work tree 裡；所有檢查都在寫檔之前，被拒絕的呼叫不留下任何檔案。
- **不檢查的**：子指令後面的路徑參數（例如 `worktree add <path>`），以及 `command(program, dir)` 除了 `dir` 以外的參數；測試要自己只傳 fixture 裡的路徑。
- 一律 `Command::current_dir(<絕對路徑>)`，不用 process 的 cwd；`root()` 已解析 symlink（macOS 的 `/var` → `/private/var`）。
- `GIT_CONFIG_GLOBAL=/dev/null`、`GIT_CONFIG_NOSYSTEM=1`、`GIT_CEILING_DIRECTORIES=<root>`；固定 author／committer 與日期，所以同樣的內容、parent、訊息得到同樣的 commit id（不同 commit 請用不同訊息）。
- 移除繼承來的 `GIT_*`、`AGEND_*`（含 `GIT_DIR`、`GIT_WORK_TREE`、`AGEND_HOME`）。這是 `command()` 當下的快照；之後才在父程序設定的變數，只有固定清單（`GIT_DIR`、`GIT_WORK_TREE`、`GIT_INDEX_FILE`、`GIT_COMMON_DIR`、`GIT_OBJECT_DIRECTORY`、`GIT_ALTERNATE_OBJECT_DIRECTORIES`、`GIT_NAMESPACE`、`GIT_CONFIG`、`GIT_CONFIG_PARAMETERS`、`GIT_CONFIG_COUNT`、`GIT_CONFIG_SYSTEM`、`GIT_DISCOVERY_ACROSS_FILESYSTEM`、`AGEND_HOME`）會被移除，其他的仍會傳給 git。

## 下一步

```bash
cargo test -p agend-testkit --test git_fixture
```
