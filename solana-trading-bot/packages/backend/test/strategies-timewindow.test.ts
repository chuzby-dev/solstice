import { describe, expect, it } from "vitest";
import type { PriceTick, StrategyConfig } from "@trading-bot/shared";
import { ShortWindowGridStrategy } from "../src/strategy-engine/strategies/shortWindowGrid.js";
import type { StrategyContext } from "../src/strategy-engine/StrategyBase.js";

const TOKEN_MINT = "So11111111111111111111111111111111111111";
const TOKEN_SYMBOL = "SOL";
const NOW = new Date("2026-01-01T00:00:00.000Z");

/** Builds a tick `secondsAgo` seconds before NOW, so time-windowed strategies (which
 * filter priceHistory by real elapsed time relative to ctx.now) see consistent,
 * hand-computable data. */
function tick(priceUsd: number, secondsAgo: number): PriceTick {
  return { tokenMint: TOKEN_MINT, tokenSymbol: TOKEN_SYMBOL, priceUsd, timestamp: new Date(NOW.getTime() - secondsAgo * 1000).toISOString() };
}

function configFor(strategyId: StrategyConfig["strategyId"], params: Record<string, number>): StrategyConfig {
  return {
    id: "cfg-test",
    strategyId,
    tokenMint: TOKEN_MINT,
    tokenSymbol: TOKEN_SYMBOL,
    params,
    active: true,
    createdAt: new Date(0).toISOString(),
  };
}

describe("ShortWindowGridStrategy", () => {
  const strategy = new ShortWindowGridStrategy();
  const params = { windowMinutes: 5, gridLevels: 6, orderSizeUsd: 100 };

  it("holds when fewer than 3 ticks fall within the window", () => {
    const history = [tick(100, 280), tick(101, 200)]; // only 2, both within 5min
    const ctx: StrategyContext = {
      config: configFor("short-window-grid", params),
      priceHistory: history,
      latestPrice: history[1]!,
      now: NOW,
      currentPosition: null,
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("ignores ticks outside the rolling window", () => {
    // All ticks are older than 5 minutes (300s) ago -> 0 ticks in window
    const history = [tick(100, 400), tick(101, 350), tick(102, 320)];
    const ctx: StrategyContext = {
      config: configFor("short-window-grid", params),
      priceHistory: history,
      latestPrice: history[2]!,
      now: NOW,
      currentPosition: null,
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("holds when there's no price movement within the window", () => {
    const history = [tick(100, 280), tick(100, 200), tick(100, 120), tick(100, 40)];
    const ctx: StrategyContext = {
      config: configFor("short-window-grid", params),
      priceHistory: history,
      latestPrice: history[3]!,
      now: NOW,
      currentPosition: null,
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("buys when price crosses down through an auto-computed grid line", () => {
    // window prices [100,105,95,90] -> range [90,105], gridStep 2.5
    // previous(95) level 2, latest(90) level 0 -> crossed down
    const history = [tick(100, 280), tick(105, 200), tick(95, 120), tick(90, 40)];
    const ctx: StrategyContext = {
      config: configFor("short-window-grid", params),
      priceHistory: history,
      latestPrice: history[3]!,
      now: NOW,
      currentPosition: null,
      lastSignalAt: null,
    };
    const signal = strategy.onInterval(ctx);
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(100);
    expect(signal?.reason).toContain("auto-range");
  });

  it("sells when price crosses up through a grid line above the entry", () => {
    // window prices [90,95,100,108] -> range [90,108], gridStep 3
    // previous(100) level 3, latest(108) level 6 -> crossed up, above entry of 95
    const history = [tick(90, 280), tick(95, 200), tick(100, 120), tick(108, 40)];
    const ctx: StrategyContext = {
      config: configFor("short-window-grid", params),
      priceHistory: history,
      latestPrice: history[3]!,
      now: NOW,
      currentPosition: { quantity: 1, avgEntryPriceUsd: 95 },
      lastSignalAt: null,
    };
    const signal = strategy.onInterval(ctx);
    expect(signal?.action).toBe("sell");
    expect(signal?.sizeUsd).toBeCloseTo(108, 5);
  });

  it("sits out a quiet market whose auto-range is too thin to be worth trading fees", () => {
    // Reproduces an observed real case: a ~0.1% range over the window, well under the
    // 0.3% default minRangePct, would otherwise still produce a clean level crossing.
    const history = [tick(75.34, 280), tick(75.4269, 200), tick(75.36, 120), tick(75.3409, 40)];
    const ctx: StrategyContext = {
      config: configFor("short-window-grid", params),
      priceHistory: history,
      latestPrice: history[3]!,
      now: NOW,
      currentPosition: null,
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });
});
