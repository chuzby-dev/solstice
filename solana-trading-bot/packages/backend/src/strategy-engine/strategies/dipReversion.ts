import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";

const MIN_LOOKBACK_MINUTES = 30;
const MAX_LOOKBACK_MINUTES = 180;
/** Entry requires at least this fraction of the requested lookback to actually be present
 * in ctx.priceHistory (e.g. right after a restart, or before HISTORY_LIMIT has filled).
 * Without this, a handful of ticks spanning 3 real minutes could be mistaken for a full
 * 90-minute lookback and produce a meaningless "dip" reading — a much bigger risk here
 * than for the 1-15min windows short-window-grid/range-scalper use, since the gap between
 * "some data" and "the full requested window" is proportionally much larger at 90-180min. */
const MIN_WINDOW_COVERAGE_FRACTION = 0.8;

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

/**
 * Dip Reversion — buys a confirmed dip on a 30-180 minute timeframe and holds for a bounce.
 *
 * Unlike every other strategy in this codebase, this one wasn't designed from a TA
 * playbook and then tuned — it was reverse-engineered from what actually has a
 * statistically real edge in this app's own historical SOL/USD data (Birdeye, see
 * docs/ARCHITECTURE.md "Backtesting"), tested BEFORE any code was written:
 *
 * - Momentum/trend-following has no edge here at any timescale from 5 minutes to 168
 *   hours (hit rate <=50%, negative average forward return throughout) — the 180-day
 *   window is a real -42% secular decline, so chasing direction mostly means buying into
 *   a slow bleed. This is why momentum/rsi-macd/volatility-breakout all struggled to
 *   validate in the first tuning pass, not just imprecise parameters.
 * - Buying a confirmed dip and holding has a real edge that STRENGTHENS with lookback: at
 *   10-20min scales it's real but thin (~53-57% hit rate, ~0.01-0.04% avg forward return —
 *   smaller than a typical ~0.3% round-trip fee, not tradeable). At 60-180min scales it's
 *   strong and fee-beating: e.g. 90min lookback / 60min hold / 1.5% dip threshold measured
 *   72.9% hit rate and +0.33% average forward return across 288 independent samples in 14
 *   days of 1-minute data.
 *
 * This is why every param here is real elapsed time or a % magnitude, never a tick count
 * (contrast momentum.lookbackPeriods, rsiMacd's periods, etc., all in `TICK_COUNT_PARAMS`
 * in backtest/sweep.ts): a strategy whose edge only exists at a 60-180 minute timeframe
 * would be silently meaningless if written as a tick-count period and fed the live 2s-poll
 * engine's tick stream. `ctx.priceHistory`'s budget (market/priceCache.ts's
 * HISTORY_LIMIT) was raised from 600 to 6000 ticks (~200min at the default poll rate)
 * specifically so this strategy's 30-180min lookback is actually available live, not just
 * in backtest.
 *
 * The dip is measured from the window's own high (not just first-tick-vs-now) so a peak
 * anywhere within the lookback counts, not only one at the very start of the window.
 * Entry reuses range-scalper's confirmation-tick (buy the turn, not the falling knife) and
 * re-entry-cooldown idioms; exit uses confluence-scalper's simpler fixed-%-of-entry
 * target/stop rather than range-scalper's range-relative one, since there's no "range" to
 * be relative to here — just a dip magnitude and a bounce target off the entry price.
 * `hardStopPct` runs unconditionally before any window-data check, same reasoning as every
 * other strategy's hard stop: a position must never go unprotected after a data-feed gap.
 *
 * HONEST OUTCOME, round 1 (45-day fee/execution-realistic backtest, tuning/validation
 * split — see docs/ARCHITECTURE.md "Backtesting"): the ORIGINAL defaults (90min lookback,
 * 1.5% dip threshold) were net negative on both the tuning window (-0.09%, 87 round trips)
 * and validation (-0.05%, 12 round trips). Root cause, found by simulating the exact
 * entry/exit rule fee-free: the raw edge per trade (+0.21% avg) was real but SMALLER than
 * the ~0.31% round-trip fee at this position size — the signal was firing on dips too
 * small/frequent to be worth trading.
 *
 * HONEST OUTCOME, round 2: widened to a rarer, larger-dip trigger (180min lookback — the
 * clamp ceiling — and 2.5% threshold, plus matching wider target/stop/hold/cooldown) after
 * directly measuring where the fee-free avg-return-per-trade clears the fee floor (~0.5%
 * avg return at this shape, comfortably above the ~0.3% cost). Result: the current shipped
 * defaults are the first strategy in this codebase with a POSITIVE return on BOTH the
 * tuning window (+0.06%, 32 round trips) and the held-out validation window (+0.01%, 3
 * round trips). Still a thin result — 3 validation trades isn't strong statistical
 * confidence, just a genuinely different outcome than every prior "negative or overfit"
 * result. A further-tuned variant found by the sweep looked much better on tuning (+0.42%,
 * 100% win rate) but had ZERO validation trades — rejected on the same overfitting
 * standard applied everywhere else in this codebase, not adopted just because the number
 * looked good.
 *
 * HONEST OUTCOME, round 3 (90-day TWO-REGIME retest): round 2's entire 45-day window was a
 * rising market (+15%). Extending to 90 days of 1m candles adds a genuinely out-of-sample
 * first half that was a -23% decline — and the shipped defaults FAILED it: -0.13% on the
 * decline-dominated tuning window (39 round trips, 41% win rate) vs +0.06% on the
 * rising-regime validation window (11 round trips). Notably, the raw ENTRY signal stayed
 * positive in both halves on a fee-free fixed-hold test (+0.27% declining / +0.50% rising,
 * n=31/42) — what fails in the declining regime is the full package: the 1.5% bounce
 * target rarely fills before the 5% hard stop or time stop does, so exits systematically
 * capture the downside and fees eat the rest. The round-3 sweep did find a config positive
 * on both windows (+0.35%/+0.07%) but with only 8 tuning / 2 validation trades — rejected
 * on the exact same thin-sample overfitting standard as round 2's rejected variant.
 * Verdict downgraded to `not-profitable`: round 2's "first profitable strategy" result was
 * a regime artifact, not a durable edge. This is the second strategy in this codebase
 * whose apparent edge vanished under a regime split (see doubleBottomRetest.ts) — any
 * future "profitable" claim must be validated across BOTH a rising and a declining window
 * before it counts.
 */
export class DipReversionStrategy extends StrategyBase {
  readonly id = "dip-reversion" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const p = ctx.config.params;
    const lookbackMinutes = clamp(p.lookbackMinutes ?? 180, MIN_LOOKBACK_MINUTES, MAX_LOOKBACK_MINUTES);
    const dipThresholdPct = p.dipThresholdPct ?? 2.5;
    const targetBouncePct = p.targetBouncePct ?? 1.5;
    const holdMinutes = p.holdMinutes ?? 180;
    const hardStopPct = p.hardStopPct ?? 5;
    const reentryCooldownMinutes = p.reentryCooldownMinutes ?? 90;
    const positionSizeUsd = p.positionSizeUsd ?? 150;

    const price = ctx.latestPrice.priceUsd;
    const nowMs = ctx.now.getTime();

    // ---------- Exit management (position open) ----------
    if (ctx.currentPosition) {
      const entry = ctx.currentPosition.avgEntryPriceUsd;
      const sizeUsd = ctx.currentPosition.quantity * price;

      // Emergency backstop, checked first and unconditionally — see class doc.
      const hardStopPrice = entry * (1 - hardStopPct / 100);
      if (price <= hardStopPrice) {
        return this.sell(ctx, sizeUsd, `Hard stop: price ${price} <= ${hardStopPct}% below entry ${entry.toFixed(4)}`);
      }

      const targetPrice = entry * (1 + targetBouncePct / 100);
      if (price >= targetPrice) {
        return this.sell(ctx, sizeUsd, `Bounce target: price ${price} >= ${targetBouncePct}% above entry ${entry.toFixed(4)}`);
      }

      if (ctx.lastSignalAt) {
        const heldMinutes = (nowMs - ctx.lastSignalAt.getTime()) / 60_000;
        if (heldMinutes >= holdMinutes) {
          return this.sell(ctx, sizeUsd, `Time stop: held ${heldMinutes.toFixed(1)}min >= ${holdMinutes}min, bounce thesis expired`);
        }
      }

      return null;
    }

    // ---------- Entry pipeline (flat) ----------
    const windowMs = lookbackMinutes * 60_000;
    const windowTicks = this.recentWindow(ctx, windowMs);
    if (windowTicks.length < 2) return null;

    const oldestTick = windowTicks[0]!;
    const actualSpanMs = nowMs - new Date(oldestTick.timestamp).getTime();
    if (actualSpanMs < windowMs * MIN_WINDOW_COVERAGE_FRACTION) return null; // not enough real history yet

    const windowHigh = Math.max(...windowTicks.map((t) => t.priceUsd));
    if (windowHigh <= 0) return null;
    const dipPct = ((windowHigh - price) / windowHigh) * 100;
    if (dipPct < dipThresholdPct) return null;

    // Confirmation tick — buy the turn, not the fall.
    const previousTick = windowTicks[windowTicks.length - 2];
    if (!previousTick || price <= previousTick.priceUsd) return null;

    // Re-entry cooldown since this config's last trade.
    if (ctx.lastSignalAt) {
      const sinceLastTradeMinutes = (nowMs - ctx.lastSignalAt.getTime()) / 60_000;
      if (sinceLastTradeMinutes < reentryCooldownMinutes) return null;
    }

    return this.buy(
      ctx,
      positionSizeUsd,
      `Dip entry: price ${price} is ${dipPct.toFixed(2)}% below the ${lookbackMinutes}min high of $${windowHigh.toFixed(4)}, turning up`,
    );
  }
}
