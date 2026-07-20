import { describe, expect, it } from "vitest";
import type { PriceTick, StrategyConfig } from "@trading-bot/shared";
import { RangeScalperStrategy } from "../src/strategy-engine/strategies/rangeScalper.js";
import type { StrategyContext } from "../src/strategy-engine/StrategyBase.js";

const TOKEN_MINT = "So11111111111111111111111111111111111111";
const TOKEN_SYMBOL = "SOL";
const NOW = new Date("2026-01-01T00:00:00.000Z");

function tick(priceUsd: number, secondsAgo: number): PriceTick {
  return { tokenMint: TOKEN_MINT, tokenSymbol: TOKEN_SYMBOL, priceUsd, timestamp: new Date(NOW.getTime() - secondsAgo * 1000).toISOString() };
}

const defaultParams = {
  windowMinutes: 5,
  positionSizeUsd: 100,
  buyZonePct: 20,
  targetRangePct: 70,
  stopBufferPct: 10,
  hardStopPct: 2,
  minRangePct: 0.3,
  maxTrendEfficiency: 0.35,
  maxHoldMinutes: 10,
};

function configWith(params: Record<string, number>): StrategyConfig {
  return {
    id: "cfg-scalper",
    strategyId: "range-scalper",
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

/** Choppy window with a dip that then ticks back up:
 * prices [100.4, 101.2, 100.0, 99.8, 100.0] -> low 99.8, high 101.2, range 1.4;
 * efficiency = 0.4/2.4 ~ 0.17 (range-bound, passes regime filter);
 * buy zone threshold = 99.8 + 1.4*0.20 = 100.08 (price 100.0 qualifies);
 * target = 99.8 + 1.4*0.70 = 100.78, stop = 99.8 - 1.4*0.10 = 99.66 ->
 * reward 0.78, risk 0.34, R:R 2.29 (clears the 1.2x gate). */
function validBuyWindow(): PriceTick[] {
  return [tick(100.4, 240), tick(101.2, 180), tick(100.0, 120), tick(99.8, 60), tick(100.0, 0)];
}

describe("RangeScalperStrategy — entry filters", () => {
  const strategy = new RangeScalperStrategy();

  it("buys a confirmed turn near the range low in a choppy (non-trending) window", () => {
    const signal = strategy.onInterval(ctxWith(validBuyWindow()));
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(100);
    expect(signal?.reason).toContain("R:R");
  });

  it("holds when price is in the buy zone but still falling (no confirmation tick)", () => {
    const history = [tick(100.4, 240), tick(101.2, 180), tick(100.2, 120), tick(100.0, 60), tick(99.8, 0)];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds in a trending market even when price is in the buy zone and turning up (regime filter)", () => {
    const history = [tick(103, 240), tick(102, 180), tick(101, 120), tick(100, 60), tick(100.15, 0)];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when the window's range is too thin to cover trading costs", () => {
    const history = [tick(100, 240), tick(100.05, 180), tick(100.02, 120), tick(100.06, 60), tick(100.04, 0)];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds during the re-entry cooldown (half a window since the last trade)", () => {
    const ctx = ctxWith(validBuyWindow(), { lastSignalAt: new Date(NOW.getTime() - 30_000) }); // 5min window -> 150s cooldown
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("buys again once the cooldown has passed", () => {
    const ctx = ctxWith(validBuyWindow(), { lastSignalAt: new Date(NOW.getTime() - 200_000) }); // 200s > 150s
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });

  it("holds when the setup's reward:risk is below the 1.2x gate", () => {
    // target = 99.8 + 1.4*0.25 = 100.15 -> reward 0.15; stop = 99.8 - 1.4*0.50 = 99.10 -> risk 0.90; R:R 0.17
    const ctx = ctxWith(validBuyWindow(), { config: configWith({ ...defaultParams, targetRangePct: 25, stopBufferPct: 50 }) });
    expect(strategy.onInterval(ctx)).toBeNull();
  });
});

describe("RangeScalperStrategy — window clamping (1-15 min)", () => {
  const strategy = new RangeScalperStrategy();

  it("clamps windowMinutes above 15 down to 15, excluding older ticks", () => {
    const history = [tick(100.4, 1200), tick(101.2, 1140), tick(100.0, 1080), tick(99.8, 1020), tick(100.0, 0)];
    const ctx = ctxWith(history, { config: configWith({ ...defaultParams, windowMinutes: 60 }) });
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("clamps windowMinutes below 1 up to 1, keeping ticks from the last minute", () => {
    const history = [tick(100.4, 50), tick(101.2, 40), tick(100.0, 30), tick(99.8, 20), tick(100.0, 0)];
    const ctx = ctxWith(history, { config: configWith({ ...defaultParams, windowMinutes: 0.1 }) });
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });
});

describe("RangeScalperStrategy — exit management", () => {
  const strategy = new RangeScalperStrategy();
  const holding = { quantity: 2, avgEntryPriceUsd: 100 };
  const twoMinAgo = new Date(NOW.getTime() - 2 * 60_000);

  it("fires the hard stop immediately, even with only one tick of data (data-gap resilience)", () => {
    const history = [tick(97.9, 0)]; // hard stop = 100 * (1 - 2%) = 98.0
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: twoMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Hard stop");
    expect(signal?.sizeUsd).toBeCloseTo(195.8, 5);
  });

  it("holds when price is above the hard stop and there's not enough data for the range checks", () => {
    const history = [tick(99, 0)];
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: twoMinAgo }))).toBeNull();
  });

  it("fires the range stop once price breaks below the established (pre-existing) range low", () => {
    // Reference ticks (excluding the latest) establish low=99, high=101, range=2.
    // stop = 99 - 2*0.10 = 98.8. Latest tick breaks below that but stays above the
    // hard stop (98.0), proving the range stop is reachable and independent of it.
    const history = [tick(101, 240), tick(100.5, 180), tick(99, 120), tick(99.5, 60), tick(98.5, 0)];
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: twoMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Range stop");
    expect(signal?.sizeUsd).toBeCloseTo(197, 5);
  });

  it("does NOT fire the range stop from a self-referential range (regression: a stop derived from a window including the current tick can never fire)", () => {
    // Same shape as the reachable case above, but the price break is the tick that
    // WOULD define the window low if the window included it — this must still resolve
    // via the reference-range (pre-existing ticks only), which does not include it,
    // so the stop must be evaluated against ticks 240/180/120/60s ago only.
    const history = [tick(101, 240), tick(100, 180), tick(99, 120), tick(99.5, 60), tick(99.4, 0)];
    // reference range: low 99, high 101, range 2 -> stop = 98.8; price 99.4 > 98.8, so no stop.
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: twoMinAgo }));
    expect(signal).toBeNull();
  });

  it("holds a profitable position below the range target, even though it's an elevated price (regression: target must not fire on any bare new high)", () => {
    // Reference ticks establish low=99, high=101, range=2 -> target = 99 + 2*0.70 = 100.4.
    // Latest price (100.3) is profitable and not even a new high vs the reference, but
    // is below the actual computed target -> must hold, not exit.
    const history = [tick(99, 240), tick(100, 180), tick(101, 120), tick(100.2, 60), tick(100.3, 0)];
    const ctx = ctxWith(history, { currentPosition: { quantity: 2, avgEntryPriceUsd: 99.5 }, lastSignalAt: twoMinAgo });
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("fires the range target once price actually clears the established target level", () => {
    const history = [tick(99, 240), tick(100, 180), tick(101, 120), tick(100.2, 60), tick(100.5, 0)];
    const ctx = ctxWith(history, { currentPosition: { quantity: 2, avgEntryPriceUsd: 99.5 }, lastSignalAt: twoMinAgo });
    const signal = strategy.onInterval(ctx);
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Range target");
    expect(signal?.sizeUsd).toBeCloseTo(201, 5);
  });

  it("exits via the time stop once the scalp thesis has expired", () => {
    const history = [tick(100.1, 0)];
    const elevenMinAgo = new Date(NOW.getTime() - 11 * 60_000);
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: elevenMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Time stop");
  });

  it("holds a healthy position that has hit neither stop, target, nor time limit", () => {
    const history = [tick(100.1, 0)];
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: twoMinAgo }))).toBeNull();
  });

  it("skips the time stop gracefully when the entry time is unknown (position opened by another strategy)", () => {
    const history = [tick(100.1, 0)];
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: null }))).toBeNull();
  });
});
