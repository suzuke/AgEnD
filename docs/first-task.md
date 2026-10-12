# 第一個任務：讓 agent 建立 README

> **先看這三行**
> - 先完成 [啟動教學](quickstart.md)，保持 daemon 執行、`try-opencode` 已加入。
> - 這次只操作新建的試用 Git repository，模型會使用工具並可能產生費用。
> - 流程是：建立檔案 → 檢查非空 → 你確認 → 本機合併。

## 1 · 準備試用 repo

在終端 B 離開管理介面（`q`），回到下載的 `agend-v2` 原始碼根目錄。確認仍有同一個 `AGEND_HOME`：

```sh
export PATH="$HOME/.local/bin:$PATH"
export AGEND_HOME="$HOME/agend-trial"
trial_repo=$(mktemp -d "$HOME/agend-first-task.XXXXXX")
git -C "$trial_repo" init -b main
git -C "$trial_repo" config user.name "AgEnD Trial"
git -C "$trial_repo" config user.email "trial@localhost"
git -C "$trial_repo" commit --allow-empty -m "Start trial"
printf '試用 repo：%s\n' "$trial_repo"
```

**確認：** 出現 `Start trial` 的提交，並印出新的 repo 路徑。Git 名稱只設定在這個 repo。

## 2 · 套用範例流程

仍在 `agend-v2` 原始碼根目錄執行：

```sh
agend workflow check examples/first-task.toml
```

**確認：** 檢查成功後，才套用：

```sh
agend workflow apply examples/first-task.toml
agend team add trial --repo "$trial_repo" --workflow first-task
agend team join trial try-opencode --role dev
```

這份流程使用本機 Git，**會在合併前等你確認**。每次重做請使用新的團隊名稱；既有 workflow／team 不要重複建立。

## 3 · 建立任務

```sh
agend task create --team trial --role dev "建立試用 README"
agend app --lang zh-TW
```

**確認：** CLI 印出 task ID，介面開始顯示工作進度。任務會在 daemon 建立的獨立 worktree 裡執行。

如果「需要你」出現目錄權限請求，先讀完整路徑，確認屬於這次試用 worktree 再允許。

## 4 · 檢查結果，再決定合併

按 `!` 開啟待辦。等到審查項目出現後，閱讀任務結果與待合併提交。

這份範例只檢查 README 非空，不代表內容正確。可以另開終端，使用待辦顯示的完整 head 檢查實際檔案：

```sh
git -C /剛才印出的試用repo路徑 show 完整head:README.md
```

滿意才在待辦選擇核准；需要修改則選擇退回並填理由。

## 5 · 確認完成

按 `/` 搜尋步驟 3 的 task ID，按 Enter 開啟任務詳情；確認狀態為「已完成」，再按 `q` 返回終端 B，保留原本的 `trial_repo` 變數：

```sh
git -C "$trial_repo" log -2 --oneline
cat "$trial_repo/README.md"
```

**確認：** 任務顯示完成、main 出現合併提交、README 已存在。若尚在等待或受阻，回到「需要你」看原因。

## 下一步

[停止試用](quickstart.md#操作速查)；或參考 [workflow 與團隊命令](architecture/pipeline-runtime.md#命令與協定) 增加 reviewer、換成專案的真正 checks。試用 repo 保留在先前印出的路徑，確認不需要後再自行刪除。
