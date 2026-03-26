import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { DailySummary } from "../src/daily-summary.js";
import { EventLog } from "../src/event-log.js";
import { mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("DailySummary.generateText", () => {
  let tmpDir: string;
  let eventLog: EventLog;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `ccd-summary-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
    eventLog = new EventLog(join(tmpDir, "events.db"));
  });

  afterEach(() => {
    eventLog.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("generates summary with costs and rotations", () => {
    eventLog.insert("proj-a", "context_rotation", { handover_status: "complete" });
    eventLog.insert("proj-b", "cost_snapshot", { accumulated_cents: 200 });

    const costMap = new Map([["proj-a", 820], ["proj-b", 200]]);
    const text = DailySummary.generateText(eventLog, ["proj-a", "proj-b"], costMap, 1020);
    expect(text).toContain("proj-a");
    expect(text).toContain("$8.20");
    expect(text).toContain("1 rotation");
    expect(text).toContain("$10.20"); // fleet total
  });

  it("highlights hang anomalies", () => {
    eventLog.insert("proj-a", "hang_detected", {});
    eventLog.insert("proj-a", "hang_detected", {});

    const text = DailySummary.generateText(eventLog, ["proj-a"], new Map([["proj-a", 0]]), 0);
    expect(text).toContain("2 hangs");
  });

  it("highlights incomplete handovers", () => {
    eventLog.insert("proj-a", "context_rotation", { handover_status: "timeout" });
    eventLog.insert("proj-a", "context_rotation", { handover_status: "complete" });

    const text = DailySummary.generateText(eventLog, ["proj-a"], new Map([["proj-a", 0]]), 0);
    expect(text).toContain("1 incomplete handover");
  });

  it("shows deferred schedules", () => {
    eventLog.insert("proj-a", "schedule_deferred", { schedule_id: "x" });

    const text = DailySummary.generateText(eventLog, ["proj-a"], new Map([["proj-a", 0]]), 0);
    expect(text).toContain("1 deferred");
  });

  it("handles empty events gracefully", () => {
    const text = DailySummary.generateText(eventLog, ["proj-a"], new Map([["proj-a", 500]]), 500);
    expect(text).toContain("proj-a: $5.00");
    expect(text).toContain("Total: $5.00");
    expect(text).not.toContain("⚠️");
  });
});
