import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";

/** Momentum/Trend-following: enter on breakout above the N-period high.
 *
 * Simplification: the spec calls for "breakout above N-period high with volume
 * confirmation," but the Jupiter Price API used for paper-trading prices does not
 * expose trade volume. Volume confirmation is deferred to a later phase once a
 * volume-capable data source (e.g. Birdeye/DexScreener) is wired in; for now this
 * strategy triggers on price breakout alone. This is called out in docs/ARCHITECTURE.md. */
export class MomentumStrategy extends StrategyBase {
  readonly id = "momentum" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const { lookbackPeriods, breakoutPct, positionSizeUsd, takeProfitPct } = ctx.config.params;

    if (ctx.currentPosition && takeProfitPct) {
      const targetPrice = ctx.currentPosition.avgEntryPriceUsd * (1 + takeProfitPct / 100);
      if (ctx.latestPrice.priceUsd >= targetPrice) {
        const sizeUsd = ctx.currentPosition.quantity * ctx.latestPrice.priceUsd;
        return this.sell(ctx, sizeUsd, `Take-profit: price ${ctx.latestPrice.priceUsd} >= target ${targetPrice.toFixed(4)}`);
      }
      return null; // already in a position from this strategy; don't pyramid in
    }

    const period = lookbackPeriods ?? 20;
    if (ctx.priceHistory.length < period + 1) return null; // not enough history yet

    const window = ctx.priceHistory.slice(-(period + 1), -1); // exclude latest tick
    const periodHigh = Math.max(...window.map((t) => t.priceUsd));
    const breakoutThreshold = periodHigh * (1 + (breakoutPct ?? 1) / 100);

    if (ctx.latestPrice.priceUsd > breakoutThreshold) {
      return this.buy(
        ctx,
        positionSizeUsd ?? 200,
        `Breakout: price ${ctx.latestPrice.priceUsd} > ${period}-period high ${periodHigh.toFixed(4)} + ${breakoutPct ?? 1}%`,
      );
    }

    return null;
  }
}
