import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";

const MIN_LOOKBACK_MINUTES = 10;
const MAX_LOOKBACK_MINUTES = 45;
/** Same coverage-floor idiom as dip-reversion/mean-reversion — entry requires at least this
 * fraction of the requested lookback to actually be present in ctx.priceHistory, so a
 * handful of ticks right after a restart can't be mistaken for a full window. */
const MIN_WINDOW_COVERAGE_FRACTION = 0.8;
/** The recent sub-window (as a fraction of lookbackMinutes) used to test whether the
 * decline was concentrated near the end of the window rather than spread across all of it.
 * Not exposed as a strategy param — deliberately fixed, same reasoning as dip-reversion's
 * fixed coverage floor: it's a shape constant the edge was measured against, not something
 * to let the tuner drift away from the tested behavior. */
const CONCENTRATION_WINDOW_FRACTION = 1 / 3;
const MIN_CONCENTRATION_WINDOW_MINUTES = 3;

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

/**
 * Flash-Dip Reversal — buys a SHARP, CONCENTRATED price drop (most of the decline happening
 * in the final third of a short 10-45 minute lookback) on a confirming uptick, and holds
 * for a bounce. Distinct from dip-reversion (which buys any dip of a given magnitude over a
 * 30-180min window, regardless of how fast it happened) and from mean-reversion (which buys
 * any std-dev deviation from a rolling mean, also speed-agnostic).
 *
 * Reverse-engineered from real historical SOL/USD data the same way dip-reversion was
 * (Birdeye, see docs/ARCHITECTURE.md "Backtesting"), tested BEFORE any code was written.
 * dip-reversion's own class doc already noted its edge was "real but thin" at 10-20min
 * lookbacks — smaller than the ~0.3% round-trip fee. The follow-up question this strategy
 * answers: is that short-timeframe edge thin because it's genuinely weak, or because it's
 * mixing two different situations — a fast flash-crash/liquidation wick (which tends to
 * snap back) and a slow grinding decline (which tends to keep grinding, i.e. is closer to a
 * real trend than a dip)? Splitting the same 45-day dataset's confirmed-dip population by
 * how CONCENTRATED the decline was (>=60% of the total drop happening in the final third of
 * the lookback window) answered it: on independent, non-overlapping samples at a 20min
 * lookback / 1% drop threshold, SHARP declines beat GRADUAL ones on every hold window
 * tested — e.g. 30min hold: sharp n=90, 57.8% hit rate, +0.176% avg forward return, ~1.68
 * profit factor, vs. gradual n=108, 39.8% hit rate, -0.066% avg (net negative). The
 * concentration filter is what turns dip-reversion's thin short-timeframe signal into
 * something that can clear fees — it's not just "dip-reversion but shorter."
 *
 * Like dip-reversion's own honest history, this raw signal test ignores fees, position
 * sizing, and the risk manager entirely — it's what motivated this strategy's design, not
 * proof it survives contact with the real execution model.
 *
 * HONEST OUTCOME (45-day fee/execution-realistic backtest, tuning/validation split — see
 * docs/ARCHITECTURE.md "Backtesting"): it did NOT survive contact. The shipped defaults are
 * net-negative on both windows (tuning -0.07%, 33 round trips, 42.4% win rate; validation
 * -0.01%, 3 round trips). A 150-trial tuning sweep found a config that looked good on the
 * tuning window (lookbackMinutes=14, dropThresholdPct=1.29, concentrationFraction=0.41,
 * targetBouncePct=1.30, holdMinutes=79: +0.20% return, 71.4% win rate) but it still lost
 * money on the 1-trade held-out validation window — the same "great on tuning, unconfirmed
 * out-of-sample" pattern that got an equally good-looking dip-reversion variant rejected.
 * Unlike dip-reversion, no wider/rarer-trigger reshaping was found (within the searched
 * range) that flipped both windows positive. Conclusion: the raw forward-return edge this
 * strategy was built around (~+0.15-0.2% avg, from independent-sample testing before any
 * code was written) is real but sits too close to the ~0.3% round-trip fee floor to survive
 * real position sizing and risk-manager overhead — a textbook case of a genuine statistical
 * signal that isn't a tradeable edge. Kept in the registry as `not-profitable` rather than
 * deleted, same as every other honestly-negative strategy in this codebase.
 *
 * All params are real elapsed time or a % magnitude, never a tick count — same reasoning as
 * dip-reversion/mean-reversion (see backtest/sweep.ts's TICK_COUNT_PARAMS comment): this
 * strategy's edge only exists at a specific real-world timeframe, so it must stay
 * meaningful regardless of the live engine's poll rate. Structurally: hard stop runs
 * unconditionally before any window-data check (never leave a position unprotected after a
 * restart/data-gap); entry reuses the confirmation-tick and re-entry-cooldown idioms shared
 * by every dip-style strategy in this codebase.
 *
 * Concentration is measured by comparing the dip% (window-high to current price) computed
 * over the FULL lookback window against the same formula computed over just the trailing
 * third of it. If most of the drop is visible even in that short recent slice, the two
 * numbers are close and concentrationFraction is near 1; if the drop was spread evenly
 * across the whole window, the short slice only sees a small piece of it and
 * concentrationFraction is low. This reuses `recentWindow` at two sizes rather than
 * searching for the window's trough index directly.
 */
export class FlashDipReversalStrategy extends StrategyBase {
  readonly id = "flash-dip-reversal" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const p = ctx.config.params;
    const lookbackMinutes = clamp(p.lookbackMinutes ?? 20, MIN_LOOKBACK_MINUTES, MAX_LOOKBACK_MINUTES);
    const dropThresholdPct = p.dropThresholdPct ?? 1;
    const concentrationFraction = p.concentrationFraction ?? 0.6;
    const targetBouncePct = p.targetBouncePct ?? 1;
    const holdMinutes = p.holdMinutes ?? 45;
    const hardStopPct = p.hardStopPct ?? 2.5;
    const reentryCooldownMinutes = p.reentryCooldownMinutes ?? 20;
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
    if (actualSpanMs < windowMs * MIN_WINDOW_COVERAGE_FRACTION) return null;

    const windowHigh = Math.max(...windowTicks.map((t) => t.priceUsd));
    if (windowHigh <= 0) return null;
    const dipPct = ((windowHigh - price) / windowHigh) * 100;
    if (dipPct < dropThresholdPct) return null;

    // Concentration check — is most of this dip recent, or spread across the whole window?
    const concentrationWindowMs = Math.max(MIN_CONCENTRATION_WINDOW_MINUTES * 60_000, windowMs * CONCENTRATION_WINDOW_FRACTION);
    const shortWindowTicks = this.recentWindow(ctx, concentrationWindowMs);
    const shortWindowHigh = shortWindowTicks.length > 0 ? Math.max(...shortWindowTicks.map((t) => t.priceUsd)) : windowHigh;
    const shortDipPct = ((shortWindowHigh - price) / shortWindowHigh) * 100;
    if (shortDipPct / dipPct < concentrationFraction) return null; // decline too spread out — not a flash dip

    // Confirmation tick — buy the turn, not the falling knife.
    const previousTick = windowTicks[windowTicks.length - 2];
    if (!previousTick || price <= previousTick.priceUsd) return null;

    if (ctx.lastSignalAt) {
      const sinceLastTradeMinutes = (nowMs - ctx.lastSignalAt.getTime()) / 60_000;
      if (sinceLastTradeMinutes < reentryCooldownMinutes) return null;
    }

    return this.buy(
      ctx,
      positionSizeUsd,
      `Flash-dip entry: price ${price} is ${dipPct.toFixed(2)}% below the ${lookbackMinutes}min high of $${windowHigh.toFixed(4)}, ${(shortDipPct / dipPct * 100).toFixed(0)}% of it within the last ${(concentrationWindowMs / 60_000).toFixed(0)}min, turning up`,
    );
  }
}
