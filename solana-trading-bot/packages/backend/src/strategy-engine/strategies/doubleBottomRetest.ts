import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";

const MIN_LOOKBACK_MINUTES = 45;
const MAX_LOOKBACK_MINUTES = 150;
const MIN_WINDOW_COVERAGE_FRACTION = 0.8;
/** The first low must be found in the earlier portion of the lookback window, leaving room
 * for a genuine bounce-then-retest to play out in the remainder. Not exposed as a param —
 * a fixed shape constant the edge was measured against, same reasoning as
 * flashDipReversal's CONCENTRATION_WINDOW_FRACTION. */
const FIRST_LOW_SEARCH_FRACTION = 2 / 3;
/** How far the retest is allowed to dip below the first low before it counts as a
 * breakdown instead of a retest. Small and fixed, not tunable — widening this would let
 * genuine downtrend continuations masquerade as retests. */
const BREAK_BUFFER_PCT = 0.15;

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

/**
 * Double-Bottom Retest — buys when price makes a low, bounces a real amount off it, pulls
 * back down to RETEST that same low without breaking meaningfully below it, then turns up
 * again. Classic technical "double bottom" shape. Structurally stricter than
 * dip-reversion/flash-dip-reversal's single-confirming-tick entry: those buy on the FIRST
 * sign of a turn, which lets a lot of dead-cat-bounce noise through (a bounce that
 * immediately fails and keeps falling looks identical to a real reversal until later ticks
 * disambiguate them). Requiring the low to be defended on a second visit is a much stronger
 * filter for genuine support — at the cost of firing less often, since most dips never get
 * (or need) a retest at all.
 *
 * Reverse-engineered from real historical SOL/USD data the same way as this codebase's
 * other reversal strategies (Birdeye, see docs/ARCHITECTURE.md "Backtesting"), tested
 * BEFORE any code was written. On independent, non-overlapping samples across 45 days of
 * 1-minute data, requiring a real (>=1%) bounce before the retest was the difference
 * between noise and a strong edge: at a fixed 0.5% bounce threshold the signal was flat
 * (hit rate ~50%, avg forward return ~0%, profit factor ~1.0 — no edge at all) at every
 * lookback/hold combination tried. Raising the bounce requirement to >=1% (same lookback
 * window, same retest logic) turned it into the strongest signal found in this codebase to
 * date: at a 90min lookback / 60min hold, n=78 independent samples, 65.4% hit rate, +0.378%
 * average forward return (comfortably above the ~0.3% round-trip fee floor), profit factor
 * 3.39. A stricter 1.5% bounce threshold measured even higher average returns (+0.81%, 18
 * samples) but on a much smaller sample — the 1% threshold was chosen as the shipped
 * default specifically because it has the larger, more statistically trustworthy sample.
 *
 * Like every reversal strategy in this codebase, this raw signal test ignores fees,
 * position sizing, and the risk manager entirely — it's what motivated the design, not
 * proof it survives contact with the real execution model.
 *
 * HONEST OUTCOME (45-day fee/execution-realistic backtest, tuning/validation split — see
 * docs/ARCHITECTURE.md "Backtesting"): despite being the strongest raw signal found in this
 * codebase to date, it did NOT survive contact. Shipped defaults are barely positive on the
 * tuning window (+0.03%, 59 round trips, 52.5% win rate, 0.27% fee drag — the fees alone
 * ate most of the raw edge) and net-negative on the 11-trade held-out validation window
 * (-0.06%). A 150-trial tuning sweep found a config that looked much better on tuning
 * (lookbackMinutes=66, bouncePct=1.17, retestTolerancePct=0.22, targetBouncePct=2.27,
 * holdMinutes=114, hardStopPct=3.54, reentryCooldownMinutes=25: +0.17% return, 80% win
 * rate) but it produced only 1 validation trade — the same "great on tuning, unconfirmed
 * out-of-sample" pattern that got flash-dip-reversal's best tuned config and an equally
 * strong-looking dip-reversion variant rejected. Conclusion: a genuinely real, well-sampled
 * raw statistical edge (n=78, 65.4% hit rate, comfortably above the fee floor on paper)
 * still lost most of its value to realistic fees and round-trip friction once actually
 * traded — the double-bottom filter produces a real signal, just not one with enough
 * margin over trading costs to be worth activating. Kept in the registry as
 * `not-profitable` rather than deleted, same as every other honestly-negative strategy in
 * this codebase.
 *
 * All params are real elapsed time or a % magnitude, never a tick count — same reasoning as
 * every other strategy in this family (see backtest/sweep.ts's TICK_COUNT_PARAMS comment).
 * Structurally: hard stop runs unconditionally before any window-data check (never leave a
 * position unprotected after a restart/data-gap); exit priority mirrors dip-reversion/
 * flash-dip-reversal exactly (hard stop -> target bounce -> time stop).
 *
 * The first-low search is confined to the earlier FIRST_LOW_SEARCH_FRACTION of the lookback
 * window (by elapsed time, not tick count, so it stays meaningful regardless of the live
 * engine's poll rate) so there's always room left in the window for the bounce and retest
 * to actually happen. BREAK_BUFFER_PCT gives the retest a small amount of room to dip
 * slightly below the first low without being called a breakdown — real retests rarely land
 * on the exact same price twice.
 */
export class DoubleBottomRetestStrategy extends StrategyBase {
  readonly id = "double-bottom-retest" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const p = ctx.config.params;
    const lookbackMinutes = clamp(p.lookbackMinutes ?? 90, MIN_LOOKBACK_MINUTES, MAX_LOOKBACK_MINUTES);
    const bouncePct = p.bouncePct ?? 1;
    const retestTolerancePct = p.retestTolerancePct ?? 0.3;
    const targetBouncePct = p.targetBouncePct ?? 1;
    const holdMinutes = p.holdMinutes ?? 60;
    const hardStopPct = p.hardStopPct ?? 2.5;
    const reentryCooldownMinutes = p.reentryCooldownMinutes ?? 60;
    const positionSizeUsd = p.positionSizeUsd ?? 150;

    const price = ctx.latestPrice.priceUsd;
    const nowMs = ctx.now.getTime();

    // ---------- Exit management (position open) ----------
    if (ctx.currentPosition) {
      const entry = ctx.currentPosition.avgEntryPriceUsd;
      const sizeUsd = ctx.currentPosition.quantity * price;

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
          return this.sell(ctx, sizeUsd, `Time stop: held ${heldMinutes.toFixed(1)}min >= ${holdMinutes}min, retest thesis expired`);
        }
      }

      return null;
    }

    // ---------- Entry pipeline (flat) ----------
    const windowMs = lookbackMinutes * 60_000;
    const windowTicks = this.recentWindow(ctx, windowMs);
    if (windowTicks.length < 3) return null;

    const oldestTick = windowTicks[0]!;
    const oldestMs = new Date(oldestTick.timestamp).getTime();
    const actualSpanMs = nowMs - oldestMs;
    if (actualSpanMs < windowMs * MIN_WINDOW_COVERAGE_FRACTION) return null;

    // First low: the minimum within the earlier FIRST_LOW_SEARCH_FRACTION of the window,
    // leaving room afterward for the bounce and retest to play out.
    const firstLowSearchEndMs = oldestMs + windowMs * FIRST_LOW_SEARCH_FRACTION;
    let firstLowIdx = 0;
    for (let i = 0; i < windowTicks.length; i++) {
      const t = windowTicks[i]!;
      if (new Date(t.timestamp).getTime() > firstLowSearchEndMs) break;
      if (t.priceUsd < windowTicks[firstLowIdx]!.priceUsd) firstLowIdx = i;
    }
    const firstLow = windowTicks[firstLowIdx]!.priceUsd;
    if (firstLow <= 0) return null;

    // Bounce: the highest price reached between the first low and now.
    let bounceHighIdx = firstLowIdx;
    for (let i = firstLowIdx; i < windowTicks.length; i++) {
      if (windowTicks[i]!.priceUsd > windowTicks[bounceHighIdx]!.priceUsd) bounceHighIdx = i;
    }
    const bounceHigh = windowTicks[bounceHighIdx]!.priceUsd;
    const bouncedPct = ((bounceHigh - firstLow) / firstLow) * 100;
    if (bouncedPct < bouncePct) return null;
    if (bounceHighIdx >= windowTicks.length - 1) return null; // no room left for a retest

    // No breakdown: nothing after the bounce high may fall meaningfully below the first low.
    const breakdownFloor = firstLow * (1 - BREAK_BUFFER_PCT / 100);
    for (let i = bounceHighIdx + 1; i < windowTicks.length; i++) {
      if (windowTicks[i]!.priceUsd < breakdownFloor) return null;
    }

    // Retest: current price must have come back close to the first low.
    const retestDistPct = (Math.abs(price - firstLow) / firstLow) * 100;
    if (retestDistPct > retestTolerancePct) return null;

    // Confirmation tick — buy the turn off the retest, not the fall into it.
    const previousTick = windowTicks[windowTicks.length - 2];
    if (!previousTick || price <= previousTick.priceUsd) return null;

    if (ctx.lastSignalAt) {
      const sinceLastTradeMinutes = (nowMs - ctx.lastSignalAt.getTime()) / 60_000;
      if (sinceLastTradeMinutes < reentryCooldownMinutes) return null;
    }

    return this.buy(
      ctx,
      positionSizeUsd,
      `Double-bottom entry: price ${price} retested the ${lookbackMinutes}min low of $${firstLow.toFixed(4)} (bounced ${bouncedPct.toFixed(2)}% off it first), turning up`,
    );
  }
}
