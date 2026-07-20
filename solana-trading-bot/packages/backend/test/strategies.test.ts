import { describe, expect, it } from "vitest";
import type { PriceTick, StrategyConfig } from "@trading-bot/shared";
import { DcaStrategy } from "../src/strategy-engine/strategies/dca.js";
import { MomentumStrategy } from "../src/strategy-engine/strategies/momentum.js";
import type { StrategyContext } from "../src/strategy-engine/StrategyBase.js";

const TOKEN_MINT = "So11111111111111111111111111111111111111";
const TOKEN_SYMBOL = "SOL";

function tick(priceUsd: number, offsetMs = 0): PriceTick {
  return { tokenMint: TOKEN_MINT, tokenSymbol: TOKEN_SYMBOL, priceUsd, timestamp: new Date(offsetMs).toISOString() };
}

function dcaConfig(params: Record<string, number>): StrategyConfig {
  return {
    id: "cfg-1",
    strategyId: "dca",
    tokenMint: TOKEN_MINT,
    tokenSymbol: TOKEN_SYMBOL,
    params,
    active: true,
    createdAt: new Date(0).toISOString(),
  };
}

function momentumConfig(params: Record<string, number>): StrategyConfig {
  return {
    id: "cfg-2",
    strategyId: "momentum",
    tokenMint: TOKEN_MINT,
    tokenSymbol: TOKEN_SYMBOL,
    params,
    active: true,
    createdAt: new Date(0).toISOString(),
  };
}

describe("DcaStrategy", () => {
  const strategy = new DcaStrategy();

  it("buys immediately when it has never signaled before", () => {
    const ctx: StrategyContext = {
      config: dcaConfig({ intervalMinutes: 60, amountUsd: 100 }),
      priceHistory: [tick(10)],
      latestPrice: tick(10),
      now: new Date(),
      currentPosition: null,
      lastSignalAt: null,
    };
    const signal = strategy.onInterval(ctx);
    expect(signal).not.toBeNull();
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(100);
  });

  it("holds when the interval has not elapsed yet", () => {
    const now = new Date();
    const ctx: StrategyContext = {
      config: dcaConfig({ intervalMinutes: 60, amountUsd: 100 }),
      priceHistory: [tick(10)],
      latestPrice: tick(10),
      now,
      currentPosition: null,
      lastSignalAt: new Date(now.getTime() - 5 * 60_000), // 5 minutes ago
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("buys again once the interval has elapsed", () => {
    const now = new Date();
    const ctx: StrategyContext = {
      config: dcaConfig({ intervalMinutes: 60, amountUsd: 100 }),
      priceHistory: [tick(10)],
      latestPrice: tick(10),
      now,
      currentPosition: null,
      lastSignalAt: new Date(now.getTime() - 61 * 60_000), // 61 minutes ago
    };
    const signal = strategy.onInterval(ctx);
    expect(signal?.action).toBe("buy");
  });

  it("takes profit when the position has gained enough, ignoring the interval", () => {
    const now = new Date();
    const ctx: StrategyContext = {
      config: dcaConfig({ intervalMinutes: 60, amountUsd: 100, takeProfitPct: 20 }),
      priceHistory: [tick(13)],
      latestPrice: tick(13), // 30% above entry of 10
      now,
      currentPosition: { quantity: 10, avgEntryPriceUsd: 10 },
      lastSignalAt: new Date(now.getTime() - 5 * 60_000), // recent, would otherwise hold
    };
    const signal = strategy.onInterval(ctx);
    expect(signal?.action).toBe("sell");
    expect(signal?.sizeUsd).toBeCloseTo(130, 5);
  });
});

describe("MomentumStrategy", () => {
  const strategy = new MomentumStrategy();

  it("holds when there isn't enough price history yet", () => {
    const ctx: StrategyContext = {
      config: momentumConfig({ lookbackPeriods: 20, breakoutPct: 1, positionSizeUsd: 200 }),
      priceHistory: [tick(10), tick(11)],
      latestPrice: tick(11),
      now: new Date(),
      currentPosition: null,
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("holds when price has not broken out above the period high", () => {
    const history = Array.from({ length: 21 }, (_, i) => tick(10 + i * 0.01));
    const ctx: StrategyContext = {
      config: momentumConfig({ lookbackPeriods: 20, breakoutPct: 1, positionSizeUsd: 200 }),
      priceHistory: history,
      latestPrice: history[history.length - 1] as PriceTick,
      now: new Date(),
      currentPosition: null,
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("buys on a breakout above the N-period high plus the breakout margin", () => {
    const window = Array.from({ length: 20 }, () => tick(10)); // flat period high of 10
    const breakout = tick(10.5); // > 10 * 1.01
    const ctx: StrategyContext = {
      config: momentumConfig({ lookbackPeriods: 20, breakoutPct: 1, positionSizeUsd: 200 }),
      priceHistory: [...window, breakout],
      latestPrice: breakout,
      now: new Date(),
      currentPosition: null,
      lastSignalAt: null,
    };
    const signal = strategy.onInterval(ctx);
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(200);
  });

  it("does not pyramid into an existing position below take-profit", () => {
    const window = Array.from({ length: 20 }, () => tick(10));
    const breakout = tick(10.5);
    const ctx: StrategyContext = {
      config: momentumConfig({ lookbackPeriods: 20, breakoutPct: 1, positionSizeUsd: 200, takeProfitPct: 15 }),
      priceHistory: [...window, breakout],
      latestPrice: breakout,
      now: new Date(),
      currentPosition: { quantity: 20, avgEntryPriceUsd: 10 }, // only +5% gain, below 15% target
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("takes profit once the held position hits the take-profit target", () => {
    const ctx: StrategyContext = {
      config: momentumConfig({ lookbackPeriods: 20, breakoutPct: 1, positionSizeUsd: 200, takeProfitPct: 15 }),
      priceHistory: [tick(11.6)],
      latestPrice: tick(11.6), // +16% above entry of 10
      now: new Date(),
      currentPosition: { quantity: 20, avgEntryPriceUsd: 10 },
      lastSignalAt: null,
    };
    const signal = strategy.onInterval(ctx);
    expect(signal?.action).toBe("sell");
    expect(signal?.sizeUsd).toBeCloseTo(232, 5);
  });
});
