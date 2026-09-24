# agend-holder 測試

> **TL;DR**
> - 目前只測控制鍵的位元組。
> - 記住：holder 測試用 `sh`、`cat` 這類假程式取代 agent。
> - 下一步：第 4 關的探測 client 與斷線重連測試。

## 怎麼跑

```bash
cargo test -p agend-holder
```

## 測試分類

| 測試 | 證明什麼 |
|---|---|
| `pty::tests` | `Esc` 送出的正好是 1 個位元組 0x1b |

## 用到的假實作

- 目前無；第 4 關會用 testkit 的假 agent 程式

## 還沒測的

- [ ] PTY 讀寫、畫面快照、resize、exit code
- [ ] daemon 斷線重連、bash 存活
- [ ] 附屬程序持有
- [ ] 協定版本協商

## 下一步

```bash
cargo test -p agend-holder
```
