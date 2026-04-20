import { describe, it, expect } from "vitest";
import { resolveAccessPathFromConfig } from "../src/access-path.js";
import { join } from "node:path";

describe("resolveAccessPathFromConfig", () => {
  const dataDir = "/data";

  it("returns fleet-level path for topic mode", () => {
    const result = resolveAccessPathFromConfig(dataDir, "my-inst", { mode: "topic" });
    expect(result).toBe(join(dataDir, "access", "access.json"));
  });

  it("returns per-instance path when no fleet channel configured", () => {
    const result = resolveAccessPathFromConfig(dataDir, "my-inst", undefined);
    expect(result).toBe(join(dataDir, "instances", "my-inst", "access.json"));
  });

  it("rejects instance names that contain path traversal segments (P4.3)", () => {
    expect(() => resolveAccessPathFromConfig(dataDir, "..", undefined)).toThrow(/Invalid instance name/);
    expect(() => resolveAccessPathFromConfig(dataDir, ".", undefined)).toThrow(/Invalid instance name/);
    expect(() => resolveAccessPathFromConfig(dataDir, "../etc", undefined)).toThrow(/Invalid instance name/);
    expect(() => resolveAccessPathFromConfig(dataDir, "a/b", undefined)).toThrow(/Invalid instance name/);
    expect(() => resolveAccessPathFromConfig(dataDir, "a\\b", undefined)).toThrow(/Invalid instance name/);
    expect(() => resolveAccessPathFromConfig(dataDir, "", undefined)).toThrow(/Invalid instance name/);
    expect(() => resolveAccessPathFromConfig(dataDir, "a\0b", undefined)).toThrow(/Invalid instance name/);
  });

  it("topic mode does not require instance validation (instance is unused)", () => {
    // Topic mode doesn't embed instance in the path, so it tolerates anything
    // — validation only applies to the per-instance branch.
    const result = resolveAccessPathFromConfig(dataDir, "..", { mode: "topic" });
    expect(result).toBe(join(dataDir, "access", "access.json"));
  });

  it("accepts conventional instance names with letters/digits/_-.", () => {
    expect(() => resolveAccessPathFromConfig(dataDir, "main", undefined)).not.toThrow();
    expect(() => resolveAccessPathFromConfig(dataDir, "my-inst_2", undefined)).not.toThrow();
    expect(() => resolveAccessPathFromConfig(dataDir, "v1.0-main", undefined)).not.toThrow();
  });
});
