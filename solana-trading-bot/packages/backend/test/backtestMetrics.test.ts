import { describe, expect, it } from "vitest";
import type { Trade } from "@trading-bot/shared";
import { computeMetrics, scoreResult, type BacktestMetrics } from "../src/backtest/metrics.js";
import type { BacktestResult } from "../src/backtest/backtestEngine.js";

function makeTrade(overrides: Partial<Trade> = {}): Trade {
  return {
    id: "t1",
    strategyConfigId: "cfg-1",
    strategyId: "momentum",
    action: "buy",
    tokenMint: "mint",
    tokenSymbol: "SOL",
    priceUsd: 100,
    sizeUsd: 1000,
    sizeToken: 10,
    feeUsd: 1,
    reason: "test",
    simulated: true,
    txHash: null,
    network: null,
    confirmationSlot: null,
    timestamp: "2026-01-01T00:00:00.000Z",
    ...overrides,
  };
}

function makeResult(overrides: Partial<BacktestResult> = {}): BacktestResult {
  return {
    strategyId: "momentum",
    params: {},
    startingCashUsd: 10_000,
    endingTotalValueUsd: 10_000,
    fills: [],
    trades: [],
    equityCurve: [{ timestamp: "2026-01-01T00:00:00.000Z", totalValueUsd: 10_000 }],
    realizedPnlUsd: 0,
    totalFeesUsd: 0,
    ...overrides,
  };
}

describe("computeMetrics", () => {
  it("computes total return from starting vs ending value", () => {
    const metrics = computeMetrics(makeResult({ endingTotalValueUsd: 11_000 }));
    expect(metrics.totalReturnUsd).toBe(1000);
    expect(metrics.totalReturnPct).toBeCloseTo(10, 6);
  });

  it("derives win rate, profit factor, and avg hold time from sell fills' realized deltas", () => {
    const buy = makeTrade({ id: "b1", action: "buy" });
    const sellWin = makeTrade({ id: "s1", action: "sell", timestamp: "2026-01-01T00:10:00.000Z" });
    const sellLoss = makeTrade({ id: "s2", action: "sell", timestamp: "2026-01-01T00:20:00.000Z" });
    const result = makeResult({
      trades: [buy, sellWin, sellLoss],
      fills: [
        { trade: buy, realizedDeltaUsd: null, holdMinutes: null },
        { trade: sellWin, realizedDeltaUsd: 50, holdMinutes: 10 },
        { trade: sellLoss, realizedDeltaUsd: -20, holdMinutes: 15 },
      ],
    });
    const metrics = computeMetrics(result);
    expect(metrics.roundTripCount).toBe(2);
    expect(metrics.winRate).toBeCloseTo(0.5, 6);
    expect(metrics.profitFactor).toBeCloseTo(50 / 20, 6);
    expect(metrics.avgHoldMinutes).toBeCloseTo(12.5, 6);
  });

  it("computes max drawdown from the equity curve's peak-to-trough", () => {
    const result = makeResult({
      equityCurve: [
        { timestamp: "t0", totalValueUsd: 10_000 },
        { timestamp: "t1", totalValueUsd: 12_000 },
        { timestamp: "t2", totalValueUsd: 9_000 },
        { timestamp: "t3", totalValueUsd: 11_000 },
      ],
    });
    expect(computeMetrics(result).maxDrawdownPct).toBeCloseTo(25, 6);
  });

  it("returns null win rate / profit factor / avg hold time when there are no round trips", () => {
    const metrics = computeMetrics(makeResult());
    expect(metrics.winRate).toBeNull();
    expect(metrics.profitFactor).toBeNull();
    expect(metrics.avgHoldMinutes).toBeNull();
  });
});

describe("scoreResult", () => {
  it("penalizes candidates below the minimum trade count with -Infinity", () => {
    const metrics = computeMetrics(makeResult());
    expect(scoreResult(metrics, 5)).toBe(Number.NEGATIVE_INFINITY);
  });

  it("rewards return and penalizes drawdown once the minimum trade count is met", () => {
    const metrics: BacktestMetrics = {
      totalReturnPct: 20,
      totalReturnUsd: 2000,
      tradeCount: 10,
      roundTripCount: 5,
      winRate: 0.6,
      profitFactor: 2,
      maxDrawdownPct: 10,
      avgHoldMinutes: 5,
      totalFeesUsd: 10,
      feeDragPct: 0.1,
    };
    expect(scoreResult(metrics, 5)).toBeCloseTo(20 - 0.5 * 10, 6);
  });
});
