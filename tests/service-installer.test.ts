import { describe, it, expect } from "vitest";
import { renderLaunchdPlist, renderSystemdUnit, detectPlatform, uninstallService } from "../src/service-installer.js";

describe("ServiceInstaller", () => {
  const vars = {
    label: "com.claude-channel-daemon",
    execPath: "/usr/local/bin/claude-channel-daemon",
    workingDirectory: "/Users/test/project",
    logPath: "/Users/test/.claude-channel-daemon/daemon.log",
  };

  it("detects platform correctly", () => {
    const platform = detectPlatform();
    expect(["macos", "linux"]).toContain(platform);
  });

  it("renders launchd plist with correct values", () => {
    const plist = renderLaunchdPlist(vars);
    expect(plist).toContain("<string>com.claude-channel-daemon</string>");
    expect(plist).toContain("<string>/usr/local/bin/claude-channel-daemon</string>");
    expect(plist).toContain("<string>start</string>");
  });

  it("renders systemd unit with correct values", () => {
    const unit = renderSystemdUnit(vars);
    expect(unit).toContain("ExecStart=/usr/local/bin/claude-channel-daemon start");
    expect(unit).toContain("WorkingDirectory=/Users/test/project");
  });
});
