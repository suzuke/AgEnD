import { Cron } from "croner";
import type { EventLog } from "./event-log.js";
import type { DailySummaryConfig } from "./types.js";
import { formatCents } from "./cost-guard.js";

export class DailySummary {
  private job: Cron | null = null;

  constructor(
    private config: DailySummaryConfig,
    private timezone: string,
    private onSummary: (text: string) => void,
    private getSummaryText: () => string,
  ) {}

  start(): void {
    if (!this.config.enabled) return;
    const cron = `${this.config.minute} ${this.config.hour} * * *`;
    this.job = new Cron(cron, { timezone: this.timezone }, () => {
      const text = this.getSummaryText();
      this.onSummary(text);
    });
  }

  stop(): void {
    this.job?.stop();
    this.job = null;
  }

  static generateText(
    eventLog: EventLog,
    instances: string[],
    costCentsMap: Map<string, number>,
    fleetTotalCents: number,
  ): string {
    const today = new Date().toISOString().split("T")[0];
    const todayEvents = eventLog.query({ since: today, limit: 1000 });

    const lines: string[] = [`📊 Daily Report — ${today}`, ""];

    for (const name of instances) {
      const instanceEvents = todayEvents.filter(e => e.instance_name === name);
      const rotations = instanceEvents.filter(e => e.event_type === "context_rotation").length;
      const hangs = instanceEvents.filter(e => e.event_type === "hang_detected").length;
      const deferred = instanceEvents.filter(e => e.event_type === "schedule_deferred").length;
      const costCents = costCentsMap.get(name) ?? 0;
      const incompleteHandovers = instanceEvents.filter(e =>
        e.event_type === "context_rotation" &&
        (e.payload as Record<string, unknown> | null)?.handover_status !== "complete"
      ).length;

      let line = `${name}: ${formatCents(costCents)}`;
      if (rotations > 0) line += `, ${rotations} rotation${rotations > 1 ? "s" : ""}`;
      if (deferred > 0) line += `, ${deferred} deferred`;

      const anomalies: string[] = [];
      if (hangs > 0) anomalies.push(`${hangs} hang${hangs > 1 ? "s" : ""}`);
      if (incompleteHandovers > 0) anomalies.push(`${incompleteHandovers} incomplete handover${incompleteHandovers > 1 ? "s" : ""}`);
      if (anomalies.length > 0) line += ` ⚠️ ${anomalies.join(", ")}`;

      lines.push(line);
    }

    lines.push("");
    lines.push(`Total: ${formatCents(fleetTotalCents)}`);

    return lines.join("\n");
  }
}
