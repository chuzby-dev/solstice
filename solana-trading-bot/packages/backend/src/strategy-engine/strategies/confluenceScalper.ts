import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";
import { bollingerBands, emaLatest, rsi } from "../indicators.js";

/** Confluence Scalper: combines the standard short-term-scalping technical-analysis
 * toolkit into one entry decision, rather than relying on any single indicator the way
 * every other strategy in this codebase does (one moving average, one breakout, one
 * crossover, or one range position).
 *
 * Confluence used:
 *  - EMA(fast)/EMA(slow) trend filter — only trade pullbacks WITH the short-term trend,
 *    never against it. This is the single highest-impact rule in most real scalping
 *    systems: a technically perfect pullback entry against a falling trend still loses
 *    more often than it wins.
 *  - Bollinger Bands — price at/below the lower band flags a statistically stretched
 *    pullback worth watching (population-stddev definition, matching standard charting
 *    platforms).
 *  - RSI — a pullback into `oversoldRsi` territory is the other qualifying signal for
 *    "worth watching." This is an OR with the Bollinger condition, not an AND: requiring
 *    both simultaneously is the same mistake an earlier strategy in this codebase made
 *    (a reward:risk gate tuned so strict it could barely ever fire) — one confirmed
 *    pullback signal is enough, not a rare alignment of two.
 *  - Confirmation tick — buy the turn, not the fall, the same defense already proven out
 *    in RangeScalperStrategy: a dip alone isn't an entry, a dip that's ticking back up is.
 *
 * Deliberately excluded, despite being common scalping tools:
 *  - VWAP — needs trade volume, which Pyth's price feed doesn't provide (same
 *    limitation already documented on MomentumStrategy/VolatilityBreakoutStrategy).
 *  - Stochastic Oscillator — measures essentially the same thing as RSI (momentum
 *    extremes over a lookback). Adding both isn't more robust confirmation, it's the
 *    same signal counted twice.
 *  - MACD — already the dedicated signal in RsiMacdStrategy. This strategy is meant to
 *    add a genuinely different angle (trend + volatility bands + pullback), not re-skin
 *    an existing one under a new name.
 *
 * Exit is deliberately simple: a fixed take-profit/stop-loss, both expressed as % of
 * entry price — unlike RangeScalperStrategy's first (buggy) attempt, there's no mixing
 * of a range-relative measure with a price-relative one to get wrong, since both sides
 * use the same unit. Realistic defaults (0.3% target / 0.15% stop) give a fixed 2:1
 * reward:risk sized to the SOL volatility actually observed while building this
 * codebase (5-15 minute ranges commonly land between 0.05% and 0.5% — see
 * docs/ARCHITECTURE.md). On top of the price targets: exit immediately if the EMA trend
 * itself flips against the position (the entire premise was trading WITH the trend; if
 * that's gone, the thesis is gone too, independent of where price sits vs target/stop),
 * plus a time stop for a scalp whose thesis hasn't resolved quickly.
 *
 * Periods here are tick counts, matching RsiMacdStrategy/MomentumStrategy/
 * VolatilityBreakoutStrategy's convention (not real-time windows like
 * RangeScalperStrategy/ShortWindowGridStrategy) — effective real-time lookback scales
 * with PRICE_POLL_INTERVAL_MS (default 2s, so the default 21-tick slow EMA covers ~42s
 * of history; slow it down if you configure a longer poll interval). */
export class ConfluenceScalperStrategy extends StrategyBase {
  readonly id = "confluence-scalper" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const p = ctx.config.params;
    const emaFastPeriod = p.emaFastPeriod ?? 9;
    const emaSlowPeriod = p.emaSlowPeriod ?? 21;
    const bollingerPeriod = p.bollingerPeriod ?? 20;
    const bollingerStdDev = p.bollingerStdDev ?? 2;
    const rsiPeriod = p.rsiPeriod ?? 9;
    const oversoldRsi = p.oversoldRsi ?? 45;
    const overboughtRsi = p.overboughtRsi ?? 70;
    const takeProfitPct = p.takeProfitPct ?? 0.3;
    const stopLossPct = p.stopLossPct ?? 0.15;
    const maxHoldMinutes = p.maxHoldMinutes ?? 8;
    const positionSizeUsd = p.positionSizeUsd ?? 100;

    const prices = ctx.priceHistory.map((t) => t.priceUsd);
    const price = ctx.latestPrice.priceUsd;

    const emaFast = emaLatest(prices, emaFastPeriod);
    const emaSlow = emaLatest(prices, emaSlowPeriod);
    const bands = bollingerBands(prices, bollingerPeriod, bollingerStdDev);
    const currentRsi = rsi(prices, rsiPeriod);
    if (emaFast === null || emaSlow === null || bands === null || currentRsi === null) return null; // not enough history yet

    const uptrend = emaFast > emaSlow;

    if (ctx.currentPosition) {
      const entry = ctx.currentPosition.avgEntryPriceUsd;
      const sizeUsd = ctx.currentPosition.quantity * price;

      const stopPrice = entry * (1 - stopLossPct / 100);
      if (price <= stopPrice) {
        return this.sell(ctx, sizeUsd, `Stop-loss: price ${price} <= ${stopLossPct}% below entry ${entry.toFixed(4)}`);
      }

      const targetPrice = entry * (1 + takeProfitPct / 100);
      if (price >= targetPrice) {
        return this.sell(ctx, sizeUsd, `Take-profit: price ${price} >= ${takeProfitPct}% above entry ${entry.toFixed(4)}`);
      }

      if (!uptrend) {
        return this.sell(ctx, sizeUsd, `Trend invalidated: EMA${emaFastPeriod} crossed below EMA${emaSlowPeriod}, pullback thesis no longer holds`);
      }

      if (ctx.lastSignalAt) {
        const heldMinutes = (ctx.now.getTime() - ctx.lastSignalAt.getTime()) / 60_000;
        if (heldMinutes >= maxHoldMinutes) {
          return this.sell(ctx, sizeUsd, `Time stop: held ${heldMinutes.toFixed(1)}min >= ${maxHoldMinutes}min`);
        }
      }

      return null;
    }

    // ---------- Entry pipeline (flat) ----------
    if (!uptrend) return null; // never buy a pullback against the trend

    const inPullbackZone = price <= bands.lower || currentRsi <= oversoldRsi;
    if (!inPullbackZone) return null;

    if (currentRsi >= overboughtRsi) return null; // structural safety check

    const previousTick = ctx.priceHistory[ctx.priceHistory.length - 2];
    if (!previousTick || price <= previousTick.priceUsd) return null; // confirmation tick: buy the turn, not the fall

    return this.buy(
      ctx,
      positionSizeUsd,
      `Confluence entry: EMA${emaFastPeriod}(${emaFast.toFixed(4)}) > EMA${emaSlowPeriod}(${emaSlow.toFixed(4)}) uptrend, price ${price} ${price <= bands.lower ? `at/below lower Bollinger $${bands.lower.toFixed(4)}` : `RSI ${currentRsi.toFixed(1)} oversold`}, turning up`,
    );
  }
}
