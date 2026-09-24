# agend-testkit 測試

> **TL;DR**
> - 目前只測 `TempDir`。
> - 記住：fixture 只碰自己建的暫存目錄。
> - 下一步：第 2 關：契約測試通過、假 agent 可單獨啟動並回應。

## 怎麼跑

```bash
cargo test -p agend-testkit
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `tempdir::tests` | 兩個 `TempDir` 路徑不同；drop 後目錄被刪除 |

## 用到的假實作

- 不適用（這裡就是假實作的家）

## 還沒測的

- [ ] 假實作、契約測試、假 daemon、假 agent、git fixture（第 2 關）

## 下一步

```bash
cargo test -p agend-testkit
```
