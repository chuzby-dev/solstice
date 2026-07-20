import { describe, expect, it } from "vitest";
import type { RiskLimits } from "@trading-bot/shared";
import { computeStopLossPrice, evaluateSignal, MIN_TRADE_USD, type RiskEvaluationInput } from "../src/execution/riskManager.js";

const limits: RiskLimits = {
  maxPositionPct: 10,
  maxDailyLossPct: 5,
  perTokenExposurePct: 25,
  defaultStopLossPct: 8,
  maxSlippageBps: 100,
  maxPriceImpactPct: 3,
};

function baseInput(overrides: Partial<RiskEvaluationInput> = {}): RiskEvaluationInput {
  return {
    action: "buy",
    requestedSizeUsd: 100,
    priceUsd: 10,
    totalPortfolioValueUsd: 10_000,
    cashUsd: 10_000,
    currentTokenExposureUsd: 0,
    currentPositionQuantity: 0,
    dailyLossUsd: 0,
    startOfDayValueUsd: 10_000,
    limits,
    assumedLiquidityUsd: 500_000,
    simulatedSlippageBps: 10,
    ...overrides,
  };
}

describe("riskManager.evaluateSignal — buy", () => {
  it("allows a trade comfortably within all limits", () => {
    const result = evaluateSignal(baseInput());
    expect(result.allowed).toBe(true);
    expect(result.adjustedSizeUsd).toBe(100);
  });

  it("shrinks a trade that exceeds maxPositionPct", () => {
    const result = evaluateSignal(baseInput({ requestedSizeUsd: 5_000 })); // 50% of portfolio, cap is 10%
    expect(result.allowed).toBe(true);
    expect(result.adjustedSizeUsd).toBe(1_000); // 10% of 10,000
  });

  it("rejects when the per-token exposure cap is already full", () => {
    const result = evaluateSignal(baseInput({ currentTokenExposureUsd: 2_500 })); // already at 25% cap
    expect(result.allowed).toBe(false);
    expect(result.triggeredGuard).toBe("perTokenExposurePct");
  });

  it("shrinks a trade that would breach the per-token exposure cap", () => {
    const result = evaluateSignal(baseInput({ requestedSizeUsd: 5_000, currentTokenExposureUsd: 2_000 }));
    // room to exposure cap = 2,500 - 2,000 = 500; but maxPositionPct caps to 1,000 first,
    // so exposure cap (500) is the binding constraint
    expect(result.allowed).toBe(true);
    expect(result.adjustedSizeUsd).toBe(500);
  });

  it("rejects outright when the daily loss limit has been breached", () => {
    const result = evaluateSignal(baseInput({ dailyLossUsd: 600, startOfDayValueUsd: 10_000 })); // limit is 5% = 500
    expect(result.allowed).toBe(false);
    expect(result.triggeredGuard).toBe("maxDailyLossPct");
  });

  it("shrinks a trade that would exceed the assumed-liquidity price-impact ceiling", () => {
    const result = evaluateSignal(
      baseInput({ requestedSizeUsd: 50_000, totalPortfolioValueUsd: 1_000_000, cashUsd: 1_000_000, assumedLiquidityUsd: 500_000 }),
    );
    // maxPriceImpactPct 3% of 500,000 liquidity = 15,000 ceiling
    expect(result.allowed).toBe(true);
    expect(result.adjustedSizeUsd).toBe(15_000);
  });

  it("rejects outright when simulated slippage exceeds the slippage ceiling", () => {
    const tightLimits: RiskLimits = { ...limits, maxSlippageBps: 5 };
    const result = evaluateSignal(baseInput({ limits: tightLimits, simulatedSlippageBps: 10 }));
    expect(result.allowed).toBe(false);
    expect(result.triggeredGuard).toBe("maxSlippageBps");
  });

  it("shrinks a trade to available cash", () => {
    const result = evaluateSignal(baseInput({ requestedSizeUsd: 500, cashUsd: 200 }));
    expect(result.allowed).toBe(true);
    expect(result.adjustedSizeUsd).toBe(200);
  });

  it("rejects when the resulting size falls below the minimum trade size", () => {
    const result = evaluateSignal(baseInput({ requestedSizeUsd: 500, cashUsd: 0.5 }));
    expect(result.allowed).toBe(false);
    expect(result.triggeredGuard).toBe("insufficient_balance");
  });
});

describe("riskManager.evaluateSignal — sell", () => {
  it("caps a sell to the held position size", () => {
    const result = evaluateSignal(
      baseInput({ action: "sell", requestedSizeUsd: 1_000, priceUsd: 10, currentPositionQuantity: 50 }), // 50 * 10 = 500 held
    );
    expect(result.allowed).toBe(true);
    expect(result.adjustedSizeUsd).toBe(500);
  });

  it("rejects a sell when there is no position", () => {
    const result = evaluateSignal(baseInput({ action: "sell", requestedSizeUsd: 100, currentPositionQuantity: 0 }));
    expect(result.allowed).toBe(false);
    expect(result.triggeredGuard).toBe("insufficient_balance");
  });

  it("rejects a sell whose slippage exceeds the ceiling even if a position exists", () => {
    const tightLimits: RiskLimits = { ...limits, maxSlippageBps: 5 };
    const result = evaluateSignal(
      baseInput({ action: "sell", limits: tightLimits, simulatedSlippageBps: 10, currentPositionQuantity: 100, priceUsd: 10 }),
    );
    expect(result.allowed).toBe(false);
    expect(result.triggeredGuard).toBe("maxSlippageBps");
  });
});

describe("computeStopLossPrice", () => {
  it("computes the mandatory protective stop-loss below entry price", () => {
    expect(computeStopLossPrice(100, limits)).toBeCloseTo(92, 5); // 8% below entry
  });
});

describe("MIN_TRADE_USD", () => {
  it("is a small positive floor", () => {
    expect(MIN_TRADE_USD).toBeGreaterThan(0);
    expect(MIN_TRADE_USD).toBeLessThan(10);
  });
});
