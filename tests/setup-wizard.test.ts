import { describe, it, expect, vi, beforeEach } from "vitest";
import { validateBotToken, verifyBotToken } from "../src/setup-wizard.js";

describe("Setup Wizard", () => {
  it("rejects obviously invalid token format", () => {
    expect(validateBotToken("not-a-token")).toBe(false);
    expect(validateBotToken("")).toBe(false);
  });

  it("accepts valid token format", () => {
    expect(validateBotToken("123456789:ABCdefGHIjklMNOpqrSTUvwxYZ_1234567")).toBe(true);
  });

  it("verifies token against Telegram API", async () => {
    const mockFetch = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ ok: true, result: { username: "test_bot" } }),
    });
    global.fetch = mockFetch;

    const result = await verifyBotToken("123456789:ABCdefGHIjklMNOpqrSTUvwxYZ_1234567");
    expect(result).toEqual({ valid: true, username: "test_bot" });
    expect(mockFetch).toHaveBeenCalledWith(
      "https://api.telegram.org/bot123456789:ABCdefGHIjklMNOpqrSTUvwxYZ_1234567/getMe",
    );
  });

  it("returns invalid for rejected token", async () => {
    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ ok: false, description: "Unauthorized" }),
    });

    const result = await verifyBotToken("000000000:fake_token_that_is_long_enough_here");
    expect(result).toEqual({ valid: false, username: null });
  });
});
