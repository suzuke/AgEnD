import type { WebhookConfig } from "./types.js";
import type { Logger } from "./logger.js";

export interface WebhookPayload {
  event: string;
  instance: string;
  timestamp: string;
  data: Record<string, unknown>;
}

export class WebhookEmitter {
  constructor(
    private configs: WebhookConfig[],
    private logger: Logger,
  ) {}

  emit(event: string, instance: string, data: Record<string, unknown> = {}): void {
    const payload: WebhookPayload = {
      event,
      instance,
      timestamp: new Date().toISOString(),
      data,
    };

    for (const config of this.configs) {
      if (config.events.includes("*") || config.events.includes(event)) {
        this.post(config, payload);
      }
    }
  }

  private post(config: WebhookConfig, payload: WebhookPayload, retry = true): void {
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      ...config.headers,
    };

    fetch(config.url, {
      method: "POST",
      headers,
      body: JSON.stringify(payload),
      signal: AbortSignal.timeout(5000),
    }).catch((err) => {
      if (retry) {
        setTimeout(() => this.post(config, payload, false), 1000);
      } else {
        this.logger.warn({ err, url: config.url, event: payload.event }, "Webhook POST failed");
      }
    });
  }
}
