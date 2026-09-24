# agend-daemon 測試

> **TL;DR**
> - 目前只有 codex socket 路徑解析的測試。
> - 記住：每個領域模組都要能對 testkit 的假實作單獨測；每個 adapter 要跑契約測試。
> - 下一步：第 5 施工關加入 store 的 in-memory SQLite 測試。

## 怎麼跑

```bash
cargo test -p agend-daemon
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `driver::codex::tests` | 長 listen 路徑的 symlink 會被解析到真正的 socket；不存在的路徑回錯誤，不猜 |

## 用到的假實作

- `agend_testkit::tempdir::TempDir`（暫存目錄）

## 還沒測的

- [ ] store：migration、交易、保留期限、`VACUUM INTO`（第 5 施工關）
- [ ] agent runtime 與真 holder（第 6 施工關）
- [ ] codex driver 對假 app-server、delivery 的冪等與重連不遺失不重複（第 7 施工關）
- [ ] protocol server（第 8 施工關）
- [ ] pipeline、git、runner、forge local、supervisor、reconcile（第 10 施工關）
- [ ] claude／opencode driver、forge github、notifier（第 12 施工關）

## 下一步

```bash
cargo test -p agend-daemon
```
