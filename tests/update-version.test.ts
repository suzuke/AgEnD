import { describe, it, expect } from "vitest";
import { validateUpdateVersion } from "../src/topic-commands.js";

describe("validateUpdateVersion (P3.6)", () => {
  it("defaults to latest when given empty/undefined input", () => {
    expect(validateUpdateVersion(undefined)).toBe("latest");
    expect(validateUpdateVersion("")).toBe("latest");
    expect(validateUpdateVersion("   ")).toBe("latest");
  });

  it("accepts dist-tags", () => {
    expect(validateUpdateVersion("latest")).toBe("latest");
    expect(validateUpdateVersion("next")).toBe("next");
    expect(validateUpdateVersion("beta")).toBe("beta");
  });

  it("accepts semver and pre-release tags", () => {
    expect(validateUpdateVersion("1.22.0")).toBe("1.22.0");
    expect(validateUpdateVersion("1.22.0-beta.1")).toBe("1.22.0-beta.1");
    expect(validateUpdateVersion("2.0.0+build.5")).toBe("2.0.0+build.5");
  });

  it("rejects shell metacharacters so the arg cannot break out of npm install", () => {
    expect(() => validateUpdateVersion("1.0.0; rm -rf /")).toThrow(/Invalid/);
    expect(() => validateUpdateVersion("$(whoami)")).toThrow(/Invalid/);
    expect(() => validateUpdateVersion("`id`")).toThrow(/Invalid/);
    expect(() => validateUpdateVersion("1.0 && echo pwned")).toThrow(/Invalid/);
    expect(() => validateUpdateVersion("1.0 | sh")).toThrow(/Invalid/);
  });

  it("rejects paths and URLs (npm accepts these as install targets)", () => {
    expect(() => validateUpdateVersion("/tmp/evil.tgz")).toThrow(/Invalid/);
    expect(() => validateUpdateVersion("https://evil.example/pkg.tgz")).toThrow(/Invalid/);
    expect(() => validateUpdateVersion("../other-pkg")).toThrow(/Invalid/);
  });

  it("rejects whitespace inside the version", () => {
    expect(() => validateUpdateVersion("1.0 0")).toThrow(/Invalid/);
  });

  it("rejects leading punctuation that could alter npm parsing", () => {
    expect(() => validateUpdateVersion("-rf")).toThrow(/Invalid/);
    expect(() => validateUpdateVersion(".hidden")).toThrow(/Invalid/);
  });
});
