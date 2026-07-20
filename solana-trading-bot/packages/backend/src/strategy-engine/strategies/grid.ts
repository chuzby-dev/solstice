import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";

export interface GridCrossingResult {
  action: "buy" | "sell";
  sizeUsd: number;
  reason: string;
}

/** Shared level-crossing logic for grid.ts and shortWindowGrid.ts: buys when price
 * crosses down through a grid line while flat, sells when it later crosses back up
 * through a grid line above the entry price. Pure function so both strategies (which
 * differ only in how they compute lowerPrice/upperPrice) can share it.
 *
 * `minRangePct` guards against a real failure mode, not a hypothetical one: a grid
 * whose range is too thin relative to price produces grid steps of a few cents on a
 * $75 asset — this simulator charges no fees, so that reads as flat P&L here, but the
 * same behavior against real trading fees/spread would be a guaranteed loser. Below
 * this floor the strategy sits out entirely rather than trading pure noise. */
export function computeGridCrossing(
  ctx: StrategyContext,
  lowerPrice: number | undefined,
  upperPrice: number | undefined,
  gridLevels: number,
  orderSizeUsd: number,
  minRangePct: number,
): GridCrossingResult | null {
  if (!lowerPrice || !upperPrice || upperPrice <= lowerPrice) return null;
  if (((upperPrice - lowerPrice) / lowerPrice) * 100 < minRangePct) return null;

  const levels = Math.max(1, Math.floor(gridLevels));
  const gridStep = (upperPrice - lowerPrice) / levels;

  const previousTick = ctx.priceHistory[ctx.priceHistory.length - 2];
  if (!previousTick) return null; // need at least two ticks to detect a crossing

  const levelOf = (price: number): number => Math.floor((price - lowerPrice) / gridStep);
  const currentLevel = levelOf(ctx.latestPrice.priceUsd);
  const previousLevel = levelOf(previousTick.priceUsd);

  if (ctx.latestPrice.priceUsd < lowerPrice || ctx.latestPrice.priceUsd > upperPrice) return null; // outside the grid range

  if (!ctx.currentPosition) {
    if (currentLevel < previousLevel) {
      const gridLine = lowerPrice + currentLevel * gridStep;
      return { action: "buy", sizeUsd: orderSizeUsd, reason: `Crossed down through grid line $${gridLine.toFixed(4)}` };
    }
    return null;
  }

  if (currentLevel > previousLevel && ctx.latestPrice.priceUsd > ctx.currentPosition.avgEntryPriceUsd) {
    const gridLine = lowerPrice + currentLevel * gridStep;
    const sizeUsd = ctx.currentPosition.quantity * ctx.latestPrice.priceUsd;
    return { action: "sell", sizeUsd, reason: `Crossed up through grid line $${gridLine.toFixed(4)}, above entry` };
  }

  return null;
}

/** Grid Trading: divides a fixed [lowerPrice, upperPrice] into `gridLevels` equal steps.
 * See docs/ARCHITECTURE.md for the single-position simplification versus a real grid's
 * many simultaneous orders. For a range that auto-adapts to recent price action instead
 * of a fixed manual range, see ShortWindowGridStrategy. */
export class GridStrategy extends StrategyBase {
  readonly id = "grid" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const { lowerPrice, upperPrice, gridLevels, orderSizeUsd, minRangePct } = ctx.config.params;
    const result = computeGridCrossing(ctx, lowerPrice, upperPrice, gridLevels ?? 10, orderSizeUsd ?? 100, minRangePct ?? 0.3);
    if (!result) return null;
    return result.action === "buy" ? this.buy(ctx, result.sizeUsd, result.reason) : this.sell(ctx, result.sizeUsd, result.reason);
  }
}
