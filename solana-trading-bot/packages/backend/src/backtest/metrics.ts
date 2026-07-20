import type { BacktestResult } from "./backtestEngine.js";

export interface BacktestMetrics {
  totalReturnPct: number;
  totalReturnUsd: number;
  tradeCount: number;
  roundTripCount: number;
  winRate: number | null;
  profitFactor: number | null;
  maxDrawdownPct: number;
  avgHoldMinutes: number | null;
  totalFeesUsd: number;
  feeDragPct: number;
}

/** Pure summary stats over one backtest run. Win rate / profit factor / avg hold time are
 * computed from each sell fill's own realized delta (ledger.ts records this exactly per
 * fill, including partial sells), not reconstructed after the fact — see LedgerFill. */
export function computeMetrics(result: BacktestResult): BacktestMetrics {
  const { startingCashUsd, endingTotalValueUsd, trades, totalFeesUsd, equityCurve } = result;
  const totalReturnUsd = endingTotalValueUsd - startingCashUsd;
  const totalReturnPct = (totalReturnUsd / startingCashUsd) * 100;

  const sellFills = result.fills.filter((f) => f.realizedDeltaUsd !== null);
  const wins = sellFills.filter((f) => (f.realizedDeltaUsd as number) > 0);
  const losses = sellFills.filter((f) => (f.realizedDeltaUsd as number) <= 0);
  const grossProfit = wins.reduce((s, f) => s + (f.realizedDeltaUsd as number), 0);
  const grossLoss = Math.abs(losses.reduce((s, f) => s + (f.realizedDeltaUsd as number), 0));

  const holdTimes = sellFills.map((f) => f.holdMinutes).filter((m): m is number => m !== null);

  let peak = -Infinity;
  let maxDrawdownPct = 0;
  for (const point of equityCurve) {
    peak = Math.max(peak, point.totalValueUsd);
    if (peak > 0) maxDrawdownPct = Math.max(maxDrawdownPct, ((peak - point.totalValueUsd) / peak) * 100);
  }

  return {
    totalReturnPct,
    totalReturnUsd,
    tradeCount: trades.length,
    roundTripCount: sellFills.length,
    winRate: sellFills.length > 0 ? wins.length / sellFills.length : null,
    profitFactor: grossLoss > 0 ? grossProfit / grossLoss : grossProfit > 0 ? Number.POSITIVE_INFINITY : null,
    maxDrawdownPct,
    avgHoldMinutes: holdTimes.length > 0 ? holdTimes.reduce((s, m) => s + m, 0) / holdTimes.length : null,
    totalFeesUsd,
    feeDragPct: (totalFeesUsd / startingCashUsd) * 100,
  };
}

/** Composite ranking score for the parameter sweep: reward return, penalize drawdown so a
 * high-variance param set doesn't win purely on a lucky run. `minTrades` filters out configs
 * that barely traded (a single lucky trade shouldn't crown a "best" config). */
export function scoreResult(metrics: BacktestMetrics, minTrades = 5): number {
  if (metrics.roundTripCount < minTrades) return Number.NEGATIVE_INFINITY;
  return metrics.totalReturnPct - 0.5 * metrics.maxDrawdownPct;
}
