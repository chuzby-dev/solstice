import type { BuiltInStrategyId } from "@trading-bot/shared";
import type { BirdeyeInterval } from "./birdeyeClient.js";

/** whale-copy depends on live on-chain transfer data with no historical-replay source in
 * this codebase (see docs/ARCHITECTURE.md "Backtesting") — not backtestable from price
 * history alone. Shared between the CLI (scripts/backtest.ts) and the on-demand
 * /api/backtest routes so the allowlist can't drift between the two callers. */
export const REPLAYABLE_STRATEGIES: BuiltInStrategyId[] = [
  "dca",
  "momentum",
  "mean-reversion",
  "grid",
  "rsi-macd",
  "volatility-breakout",
  "short-window-grid",
  "range-scalper",
  "confluence-scalper",
  "fee-aware-scalper",
  "dip-reversion",
  "flash-dip-reversal",
  "double-bottom-retest",
];

// Scalpers need fine-grained candles to see intra-window structure; everything else holds
// positions for hours/days and is fine on coarser candles, which can look back much
// further without paginating as hard against Birdeye's 1000-candles-per-request cap.
// dip-reversion's 30-180min lookback, mean-reversion's 5-180min window,
// flash-dip-reversal's 10-45min window, and double-bottom-retest's 45-150min window all
// need 1-minute resolution to resolve meaningfully (1H candles would collapse any of them
// into ~1-3 data points).
const FINE_GRAINED = new Set<BuiltInStrategyId>([
  "range-scalper",
  "short-window-grid",
  "confluence-scalper",
  "fee-aware-scalper",
  "dip-reversion",
  "mean-reversion",
  "flash-dip-reversal",
  "double-bottom-retest",
]);

export const DEFAULT_FINE_DAYS = 14;
export const DEFAULT_COARSE_DAYS = 180;

export interface CandleConfig {
  interval: BirdeyeInterval;
  days: number;
  isFine: boolean;
}

export function getCandleConfig(strategyId: BuiltInStrategyId, fineDays = DEFAULT_FINE_DAYS, coarseDays = DEFAULT_COARSE_DAYS): CandleConfig {
  const isFine = FINE_GRAINED.has(strategyId);
  return isFine ? { interval: "1m", days: fineDays, isFine } : { interval: "1H", days: coarseDays, isFine };
}
