# agend-core 測試

> **TL;DR**
> - 只有純函式單元測試；不需要任何其他 crate 或程序。
> - 記住：測 consumer 時用 producer 產生輸入（例：`task_id_of_branch` 吃 `work_branch` 的輸出）。
> - 下一步：第 1 關加入 property test 與 `code` workflow 模擬。

## 怎麼跑

```bash
cargo test -p agend-core
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `model::tests` | backend 名稱往返；送達狀態只允許 `queued→sent→confirmed|failed`；branch 命名空間內外判斷 |
| `pipeline::stage::tests` | 6 種 kind 往返；`checks` 不是合法 kind（D19／D20） |
| `policy::busy::tests` | claude／opencode 的插入退成中斷；排隊與中斷各 backend 都支援 |

## 用到的假實作

- 無。core 不用 testkit（testkit 依賴 core，反過來會循環）。

## 還沒測的

- [ ] trait 與協定（尚未定義）
- [ ] pipeline 狀態機、workflow 存檔檢查、merge 門檻、去抖動、衝突偵測、分派規則、螢幕分類器（第 1 關）
- [ ] property test（規劃 §5.1）

## 下一步

```bash
cargo test -p agend-core
```
