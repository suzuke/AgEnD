# v2 成為 main：分支切換紀錄

> **TL;DR**
> - 2026-10-12 使用者授權將 v2 改為 main，舊版另存可辨識分支。
> - GitHub 預設分支已改為 `main`（Rust v2）；舊版為 `legacy/v1-typescript`。
> - 本次不代表完成 Claude、人工驗收或公開發布，也沒有合併待審 PR。

## 分支對照

| 原分支 | 新分支 | 切換時提交 |
|---|---|---|
| main（TypeScript） | legacy/v1-typescript | `2e38b5f1fc6e7636948d22111e9641bb20daab81` |
| v2（Rust） | main | `ea1e4879ee355616ded2480e6ab4015047cf534c` |

以 GitHub rename API 依序改名，再設定 default branch；兩個分支的提交與歷史保持不變。使用者這次指示取代先前「等 Roadmap 只剩 Claude 後才切換、且不改名」的安排。

原 v2 的 macOS／Ubuntu 必要 checks、禁止 force push／刪除與線性歷史保護隨分支轉移。既有 `strict=false`、管理員不強制套用的政策維持原值。#160、#161 的 PR base 已轉為 main。

切換時該提交的 Ubuntu CI 通過、macOS CI 失敗；改名不等於 CI 或正式版本驗收通過。剩餘工作以 [第 13 關現況](gates/gate-13-status.md) 為準。

## 既有本機 clone

新 clone 自動取得 Rust v2。既有 clone 不會自行改名；先確認工作目錄乾淨及分支歸屬，再依自己的情況調整。

同時有舊 main 與 v2 時：

```sh
git branch -m main legacy/v1-typescript
git branch -m v2 main
git fetch origin
git branch -u origin/legacy/v1-typescript legacy/v1-typescript
git branch -u origin/main main
git remote set-head origin -a
```

只有舊 main 時，先將它改為 legacy/v1-typescript，再 fetch，最後以 `git switch --track origin/main` 建立新的 main。不要對舊 main 直接 pull 新 main 的不同歷史。

## 下一步

依 [README](../README.md) 安裝；後續 PR 以 main 為目標。安裝文件與 release workflow 的名稱修正隨 #160／#161 審查交付，不因此擅自合併。
