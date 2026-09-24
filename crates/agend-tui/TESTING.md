# agend-tui 測試

> **TL;DR**
> - 目前只測語言切換。
> - 記住：畫面測試用 ratatui `TestBackend` snapshot，餵假 protocol 事件。
> - 下一步：第 11 關。

## 怎麼跑

```bash
cargo test -p agend-tui
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `i18n::tests` | `L` 在英文與繁中之間切換，切兩次回原語言 |

## 用到的假實作

- 目前無；第 11 關用 `agend_testkit::fake_daemon` 的事件

## 還沒測的

- [ ] 所有畫面與按鍵（第 11 關）

## 下一步

```bash
cargo test -p agend-tui
```
