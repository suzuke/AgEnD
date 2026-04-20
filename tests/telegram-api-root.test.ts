import { describe, it, expect } from "vitest";
import { validateTelegramApiRoot } from "../src/channel/adapters/telegram.js";

describe("validateTelegramApiRoot", () => {
  it("accepts the official Telegram API root", () => {
    expect(() => validateTelegramApiRoot("https://api.telegram.org")).not.toThrow();
  });

  it("accepts loopback hosts (E2E mock servers)", () => {
    expect(() => validateTelegramApiRoot("http://localhost:8081")).not.toThrow();
    expect(() => validateTelegramApiRoot("http://127.0.0.1:8081")).not.toThrow();
    expect(() => validateTelegramApiRoot("http://[::1]:8081")).not.toThrow();
  });

  it("rejects arbitrary external hosts (token-exfil risk)", () => {
    expect(() => validateTelegramApiRoot("https://evil.example.com")).toThrow(/allowlist/i);
    expect(() => validateTelegramApiRoot("https://api.telegram.org.evil.com")).toThrow(/allowlist/i);
  });

  it("rejects api.telegram.org over plain http (forces https)", () => {
    expect(() => validateTelegramApiRoot("http://api.telegram.org")).toThrow(/https/i);
  });

  it("rejects non-http(s) schemes", () => {
    expect(() => validateTelegramApiRoot("file:///etc/passwd")).toThrow(/http/i);
    expect(() => validateTelegramApiRoot("ftp://api.telegram.org")).toThrow(/http/i);
  });

  it("rejects malformed URLs", () => {
    expect(() => validateTelegramApiRoot("not a url")).toThrow(/Invalid/i);
    expect(() => validateTelegramApiRoot("")).toThrow(/Invalid/i);
  });

  it("hostname matching is case-insensitive", () => {
    expect(() => validateTelegramApiRoot("https://API.TELEGRAM.ORG")).not.toThrow();
  });
});
