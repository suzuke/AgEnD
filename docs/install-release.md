# 安裝與發布產物

> **TL;DR**
> - 第 13 關施工中；Actions 產物供驗收，尚未公開發布。
> - release archive 與從固定 checkout 執行 `cargo install --path` 是兩條安裝路徑。
> - 安裝 executable 不會替你註冊服務、建立 agent 或修改 Claude trust entries。

## 從原始碼安裝

先切到經審閱的固定提交；使用 `rust-toolchain.toml` 的工具鏈。在 repo 根目錄執行：

```sh
cargo install --locked --path crates/agend --root "$HOME/.local"
"$HOME/.local/bin/agend" --version
```

需要命令列直接找到 agend 時，再把 `$HOME/.local/bin` 加入自己的 PATH。Cargo 預設會拒絕覆寫既有其他來源的安裝；先核對既有版本與服務使用的 executable，再決定更新。不要把開發 worktree 的編譯產物直接覆蓋到正在使用的私有 launcher。

目前 workspace 設定 `publish = false`，**尚不提供 crates.io 的 `cargo install agend`**。上述命令從完整 checkout 建置本地 workspace 依賴，`--locked` 固定 Cargo.lock。

## 原生 archive

`cargo xtask release --out /absolute/new/directory` 產生：

- `agend-<version>-<target>.tar.gz`：單一根目錄內的 agend、README.md、LICENSE。
- `manifest.json`：來源提交、target、版本、binary 與 archive SHA-256。
- `SHA256SUMS`：archive 的 checksum。

在對應原生平台執行以下驗證，再解壓安裝：

```sh
python3 -B scripts/verify_release.py \
  --directory /absolute/artifact-directory \
  --commit <expected-source-SHA> \
  --target <expected-native-target>
```

expected SHA 應從審閱提交／Actions run 取得。雜湊檔與 archive 放在一起只提供一致性檢查，不是來源簽章。驗證器核精確 archive 內容、雜湊與 executable 權限，使用隔離 HOME 執行解壓後的 `--version`，完成後刪除自有暫存。

## Actions 與 Brew

`release-artifacts` workflow 在 macOS Intel／ARM64 與 Linux Intel／ARM64 原生建置。每份 archive 經驗證與 fake-worker 首任務 smoke，全部成功後才產生 `agend.rb`。Actions artifacts 保留 14 天；沒有建立 tag、GitHub Release 或更新 Brew tap。

formula generator 要求四份同提交／同版本的產物，重新核對 archive，URL 指向 `v<version>` Release。因此產生 formula **不代表 URL 已存在，也不代表已完成 brew install 驗收**。目前版本 0.0.0 僅供施工測試。

公開發布前仍須固定正式版本、完整 CI／獨立覆核、確認 OS 相容性與實際安裝驗收，再審閱 tag、產物與 formula；公開發布及 tap 更新另行執行。未做簽章、公證或所有舊 OS 相容性認證。

## 下一步

安裝後執行 `agend init`。它建立預設 `$HOME/.agend` 與設定並執行 doctor；既有設定保留。需要獨立資料目錄時，先設定絕對路徑的 AGEND_HOME。第 13 關完整服務／backend／Telegram 驗收進度見 [施工關頁](gates/gate-13-install.md)。

Brew 原生安裝驗證已接入 release workflow 的 macOS ARM64／Linux x86_64 jobs，僅在一次性 Actions runner 執行。`release_brew_smoke.py` 重新產生並逐字核對四平台 formula，僅將下載 URL 換成同輪 archive 的 file URL，保留 SHA 與安裝邏輯；建立唯一 tap，拒絕既有 agend 安裝，跑 install／formula test／全新 HOME init／uninstall 並檢查清理。這不證明公開 Release URL 已可下載；實跑結果另記。
