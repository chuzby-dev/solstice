import type { PriceTick } from "@trading-bot/shared";

// Sized to cover Dip Reversion's max 180-minute lookback (see strategy-engine/strategies/
// dipReversion.ts) with margin — 6000 ticks is ~200 minutes at the default 2s poll. Was
// 600 (~20min, sized only for the 5-15min windows short-window-grid/range-scalper use)
// until Dip Reversion needed real multi-hour lookback; raised here rather than left as a
// theoretical future-work item once an actual strategy needed it. Cost is trivial — a
// PriceTick is four small fields and only SOL/USDC are tracked.
const HISTORY_LIMIT = 6000;

/** In-memory rolling price history per token mint, used by strategies for lookback windows
 * (e.g. momentum's N-period high) without hitting the DB on every tick. Exported (not just
 * the singleton below) so the backtester can instantiate an isolated cache per run instead
 * of driving the live singleton. */
export class PriceCache {
  private history = new Map<string, PriceTick[]>();

  push(tick: PriceTick): void {
    const series = this.history.get(tick.tokenMint) ?? [];
    series.push(tick);
    if (series.length > HISTORY_LIMIT) series.shift();
    this.history.set(tick.tokenMint, series);
  }

  latest(tokenMint: string): PriceTick | undefined {
    const series = this.history.get(tokenMint);
    return series?.[series.length - 1];
  }

  /** Most recent `count` ticks, oldest first. */
  recent(tokenMint: string, count: number): PriceTick[] {
    const series = this.history.get(tokenMint) ?? [];
    return series.slice(Math.max(0, series.length - count));
  }

  /** Ticks from the last `ms` milliseconds (relative to now), oldest first. Used for
   * real-time-windowed strategies (e.g. a rolling 5-minute grid) and the price-history
   * chart, as opposed to `recent()`'s fixed tick count. */
  recentWithinMs(tokenMint: string, ms: number): PriceTick[] {
    const series = this.history.get(tokenMint) ?? [];
    const cutoff = Date.now() - ms;
    return series.filter((t) => new Date(t.timestamp).getTime() >= cutoff);
  }

  allLatest(): PriceTick[] {
    return [...this.history.values()].map((series) => series[series.length - 1]).filter((t): t is PriceTick => !!t);
  }
}

export const priceCache = new PriceCache();
