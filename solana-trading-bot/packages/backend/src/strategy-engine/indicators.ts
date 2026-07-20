// Pure technical-indicator functions operating on a closing-price series (oldest first).
// No DB/network access — kept pure so they're trivially unit-testable and reusable from
// any strategy or a future backtester.

export function sma(values: number[], period: number): number | null {
  if (values.length < period) return null;
  const window = values.slice(-period);
  return window.reduce((a, b) => a + b, 0) / period;
}

/** Full EMA series for `values`, seeded with the SMA of the first `period` values.
 * `series[0]` corresponds to `values[period - 1]`. Returns [] if not enough data. */
export function emaSeries(values: number[], period: number): number[] {
  if (values.length < period) return [];
  const k = 2 / (period + 1);
  const seed = values.slice(0, period).reduce((a, b) => a + b, 0) / period;
  const result: number[] = [seed];
  for (let i = period; i < values.length; i++) {
    const prev = result[result.length - 1]!;
    result.push(values[i]! * k + prev * (1 - k));
  }
  return result;
}

export function emaLatest(values: number[], period: number): number | null {
  const series = emaSeries(values, period);
  return series.length ? series[series.length - 1]! : null;
}

/** Wilder's RSI over the full provided series (standard 14-period definition, but
 * `period` is configurable). Returns null if there isn't at least `period + 1` values. */
export function rsi(values: number[], period: number): number | null {
  if (values.length < period + 1) return null;

  let avgGain = 0;
  let avgLoss = 0;
  for (let i = 1; i <= period; i++) {
    const change = values[i]! - values[i - 1]!;
    if (change >= 0) avgGain += change;
    else avgLoss -= change;
  }
  avgGain /= period;
  avgLoss /= period;

  for (let i = period + 1; i < values.length; i++) {
    const change = values[i]! - values[i - 1]!;
    const gain = change > 0 ? change : 0;
    const loss = change < 0 ? -change : 0;
    avgGain = (avgGain * (period - 1) + gain) / period;
    avgLoss = (avgLoss * (period - 1) + loss) / period;
  }

  if (avgLoss === 0) return 100;
  const rs = avgGain / avgLoss;
  return 100 - 100 / (1 + rs);
}

export interface MacdResult {
  macdLine: number;
  signalLine: number;
  histogram: number;
}

/** Standard MACD (fast EMA - slow EMA, with an EMA-of-that-line signal). Returns null
 * until there's enough history for the slow EMA plus the signal-line EMA. */
export function macd(values: number[], fastPeriod = 12, slowPeriod = 26, signalPeriod = 9): MacdResult | null {
  if (values.length < slowPeriod + signalPeriod) return null;

  const fastSeries = emaSeries(values, fastPeriod);
  const slowSeries = emaSeries(values, slowPeriod);
  const offset = slowPeriod - fastPeriod; // fastSeries is longer; align indices to slowSeries

  const macdLineSeries: number[] = [];
  for (let i = 0; i < slowSeries.length; i++) {
    const fastIdx = i + offset;
    if (fastIdx < 0 || fastIdx >= fastSeries.length) continue;
    macdLineSeries.push(fastSeries[fastIdx]! - slowSeries[i]!);
  }
  if (macdLineSeries.length < signalPeriod) return null;

  const signalSeries = emaSeries(macdLineSeries, signalPeriod);
  if (signalSeries.length === 0) return null;

  const macdLine = macdLineSeries[macdLineSeries.length - 1]!;
  const signalLine = signalSeries[signalSeries.length - 1]!;
  return { macdLine, signalLine, histogram: macdLine - signalLine };
}

/** Average absolute tick-to-tick price change over `period` — used as an ATR stand-in.
 * True ATR needs OHLC candle data (high/low/close); our price feed only has a single
 * price per poll, so this close-to-close volatility is the closest available proxy.
 * See docs/ARCHITECTURE.md for the tradeoff. */
export function closeToCloseVolatility(values: number[], period: number): number | null {
  if (values.length < period + 1) return null;
  const window = values.slice(-(period + 1));
  let sum = 0;
  for (let i = 1; i < window.length; i++) {
    sum += Math.abs(window[i]! - window[i - 1]!);
  }
  return sum / period;
}

export interface BollingerBandsResult {
  middle: number;
  upper: number;
  lower: number;
  stdDev: number;
}

/** Standard Bollinger Bands: an SMA middle band with upper/lower bands offset by
 * `stdDevMultiplier` population standard deviations (the conventional definition used
 * by charting platforms — divides by `period`, not `period - 1`). */
export function bollingerBands(values: number[], period: number, stdDevMultiplier = 2): BollingerBandsResult | null {
  if (values.length < period) return null;
  const window = values.slice(-period);
  const mean = window.reduce((a, b) => a + b, 0) / period;
  const variance = window.reduce((sum, v) => sum + (v - mean) ** 2, 0) / period;
  const stdDev = Math.sqrt(variance);
  return {
    middle: mean,
    upper: mean + stdDev * stdDevMultiplier,
    lower: mean - stdDev * stdDevMultiplier,
    stdDev,
  };
}
