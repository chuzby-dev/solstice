import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";

/** Dollar-Cost Averaging: buy a fixed USD amount every `intervalMinutes`.
 * Optionally takes profit at `takeProfitPct` above the position's average entry price.
 * The mandatory protective stop-loss is enforced globally by the risk manager/simulator,
 * not by this strategy. */
export class DcaStrategy extends StrategyBase {
  readonly id = "dca" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const { intervalMinutes, amountUsd, takeProfitPct } = ctx.config.params;

    if (ctx.currentPosition && takeProfitPct) {
      const targetPrice = ctx.currentPosition.avgEntryPriceUsd * (1 + takeProfitPct / 100);
      if (ctx.latestPrice.priceUsd >= targetPrice) {
        const sizeUsd = ctx.currentPosition.quantity * ctx.latestPrice.priceUsd;
        return this.sell(ctx, sizeUsd, `Take-profit: price ${ctx.latestPrice.priceUsd} >= target ${targetPrice.toFixed(4)}`);
      }
    }

    const intervalMs = (intervalMinutes ?? 60) * 60_000;
    const elapsedMs = ctx.lastSignalAt ? ctx.now.getTime() - ctx.lastSignalAt.getTime() : Infinity;
    if (elapsedMs < intervalMs) return null;

    return this.buy(ctx, amountUsd ?? 100, `Scheduled DCA buy (every ${intervalMinutes ?? 60}min)`);
  }
}
