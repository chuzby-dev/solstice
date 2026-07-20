import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";
import { bollingerBands, sma } from "../indicators.js";

const MIN_WINDOW_MINUTES = 5;
const MAX_WINDOW_MINUTES = 180;
/** Below this a stdDev reading is too noisy (a handful of ticks) to trust as a real
 * measure of dispersion, regardless of how long the requested window is. */
const MIN_WINDOW_TICKS = 10;
/** Same reasoning as dip-reversion's identical constant: without this, a handful of ticks
 * spanning a much shorter real interval than requested (right after a restart, or before
 * priceCache's HISTORY_LIMIT has filled) could be mistaken for a full window and produce a
 * meaningless mean/stdDev reading. */
const MIN_WINDOW_COVERAGE_FRACTION = 0.8;

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

/**
 * Mean Reversion — buys when price falls a configurable number of standard deviations
 * below its own rolling mean over a configurable real-time window, and sells once price
 * reverts back to that mean.
 *
 * This REPLACES an earlier version of this strategy that computed its moving average over
 * a fixed tick COUNT (`maPeriod`) rather than real elapsed time, had no standard-deviation
 * basis at all (just a flat %-below-MA threshold), and had none of the safety rails every
 * other strategy in this codebase carries (no confirmation tick, no hard stop, no time
 * stop, no re-entry cooldown) — exactly the "naive" pattern already documented as a known
 * problem elsewhere (see dip-reversion's and range-scalper's class docs). Rebuilt to match
 * those conventions:
 *
 * - `windowMinutes` and every other param here is real elapsed time or a % magnitude,
 *   never a tick count — the old `maPeriod` (ticks) meant the moving average's real time
 *   span silently depended on the poll interval, and would mean something different in a
 *   backtest replaying historical candles than live at a 2s poll (the same
 *   tick-count-vs-live-cadence trap documented for momentum/rsi-macd/volatility-breakout
 *   in docs/ARCHITECTURE.md).
 * - Mean and standard deviation are both computed over the SAME real-time window, reusing
 *   `indicators.ts`'s `bollingerBands` — passed `period = window.length` so it operates on
 *   the already time-filtered tick array instead of a tick count. This is the standard
 *   Bollinger Band definition, just anchored to elapsed time rather than a bar count.
 * - Entry requires a confirmation tick (price turning up, not still falling) — buy the
 *   turn, not the falling knife, same idiom as range-scalper/dip-reversion.
 * - A fixed `hardStopPct` backstop is checked unconditionally, before any window-data
 *   availability check, so a position is never left unprotected after a data-feed gap or
 *   restart — same reasoning as every other strategy's hard stop.
 * - `maxHoldMinutes` is a time stop: if price never reverts, the position doesn't sit open
 *   indefinitely waiting for a mean that may itself have drifted on.
 * - `reentryCooldownMinutes` prevents immediately re-buying the same dip right after
 *   exiting it.
 *
 * Not yet validated against real historical data with the tuning/validation split this
 * codebase otherwise requires before calling a strategy "profitable" (see
 * docs/ARCHITECTURE.md "Backtesting") — ships as `untested` until that pass runs.
 */
export class MeanReversionStrategy extends StrategyBase {
  readonly id = "mean-reversion" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const p = ctx.config.params;
    const windowMinutes = clamp(p.windowMinutes ?? 60, MIN_WINDOW_MINUTES, MAX_WINDOW_MINUTES);
    const entryStdDevs = p.entryStdDevs ?? 2;
    const hardStopPct = p.hardStopPct ?? 5;
    const maxHoldMinutes = p.maxHoldMinutes ?? 120;
    const reentryCooldownMinutes = p.reentryCooldownMinutes ?? 30;
    const positionSizeUsd = p.positionSizeUsd ?? 150;

    const price = ctx.latestPrice.priceUsd;
    const nowMs = ctx.now.getTime();
    const windowMs = windowMinutes * 60_000;
    const windowTicks = this.recentWindow(ctx, windowMs);

    // ---------- Exit management (position open) ----------
    if (ctx.currentPosition) {
      const entry = ctx.currentPosition.avgEntryPriceUsd;
      const sizeUsd = ctx.currentPosition.quantity * price;

      // Emergency backstop, checked first and unconditionally — see class doc.
      const hardStopPrice = entry * (1 - hardStopPct / 100);
      if (price <= hardStopPrice) {
        return this.sell(ctx, sizeUsd, `Hard stop: price ${price} <= ${hardStopPct}% below entry ${entry.toFixed(4)}`);
      }

      if (windowTicks.length >= MIN_WINDOW_TICKS) {
        const mean = sma(
          windowTicks.map((t) => t.priceUsd),
          windowTicks.length,
        );
        if (mean !== null && price >= mean) {
          return this.sell(ctx, sizeUsd, `Reverted to ${windowMinutes}min mean $${mean.toFixed(4)}`);
        }
      }

      if (ctx.lastSignalAt) {
        const heldMinutes = (nowMs - ctx.lastSignalAt.getTime()) / 60_000;
        if (heldMinutes >= maxHoldMinutes) {
          return this.sell(ctx, sizeUsd, `Time stop: held ${heldMinutes.toFixed(1)}min >= ${maxHoldMinutes}min, reversion thesis expired`);
        }
      }

      return null;
    }

    // ---------- Entry pipeline (flat) ----------
    if (windowTicks.length < MIN_WINDOW_TICKS) return null;

    const oldestTick = windowTicks[0]!;
    const actualSpanMs = nowMs - new Date(oldestTick.timestamp).getTime();
    if (actualSpanMs < windowMs * MIN_WINDOW_COVERAGE_FRACTION) return null; // not enough real history yet

    const bands = bollingerBands(
      windowTicks.map((t) => t.priceUsd),
      windowTicks.length,
      entryStdDevs,
    );
    if (!bands || bands.stdDev <= 0) return null; // flat/degenerate window — nothing to revert from
    if (price > bands.lower) return null; // hasn't dipped far enough below the mean yet

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
      `Reversion entry: price ${price} is ${entryStdDevs}+ std devs below the ${windowMinutes}min mean $${bands.middle.toFixed(4)} (stdDev $${bands.stdDev.toFixed(4)}), turning up`,
    );
  }
}
