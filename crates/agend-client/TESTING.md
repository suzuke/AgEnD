# agend-client 測試

> **TL;DR**
> - 目前只有一個釘住常數的測試。
> - 記住：之後的測試對 testkit 的假 daemon 跑，不需要真 daemon。
> - 下一步：第 8 施工關的重試與版本不符測試。

## 怎麼跑

```bash
cargo test -p agend-client
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `retry::tests::retry_window_matches_the_plan` | 重試視窗是規劃 §4.7 定的 10 秒（只釘數值，不是行為測試） |

## 用到的假實作

- 目前無；第 8 施工關用 `agend_testkit::fake_daemon`

## 還沒測的

- [ ] 連線、重試行為、版本不符（第 8 施工關）

## 下一步

```bash
cargo test -p agend-client
```
