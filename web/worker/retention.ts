import { DurableObject } from "cloudflare:workers";

/** One coordinator for this service's retention job, not a proxy for API traffic. */
export class RetentionAlarm extends DurableObject<Env> {
  async start(): Promise<void> {
    if ((await this.ctx.storage.getAlarm()) === null) {
      await this.ctx.storage.setAlarm(Date.now() + 60_000);
    }
  }

  async alarm(): Promise<void> {
    try {
      await this.env.DB.batch(
        [
          "challenges",
          "sessions",
          "tickets",
          "introductions",
          "profiles",
          "rate_limits",
        ].map((table) =>
          this.env.DB.prepare(`DELETE FROM ${table} WHERE expires <= ?`).bind(
            Date.now(),
          ),
        ),
      );
      await this.ctx.storage.setAlarm(Date.now() + 3_600_000);
    } catch {
      // Persist a retry even if a prolonged D1 outage exhausts platform retries.
      await this.ctx.storage.setAlarm(Date.now() + 60_000);
      throw new Error("Retention cleanup failed; retry scheduled");
    }
  }
}
