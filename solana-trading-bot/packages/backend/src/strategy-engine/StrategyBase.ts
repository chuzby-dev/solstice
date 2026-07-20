import type { BuiltInStrategyId, PriceTick, Signal, StrategyConfig } from "@trading-bot/shared";

export interface CurrentPosition {
  quantity: number;
  avgEntryPriceUsd: number;
}

export interface StrategyContext {
  config: StrategyConfig;
  /** Recent price history for this token, oldest first, latest tick included. */
  priceHistory: PriceTick[];
  latestPrice: PriceTick;
  now: Date;
  currentPosition: CurrentPosition | null;
  lastSignalAt: Date | null;
}

export abstract class StrategyBase {
  abstract readonly id: BuiltInStrategyId;

  /** Called once per engine tick for each active config of this strategy type.
   * Return null for "hold" (no action) — signals are only emitted for buy/sell. */
  abstract onInterval(ctx: StrategyContext): Signal | null;

  /** Ticks from `ctx.priceHistory` within the last `windowMs` of `ctx.now`, oldest first.
   * `priceHistory` is guaranteed oldest-first, so this scans backward from the end and
   * stops at the first tick outside the window — O(ticks actually in the window), not
   * O(full history) like `ctx.priceHistory.filter(...)` (the previous idiom here). That
   * distinction used to be cheap when `priceHistory` topped out at 600 ticks, but once
   * it was raised to 6000 (for Dip Reversion's 90-180min lookback — see
   * market/priceCache.ts's HISTORY_LIMIT), a naive filter's per-element `Date` parse
   * multiplied by thousands of history entries and hundreds of backtest-sweep trials
   * caused a real hang: an Auto-tune run on Dip Reversion froze the live server for
   * several minutes before this fix, since every strategy using the naive filter
   * (range-scalper, short-window-grid, dip-reversion) paid the full history-length scan
   * on every single replayed tick. */
  protected recentWindow(ctx: StrategyContext, windowMs: number): PriceTick[] {
    const nowMs = ctx.now.getTime();
    const result: PriceTick[] = [];
    for (let i = ctx.priceHistory.length - 1; i >= 0; i--) {
      const t = ctx.priceHistory[i]!;
      if (nowMs - new Date(t.timestamp).getTime() > windowMs) break;
      result.push(t);
    }
    result.reverse();
    return result;
  }

  protected buy(ctx: StrategyContext, sizeUsd: number, reason: string): Signal {
    return {
      strategyConfigId: ctx.config.id,
      strategyId: this.id,
      action: "buy",
      tokenMint: ctx.config.tokenMint,
      tokenSymbol: ctx.config.tokenSymbol,
      sizeUsd,
      reason,
      timestamp: ctx.now.toISOString(),
    };
  }

  protected sell(ctx: StrategyContext, sizeUsd: number, reason: string): Signal {
    return {
      strategyConfigId: ctx.config.id,
      strategyId: this.id,
      action: "sell",
      tokenMint: ctx.config.tokenMint,
      tokenSymbol: ctx.config.tokenSymbol,
      sizeUsd,
      reason,
      timestamp: ctx.now.toISOString(),
    };
  }
}
