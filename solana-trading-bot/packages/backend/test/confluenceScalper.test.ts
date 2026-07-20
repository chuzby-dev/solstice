import { describe, expect, it } from "vitest";
import type { PriceTick, StrategyConfig } from "@trading-bot/shared";
import { ConfluenceScalperStrategy } from "../src/strategy-engine/strategies/confluenceScalper.js";
import type { StrategyContext } from "../src/strategy-engine/StrategyBase.js";

const TOKEN_MINT = "So11111111111111111111111111111111111111";
const TOKEN_SYMBOL = "SOL";
const NOW = new Date("2026-01-01T00:00:00.000Z");

function tick(priceUsd: number, index: number): PriceTick {
  // Spacing doesn't matter for this strategy (tick-count periods, not time windows) —
  // only order does. Space ticks out anyway so timestamps are strictly increasing.
  return { tokenMint: TOKEN_MINT, tokenSymbol: TOKEN_SYMBOL, priceUsd, timestamp: new Date(NOW.getTime() - (1000 - index) * 1000).toISOString() };
}

const defaultParams = {
  emaFastPeriod: 9,
  emaSlowPeriod: 21,
  bollingerPeriod: 20,
  bollingerStdDev: 2,
  rsiPeriod: 9,
  oversoldRsi: 45,
  overboughtRsi: 70,
  takeProfitPct: 0.3,
  stopLossPct: 0.15,
  maxHoldMinutes: 8,
  positionSizeUsd: 100,
};

function configWith(params: Record<string, number>): StrategyConfig {
  return {
    id: "cfg-confluence",
    strategyId: "confluence-scalper",
    tokenMint: TOKEN_MINT,
    tokenSymbol: TOKEN_SYMBOL,
    params,
    active: true,
    createdAt: new Date(0).toISOString(),
  };
}

function ctxWith(history: PriceTick[], overrides: Partial<StrategyContext> = {}): StrategyContext {
  return {
    config: configWith(defaultParams),
    priceHistory: history,
    latestPrice: history[history.length - 1]!,
    now: NOW,
    currentPosition: null,
    lastSignalAt: null,
    ...overrides,
  };
}

/** 25-tick gentle uptrend (98 -> 99.92, keeps Bollinger bands narrow) followed by a
 * 5-tick pullback that dips RSI to 41.14 (below the 45 oversoldRsi threshold) then
 * ticks back up on the last bar — a verified (via a throwaway debug script run against
 * the real indicator functions, not hand-derived) confluence buy setup:
 * EMA9(99.1879) > EMA21(99.0888) uptrend, RSI 41.14 <= 45, price 98.9 > previous 98.6. */
function pullbackBuyWindow(): PriceTick[] {
  const rise = Array.from({ length: 25 }, (_, i) => 98 + i * 0.08);
  const pullback = [99.7, 99.3, 98.9, 98.6, 98.9];
  return [...rise, ...pullback].map((p, i) => tick(p, i));
}

/** Flat-then-declining 31-tick series where EMA9 crosses below EMA21 as soon as the
 * decline starts (tick 23 onward is a confirmed downtrend by this indicator pair). */
function downtrendWindow(): PriceTick[] {
  const flat = Array.from({ length: 21 }, () => 100);
  const decline = Array.from({ length: 10 }, (_, i) => 100 - i * 0.3);
  return [...flat, ...decline].map((p, i) => tick(p, i));
}

describe("ConfluenceScalperStrategy — entry", () => {
  const strategy = new ConfluenceScalperStrategy();

  it("holds when there isn't enough history for all four indicators yet", () => {
    const history = Array.from({ length: 15 }, (_, i) => tick(100 + i * 0.1, i));
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("buys on confirmed confluence: uptrend + RSI pullback + turning up", () => {
    const signal = strategy.onInterval(ctxWith(pullbackBuyWindow()));
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(100);
    expect(signal?.reason).toContain("Confluence entry");
  });

  it("holds when price is in the pullback zone but still falling (no confirmation tick)", () => {
    // Same window one tick earlier: RSI already oversold (28.53) but price is still
    // falling (98.6 < previous 98.9), so the confirmation filter blocks entry.
    const full = pullbackBuyWindow();
    const history = full.slice(0, full.length - 1); // drop the final up-tick
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when the trend filter is against the pullback, even if RSI would otherwise qualify", () => {
    // Downtrend window: entry is blocked by the trend filter before RSI/Bollinger are
    // even consulted.
    expect(strategy.onInterval(ctxWith(downtrendWindow()))).toBeNull();
  });
});

describe("ConfluenceScalperStrategy — exit", () => {
  const strategy = new ConfluenceScalperStrategy();
  const twoMinAgo = new Date(NOW.getTime() - 2 * 60_000);

  it("takes profit once price clears the target", () => {
    const history = pullbackBuyWindow(); // latest price 98.9
    const holding = { quantity: 2, avgEntryPriceUsd: 98.0 }; // target = 98 * 1.003 = 98.294
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: twoMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Take-profit");
    expect(signal?.sizeUsd).toBeCloseTo(197.8, 5);
  });

  it("cuts the loss at the stop", () => {
    const history = pullbackBuyWindow(); // latest price 98.9
    const holding = { quantity: 2, avgEntryPriceUsd: 99.5 }; // stop = 99.5 * 0.9985 = 99.3507
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: twoMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Stop-loss");
    expect(signal?.sizeUsd).toBeCloseTo(197.8, 5);
  });

  it("exits immediately once the trend flips, independent of stop/target", () => {
    const history = downtrendWindow().slice(0, -1); // 30 ticks -> latest price 97.6, downtrend
    const holding = { quantity: 2, avgEntryPriceUsd: 97.6 }; // price sits between stop and target
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: twoMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Trend invalidated");
  });

  it("holds a healthy position that hasn't hit stop, target, or a trend flip", () => {
    const history = pullbackBuyWindow(); // latest price 98.9, uptrend intact
    const holding = { quantity: 2, avgEntryPriceUsd: 98.85 }; // both stop and target are out of reach
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: twoMinAgo }))).toBeNull();
  });

  it("exits via the time stop once the scalp thesis has expired", () => {
    const history = pullbackBuyWindow();
    const holding = { quantity: 2, avgEntryPriceUsd: 98.85 };
    const nineMinAgo = new Date(NOW.getTime() - 9 * 60_000); // > 8min maxHoldMinutes
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: nineMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Time stop");
  });

  it("skips the time stop gracefully when the entry time is unknown", () => {
    const history = pullbackBuyWindow();
    const holding = { quantity: 2, avgEntryPriceUsd: 98.85 };
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: null }))).toBeNull();
  });
});
