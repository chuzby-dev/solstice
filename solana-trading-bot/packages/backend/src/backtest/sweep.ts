import type { BuiltInStrategyId, PriceTick, RiskLimits } from "@trading-bot/shared";
import { strategyMetadata } from "../strategy-engine/registry.js";
import { runBacktest } from "./backtestEngine.js";
import { computeMetrics, scoreResult, type BacktestMetrics } from "./metrics.js";

export interface ParamSpec {
  /** Sampled uniformly in [default * minFactor, default * maxFactor]. */
  minFactor: number;
  maxFactor: number;
  integer?: boolean;
}

/** Params that count price TICKS rather than elapsed time — see docs/ARCHITECTURE.md
 * "Backtesting" for why these need conversion before being applied to the live 2s-poll
 * engine. Params not listed here (windowMinutes, maxHoldMinutes, intervalMinutes, ...) are
 * already real-elapsed-time and granularity-independent. */
export const TICK_COUNT_PARAMS: Partial<Record<BuiltInStrategyId, string[]>> = {
  momentum: ["lookbackPeriods"],
  "rsi-macd": ["rsiPeriod", "macdFast", "macdSlow", "macdSignal"],
  "volatility-breakout": ["atrPeriod"],
  "fee-aware-scalper": ["smaPeriod"],
  "confluence-scalper": ["emaFastPeriod", "emaSlowPeriod", "bollingerPeriod", "rsiPeriod"],
  // dip-reversion, mean-reversion, flash-dip-reversal, and double-bottom-retest
  // deliberately have NO entry here: every one of their params (windowMinutes/
  // lookbackMinutes, maxHoldMinutes/holdMinutes, reentryCooldownMinutes) is already real
  // elapsed time, designed that way from the start specifically to avoid this whole class
  // of bug — see their class docs in strategy-engine/strategies/meanReversion.ts,
  // dipReversion.ts, flashDipReversal.ts, and doubleBottomRetest.ts. mean-reversion used
  // to have `maPeriod` here before it was rebuilt around a real-time window.
};

/** Search bounds per strategy, as a factor of the shipped default. `grid`'s lowerPrice/
 * upperPrice are absolute prices (default 0 = "unset"), so they're sampled from the
 * observed historical price range in sampleParams() instead of a multiplicative factor. */
export const PARAM_SPECS: Partial<Record<BuiltInStrategyId, Record<string, ParamSpec>>> = {
  dca: {
    intervalMinutes: { minFactor: 0.25, maxFactor: 3, integer: true },
    amountUsd: { minFactor: 0.5, maxFactor: 2 },
    takeProfitPct: { minFactor: 0.3, maxFactor: 3 },
  },
  momentum: {
    lookbackPeriods: { minFactor: 0.25, maxFactor: 4, integer: true },
    breakoutPct: { minFactor: 0.2, maxFactor: 5 },
    positionSizeUsd: { minFactor: 0.5, maxFactor: 2 },
    takeProfitPct: { minFactor: 0.3, maxFactor: 3 },
  },
  "mean-reversion": {
    windowMinutes: { minFactor: 0.2, maxFactor: 3, integer: true },
    entryStdDevs: { minFactor: 0.5, maxFactor: 2 },
    hardStopPct: { minFactor: 0.5, maxFactor: 2 },
    maxHoldMinutes: { minFactor: 0.3, maxFactor: 2, integer: true },
    reentryCooldownMinutes: { minFactor: 0.3, maxFactor: 3, integer: true },
    positionSizeUsd: { minFactor: 0.5, maxFactor: 2 },
  },
  grid: {
    gridLevels: { minFactor: 0.3, maxFactor: 3, integer: true },
    orderSizeUsd: { minFactor: 0.5, maxFactor: 2 },
    minRangePct: { minFactor: 0.3, maxFactor: 4 },
  },
  "rsi-macd": {
    rsiPeriod: { minFactor: 0.5, maxFactor: 2, integer: true },
    macdFast: { minFactor: 0.5, maxFactor: 2, integer: true },
    macdSlow: { minFactor: 0.5, maxFactor: 2, integer: true },
    macdSignal: { minFactor: 0.5, maxFactor: 2, integer: true },
    overboughtRsi: { minFactor: 0.85, maxFactor: 1.1 },
    positionSizeUsd: { minFactor: 0.5, maxFactor: 2 },
  },
  "volatility-breakout": {
    atrPeriod: { minFactor: 0.4, maxFactor: 3, integer: true },
    atrMultiplier: { minFactor: 0.4, maxFactor: 3 },
    positionSizeUsd: { minFactor: 0.5, maxFactor: 2 },
    takeProfitPct: { minFactor: 0.3, maxFactor: 3 },
  },
  "short-window-grid": {
    windowMinutes: { minFactor: 0.3, maxFactor: 3, integer: true },
    gridLevels: { minFactor: 0.3, maxFactor: 3, integer: true },
    orderSizeUsd: { minFactor: 0.5, maxFactor: 2 },
    minRangePct: { minFactor: 0.3, maxFactor: 4 },
  },
  "range-scalper": {
    windowMinutes: { minFactor: 0.2, maxFactor: 3, integer: true },
    positionSizeUsd: { minFactor: 0.5, maxFactor: 2 },
    buyZonePct: { minFactor: 0.4, maxFactor: 2.5 },
    targetRangePct: { minFactor: 0.5, maxFactor: 1.4 },
    stopBufferPct: { minFactor: 0.4, maxFactor: 2.5 },
    hardStopPct: { minFactor: 0.5, maxFactor: 3 },
    minRangePct: { minFactor: 0.3, maxFactor: 4 },
    maxTrendEfficiency: { minFactor: 0.4, maxFactor: 2 },
    maxHoldMinutes: { minFactor: 0.3, maxFactor: 3, integer: true },
  },
  "confluence-scalper": {
    emaFastPeriod: { minFactor: 0.5, maxFactor: 2, integer: true },
    emaSlowPeriod: { minFactor: 0.5, maxFactor: 2, integer: true },
    bollingerPeriod: { minFactor: 0.5, maxFactor: 2, integer: true },
    bollingerStdDev: { minFactor: 0.6, maxFactor: 1.6 },
    rsiPeriod: { minFactor: 0.5, maxFactor: 2, integer: true },
    oversoldRsi: { minFactor: 0.7, maxFactor: 1.3 },
    overboughtRsi: { minFactor: 0.85, maxFactor: 1.1 },
    takeProfitPct: { minFactor: 0.3, maxFactor: 4 },
    stopLossPct: { minFactor: 0.3, maxFactor: 4 },
    maxHoldMinutes: { minFactor: 0.3, maxFactor: 3, integer: true },
    positionSizeUsd: { minFactor: 0.5, maxFactor: 2 },
  },
  "fee-aware-scalper": {
    positionSizeUsd: { minFactor: 0.5, maxFactor: 3 },
    smaPeriod: { minFactor: 0.4, maxFactor: 3, integer: true },
    dipPct: { minFactor: 0.3, maxFactor: 4 },
    minProfitMultiple: { minFactor: 0.6, maxFactor: 2.5 },
    stopLossMultiple: { minFactor: 0.5, maxFactor: 2.5 },
    maxHoldMinutes: { minFactor: 0.3, maxFactor: 3, integer: true },
  },
  "dip-reversion": {
    lookbackMinutes: { minFactor: 0.3, maxFactor: 1, integer: true }, // default is already at the 180min clamp ceiling
    dipThresholdPct: { minFactor: 0.5, maxFactor: 2.5 },
    targetBouncePct: { minFactor: 0.4, maxFactor: 2.5 },
    holdMinutes: { minFactor: 0.3, maxFactor: 2, integer: true },
    hardStopPct: { minFactor: 0.5, maxFactor: 2 },
    reentryCooldownMinutes: { minFactor: 0.3, maxFactor: 2.5, integer: true },
    positionSizeUsd: { minFactor: 0.5, maxFactor: 2 },
  },
  "flash-dip-reversal": {
    lookbackMinutes: { minFactor: 0.5, maxFactor: 2.25, integer: true },
    dropThresholdPct: { minFactor: 0.5, maxFactor: 2.5 },
    concentrationFraction: { minFactor: 0.6, maxFactor: 1.5 },
    targetBouncePct: { minFactor: 0.4, maxFactor: 2.5 },
    holdMinutes: { minFactor: 0.3, maxFactor: 2, integer: true },
    hardStopPct: { minFactor: 0.5, maxFactor: 2 },
    reentryCooldownMinutes: { minFactor: 0.3, maxFactor: 2.5, integer: true },
    positionSizeUsd: { minFactor: 0.5, maxFactor: 2 },
  },
  "double-bottom-retest": {
    lookbackMinutes: { minFactor: 0.5, maxFactor: 1.67, integer: true },
    bouncePct: { minFactor: 0.5, maxFactor: 2 },
    retestTolerancePct: { minFactor: 0.5, maxFactor: 3 },
    targetBouncePct: { minFactor: 0.4, maxFactor: 2.5 },
    holdMinutes: { minFactor: 0.3, maxFactor: 2, integer: true },
    hardStopPct: { minFactor: 0.5, maxFactor: 2 },
    reentryCooldownMinutes: { minFactor: 0.3, maxFactor: 2.5, integer: true },
    positionSizeUsd: { minFactor: 0.5, maxFactor: 2 },
  },
};

export interface SweepTrial {
  params: Record<string, number>;
  tuning: BacktestMetrics;
  validation: BacktestMetrics | null;
  score: number;
}

export interface SweepResult {
  strategyId: BuiltInStrategyId;
  baseline: SweepTrial;
  best: SweepTrial | null;
  topTrials: SweepTrial[];
}

function randomInRange(min: number, max: number): number {
  return min + Math.random() * (max - min);
}

function sampleParams(strategyId: BuiltInStrategyId, defaults: Record<string, number>, priceRange: { min: number; max: number }, excludeParams: Set<string>): Record<string, number> {
  const specs = PARAM_SPECS[strategyId] ?? {};
  const params: Record<string, number> = { ...defaults };

  for (const [key, spec] of Object.entries(specs)) {
    if (excludeParams.has(key)) continue;
    const base = defaults[key] ?? 0;
    if (base === 0) continue; // absolute-price params (grid's lowerPrice/upperPrice) — set below
    const value = randomInRange(base * spec.minFactor, base * spec.maxFactor);
    params[key] = spec.integer ? Math.max(1, Math.round(value)) : Math.round(value * 10_000) / 10_000;
  }

  if (strategyId === "grid") {
    const mean = (priceRange.min + priceRange.max) / 2;
    const a = randomInRange(priceRange.min * 0.9, mean);
    const b = randomInRange(mean, priceRange.max * 1.1);
    params.lowerPrice = Math.min(a, b);
    params.upperPrice = Math.max(a, b);
  }

  return params;
}

export interface RunSweepOptions {
  /** Random-search trial budget (plus the shipped defaults, always evaluated as trial 0). */
  trials?: number;
  /** A candidate's round-trip count must reach this to be eligible for "best" — filters out
   * configs that only "won" via a single lucky trade. */
  minTrades?: number;
  /** Fraction of the chronological tick series held out as an out-of-sample validation
   * window, never used during the search itself. */
  validationHoldoutFraction?: number;
  /** Param names to hold fixed at the shipped default instead of sweeping — used to search
   * only "live-safe" params (see TICK_COUNT_PARAMS) when a period param can't be applied to
   * the live engine regardless of what the backtest finds (see docs/ARCHITECTURE.md). */
  excludeParams?: string[];
}

/** Random-search parameter tuning for one strategy against a chronological price series.
 * The series is split into a tuning window (searched) and a held-out validation window
 * (only used to confirm the winner still performs) to guard against overfitting to a single
 * historical run — see docs/ARCHITECTURE.md "Backtesting" for the full rationale. */
export function runSweep(strategyId: BuiltInStrategyId, ticks: PriceTick[], limits: RiskLimits, startingCashUsd: number, options: RunSweepOptions = {}): SweepResult {
  const { trials = 150, minTrades = 5, validationHoldoutFraction = 0.3, excludeParams = [] } = options;
  const excludeSet = new Set(excludeParams);

  const splitIndex = Math.floor(ticks.length * (1 - validationHoldoutFraction));
  const tuningTicks = ticks.slice(0, splitIndex);
  const validationTicks = ticks.slice(splitIndex);

  // Loop rather than Math.min(...spread): spreading a large tick series into arguments
  // overflows the call stack past ~65k elements (hit for real at 90 days of 1m candles =
  // 129,600 ticks; every earlier sweep was <=64,800 and just missed it).
  const priceRange = { min: Infinity, max: -Infinity };
  for (const t of ticks) {
    if (t.priceUsd < priceRange.min) priceRange.min = t.priceUsd;
    if (t.priceUsd > priceRange.max) priceRange.max = t.priceUsd;
  }
  const defaults = strategyMetadata[strategyId].defaultParams;

  function evaluate(params: Record<string, number>): SweepTrial {
    const tuningMetrics = computeMetrics(runBacktest(strategyId, params, tuningTicks, limits, startingCashUsd));
    const score = scoreResult(tuningMetrics, minTrades);
    const validation = validationTicks.length > 0 ? computeMetrics(runBacktest(strategyId, params, validationTicks, limits, startingCashUsd)) : null;
    return { params, tuning: tuningMetrics, validation, score };
  }

  const baseline = evaluate(defaults);
  const candidates: SweepTrial[] = [baseline];
  for (let i = 0; i < trials; i++) {
    candidates.push(evaluate(sampleParams(strategyId, defaults, priceRange, excludeSet)));
  }

  const ranked = candidates.filter((c) => Number.isFinite(c.score)).sort((a, b) => b.score - a.score);

  // Prefer a winner that also isn't a disaster out-of-sample — a config that only worked
  // on the exact window it was searched against is exactly what the validation split
  // exists to catch.
  const validated = ranked.filter((c) => !c.validation || scoreResult(c.validation, 0) > -10);
  const best = validated[0] ?? ranked[0] ?? null;

  return { strategyId, baseline, best, topTrials: ranked.slice(0, 10) };
}
