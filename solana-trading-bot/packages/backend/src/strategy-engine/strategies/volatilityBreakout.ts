import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";
import { closeToCloseVolatility } from "../indicators.js";

/** Volatility Breakout: enters when a single tick-to-tick price move exceeds
 * `atrMultiplier` times the recent average volatility (see indicators.ts for why this
 * uses a close-to-close proxy instead of true ATR). */
export class VolatilityBreakoutStrategy extends StrategyBase {
  readonly id = "volatility-breakout" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const { atrPeriod, atrMultiplier, positionSizeUsd, takeProfitPct } = ctx.config.params;

    if (ctx.currentPosition && takeProfitPct) {
      const targetPrice = ctx.currentPosition.avgEntryPriceUsd * (1 + takeProfitPct / 100);
      if (ctx.latestPrice.priceUsd >= targetPrice) {
        const sizeUsd = ctx.currentPosition.quantity * ctx.latestPrice.priceUsd;
        return this.sell(ctx, sizeUsd, `Take-profit: price ${ctx.latestPrice.priceUsd} >= target ${targetPrice.toFixed(4)}`);
      }
      return null;
    }
    if (ctx.currentPosition) return null;

    const prices = ctx.priceHistory.map((t) => t.priceUsd);
    const volatility = closeToCloseVolatility(prices, atrPeriod ?? 14);
    const previousTick = ctx.priceHistory[ctx.priceHistory.length - 2];
    if (volatility === null || !previousTick || volatility === 0) return null;

    const move = ctx.latestPrice.priceUsd - previousTick.priceUsd;
    const breakoutThreshold = volatility * (atrMultiplier ?? 2);

    if (move > breakoutThreshold) {
      return this.buy(
        ctx,
        positionSizeUsd ?? 200,
        `Volatility breakout: move ${move.toFixed(4)} > ${(atrMultiplier ?? 2)}x avg volatility ${volatility.toFixed(4)}`,
      );
    }

    return null;
  }
}
