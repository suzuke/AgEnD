# systemd 實際載入屬性

> - 四份 JSON 來自隔離 Ubuntu 24.04 容器的 systemd 255／busctl。
> - 保留原生 `GetAll` envelope、型別和值；只刪去測試不使用的屬性。
> - 這是 parser／所有權反例證據，不代表真 AgEnD 服務生命週期驗收。

2026-10-08 擷取：容器內以 systemd 為 PID 1，安裝 `systemd-sysv`、`libpam-systemd`、`dbus-user-session`，啟動 `user@0.service`。未啟動 `agend-daemon.service`，未執行模型；容器與測試檔案在擷取後移除。

`native-*` 的 user unit 由正式 service renderer 產生：home 為 `/tmp/g13 data % $value "quote"`、user home 為 `/tmp/g13-user`、PATH 為 `/usr/bin:/bin`，unit path 為 `/root/.config/systemd/user/agend-daemon.service`。

`override-*` 在同一 unit 加入下列 drop-in，再執行 `systemctl --user daemon-reload` 後擷取；FragmentPath 保持相同。

```ini
[Service]
ExecStart=
ExecStart=/usr/bin/true
ExecStop=/usr/bin/true
KillMode=control-group
Environment=AGEND_HOME=/foreign
```

兩次都先用 `systemctl --user show agend-daemon.service` 載入 unit，再依序對 `org.freedesktop.systemd1.Service` 與 `org.freedesktop.systemd1.Unit` 呼叫：

```sh
XDG_RUNTIME_DIR=/run/user/0 busctl --user --json=short call \
  org.freedesktop.systemd1 \
  /org/freedesktop/systemd1/unit/agend_2ddaemon_2eservice \
  org.freedesktop.DBus.Properties GetAll s INTERFACE
```

格式依據：[busctl JSON 型別保留](https://github.com/systemd/systemd/blob/v255/man/busctl.xml)、[systemd D-Bus 屬性](https://github.com/systemd/systemd/blob/v255/man/org.freedesktop.systemd1.xml)。原始擷取與失敗環境診斷保存在 `AgEnD-ops/g13-install-20261008/systemd-effective-*.log`。

## User bus readiness

`native-name-owner-{false,true}.json` 保留 2026-10-08 隔離 Ubuntu 24.04／systemd 255 的原生 boolean envelope。以 UID 19413 啟動 user manager 後，使用 `busctl --user --json=short --auto-start=no --allow-interactive-authorization=no call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus NameHasOwner s org.freedesktop.systemd1` 取得 false，約 57 ms 後取得 true；原始紀錄為 `AgEnD-ops/g13-install-20261008/linux-release-service-user-ready-native.jsonl`。容器／映像／暫存已刪除。當次用實驗 wrapper 完成生命週期，不冒充正式修正的驗收。

## 下一步

`cargo test -p agend --bin agend service::manager::systemd`，再於隔離 Linux 執行實際安裝／停止／解除安裝驗收；不得由這四份 fixture 宣稱整關通過。
