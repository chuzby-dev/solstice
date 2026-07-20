import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";
import { macd, rsi } from "../indicators.js";

// Histogram values near zero can land on either side of it by floating-point noise
// alone (e.g. 1e-15 from EMA rounding) even when the underlying series is genuinely
// flat. Without this tolerance, a crossover right after a flat/near-zero stretch can be
// silently missed because "previous" never reads as cleanly <= 0.
const HISTOGRAM_EPSILON = 1e-8;

/** RSI/MACD Crossover: enters on a bullish MACD crossover (confirmed by RSI not already
 * overbought), exits on a bearish MACD crossover or RSI reaching overbought. */
export class RsiMacdStrategy extends StrategyBase {
  readonly id = "rsi-macd" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const { rsiPeriod, macdFast, macdSlow, macdSignal, overboughtRsi, positionSizeUsd } = ctx.config.params;
    const prices = ctx.priceHistory.map((t) => t.priceUsd);

    const current = macd(prices, macdFast ?? 12, macdSlow ?? 26, macdSignal ?? 9);
    const previous = macd(prices.slice(0, -1), macdFast ?? 12, macdSlow ?? 26, macdSignal ?? 9);
    const currentRsi = rsi(prices, rsiPeriod ?? 14);
    if (!current || !previous || currentRsi === null) return null; // not enough history yet

    const bullishCrossover = previous.histogram <= HISTOGRAM_EPSILON && current.histogram > HISTOGRAM_EPSILON;
    const bearishCrossover = previous.histogram >= -HISTOGRAM_EPSILON && current.histogram < -HISTOGRAM_EPSILON;
    const overbought = overboughtRsi ?? 70;

    if (ctx.currentPosition) {
      if (bearishCrossover || currentRsi >= overbought) {
        const sizeUsd = ctx.currentPosition.quantity * ctx.latestPrice.priceUsd;
        const reason = bearishCrossover ? "Bearish MACD crossover" : `RSI ${currentRsi.toFixed(1)} reached overbought (${overbought})`;
        return this.sell(ctx, sizeUsd, reason);
      }
      return null;
    }

    if (bullishCrossover && currentRsi < overbought) {
      return this.buy(ctx, positionSizeUsd ?? 200, `Bullish MACD crossover, RSI ${currentRsi.toFixed(1)} (not overbought)`);
    }

    return null;
  }
}
