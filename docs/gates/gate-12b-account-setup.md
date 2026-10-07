# OpenCode instance 的私人帳戶設定

> **TL;DR**
> - 每個 OpenCode instance 有獨立 XDG 目錄；不會自動讀取你的共享 `auth.json`。
> - 建立 instance 前，先在該私人目錄登入 provider；daemon 不複製或改寫共享憑證。
> - 目前正式 push driver 固定支援 OpenCode 1.18.34；登入完成後再加入 instance。

## 登入與加入

先設定既有 AgEnD home、instance 名稱及固定 binary。以下的 `open-dev` 是範例名稱；不要指向別人的 instance 或共用目錄。

```sh
: "${AGEND_HOME:?先設定要使用的 AGEND_HOME 絕對路徑}"
open_instance=open-dev
open_program=/opt/homebrew/Cellar/opencode/1.18.34/bin/opencode
open_private="$AGEND_HOME/opencode/$open_instance"
umask 077
mkdir -p "$open_private/data" "$open_private/config" \
  "$open_private/cache" "$open_private/state"

XDG_DATA_HOME="$open_private/data" \
XDG_CONFIG_HOME="$open_private/config" \
XDG_CACHE_HOME="$open_private/cache" \
XDG_STATE_HOME="$open_private/state" \
  "$open_program" auth login --pure
```

依 OpenCode 的選單完成 provider 登入。這個命令不呼叫模型；provider 可能開啟瀏覽器登入或要求 API key。不要將 key 貼到訊息、命令歷史或 Git。憑證保存在該 instance 私人 data 目錄。

daemon 運行時，再加入 instance；`provider/model` 換成剛登入帳戶可用的模型：

```sh
agend instance add "$open_instance" opencode \
  --program "$open_program" -- --model provider/model
```

若已有共享 OpenCode 帳戶，可自行選擇把其 `auth.json` 複製到 `$open_private/data/opencode/auth.json`，設為 mode `0600`；這不是 daemon 的預設行為。保留原檔，不用移動、連結或刪除共享帳戶。登入應在 instance 啟動前完成，避免運行中的 backend 繼續使用先前載入的帳戶狀態。

## 隔離範圍

serve 與 attach 都使用同一組私人 XDG 路徑；session 歷史保留在這裡供重啟恢復。移除 instance 不代表可以刪除其他 OpenCode 程序、使用者帳戶或工作目錄。測試清理只針對已停止、可確認歸屬的測試 namespace。

`auth login --help` 的上述語法已對固定 1.18.34 binary 核對。真模型驗證使用私人憑證副本，並在前後比對共享原檔雜湊；互動登入本身未由自動 smoke 代替使用者操作。

## 下一步

回到 [12B](gate-12b-opencode.md) 查看目前驗收範圍。
