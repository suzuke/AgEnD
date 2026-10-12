# README 介面預覽

`tui-overview.svg` 來自原生 Ratatui `TestBackend` 的 140 欄 × 34 列總覽，以 scripted demo 資料呈現；不是瀏覽器 App 示意圖。

來源為 `crates/agend-tui/examples/tui_preview.rs` 匯出的第一個 HTML section。每一格的座標、文字、前景／背景色與粗體轉成 SVG；格寬 9 px、格高 22 px，viewBox 為 1260 × 748。

更新畫面時，在原始碼根目錄執行：

```sh
cargo run --locked -p agend-tui --example tui_preview > /tmp/agend-native-cells.html
```

將第一個 section 轉成同尺寸 SVG，並在 README 預覽中確認中文字寬、對齊與顏色。資料是示範內容，不代表目前有這些任務或驗收結果。
