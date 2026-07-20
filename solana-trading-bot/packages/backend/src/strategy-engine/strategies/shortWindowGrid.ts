import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";
import { computeGridCrossing } from "./grid.js";

const MIN_TICKS_FOR_RANGE = 3;

/** Short-Window Grid: like GridStrategy, but the [lowerPrice, upperPrice] range is
 * auto-computed every tick from the actual high/low of the last `windowMinutes` of
 * price action (real elapsed time, not a tick count), instead of a fixed manual range.
 * This is what "a grid over the last 5 minutes of movement" means in practice — no need
 * to guess price bounds up front, the grid follows recent volatility. The tradeoff is
 * that the range itself drifts tick to tick as the window rolls forward, so it's a
 * looser fit than a manually fixed grid once price trends strongly in one direction.
 *
 * A quiet market produces a tiny auto-range (a real observed case: a 0.125% range over
 * 5 minutes, i.e. ~0.02% per grid step on a $75 asset) — economically meaningless once
 * real trading fees exist, even though this fee-less simulator shows it as flat P&L.
 * `computeGridCrossing`'s `minRangePct` guard (see grid.ts) sits the strategy out
 * entirely rather than churning on that kind of noise. */
export class ShortWindowGridStrategy extends StrategyBase {
  readonly id = "short-window-grid" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const { windowMinutes, gridLevels, orderSizeUsd, minRangePct } = ctx.config.params;
    const windowMs = (windowMinutes ?? 5) * 60_000;

    const windowTicks = this.recentWindow(ctx, windowMs);
    if (windowTicks.length < MIN_TICKS_FOR_RANGE) return null; // not enough data in the window yet

    const prices = windowTicks.map((t) => t.priceUsd);
    const lowerPrice = Math.min(...prices);
    const upperPrice = Math.max(...prices);

    const result = computeGridCrossing(ctx, lowerPrice, upperPrice, gridLevels ?? 6, orderSizeUsd ?? 100, minRangePct ?? 0.3);
    if (!result) return null;

    const reason = `${result.reason} (auto-range $${lowerPrice.toFixed(4)}-$${upperPrice.toFixed(4)} over last ${windowMinutes ?? 5}min)`;
    return result.action === "buy" ? this.buy(ctx, result.sizeUsd, reason) : this.sell(ctx, result.sizeUsd, reason);
  }
}
