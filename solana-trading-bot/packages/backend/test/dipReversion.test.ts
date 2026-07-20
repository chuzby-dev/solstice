import { describe, expect, it } from "vitest";
import type { PriceTick, StrategyConfig } from "@trading-bot/shared";
import { DipReversionStrategy } from "../src/strategy-engine/strategies/dipReversion.js";
import type { StrategyContext } from "../src/strategy-engine/StrategyBase.js";

const TOKEN_MINT = "So11111111111111111111111111111111111111";
const TOKEN_SYMBOL = "SOL";
const NOW = new Date("2026-01-01T00:00:00.000Z");

function tick(priceUsd: number, secondsAgo: number): PriceTick {
  return { tokenMint: TOKEN_MINT, tokenSymbol: TOKEN_SYMBOL, priceUsd, timestamp: new Date(NOW.getTime() - secondsAgo * 1000).toISOString() };
}

const defaultParams = {
  lookbackMinutes: 90,
  dipThresholdPct: 1.5,
  targetBouncePct: 1.2,
  holdMinutes: 90,
  hardStopPct: 4,
  reentryCooldownMinutes: 45,
  positionSizeUsd: 150,
};

function configWith(params: Record<string, number>): StrategyConfig {
  return {
    id: "cfg-dip-reversion",
    strategyId: "dip-reversion",
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

/** Spans the full 90min default window (oldest tick exactly 5400s = 90min ago, so
 * coverage is 100%). High of $102 at 60min ago, dip to $98.5 with a confirmation tick
 * ticking up from $98.0 -> dip 3.43%, clears the default 1.5% threshold. */
function validDipWindow(): PriceTick[] {
  return [tick(100, 5400), tick(102, 3600), tick(99.5, 1800), tick(98.0, 60), tick(98.5, 0)];
}

describe("DipReversionStrategy — entry filters", () => {
  const strategy = new DipReversionStrategy();

  it("buys a confirmed dip that clears the threshold with full window coverage", () => {
    const signal = strategy.onInterval(ctxWith(validDipWindow()));
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(150);
    expect(signal?.reason).toContain("Dip entry");
  });

  it("holds when the decline from the window high is below the dip threshold", () => {
    const history = [tick(100, 5400), tick(102, 3600), tick(101.8, 1800), tick(101.6, 60), tick(101.7, 0)]; // ~0.29% dip
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when price is in dip territory but still falling (no confirmation tick)", () => {
    const history = [tick(100, 5400), tick(102, 3600), tick(99.5, 1800), tick(98.5, 60), tick(98.0, 0)]; // still ticking down
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when the available history doesn't yet cover enough of the requested lookback", () => {
    // Oldest tick only 1200s (20min) old vs a 5400s (90min) window -> 22% coverage, well
    // under the 80% floor, even though the raw dip% within those ticks looks big enough.
    const history = [tick(105, 1200), tick(100, 600), tick(98, 60), tick(98.5, 0)];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds during the re-entry cooldown since the last trade", () => {
    const ctx = ctxWith(validDipWindow(), { lastSignalAt: new Date(NOW.getTime() - 20 * 60_000) }); // 20min < 45min cooldown
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("buys again once the cooldown has passed", () => {
    const ctx = ctxWith(validDipWindow(), { lastSignalAt: new Date(NOW.getTime() - 50 * 60_000) }); // 50min > 45min cooldown
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });
});

describe("DipReversionStrategy — lookback clamping (30-180 min)", () => {
  const strategy = new DipReversionStrategy();

  it("clamps lookbackMinutes above 180 down to 180", () => {
    // Oldest tick 9000s (150min) old: 83% coverage of a 180min (10800s) window (passes),
    // but only 50% of an (unclamped) 300min (18000s) window (would fail) -> a buy here
    // proves the clamp to 180 was actually applied, not the raw 300 param.
    const history = [tick(100, 9000), tick(103, 6000), tick(99, 3000), tick(97, 60), tick(97.5, 0)];
    const ctx = ctxWith(history, { config: configWith({ ...defaultParams, lookbackMinutes: 300 }) });
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });

  it("clamps lookbackMinutes below 30 up to 30", () => {
    // Oldest tick 1500s (25min) old: inside a clamped 30min (1800s) window (83% coverage,
    // passes) but would fall entirely OUTSIDE an unclamped 10min (600s) window, which
    // would leave too little data (and no real dip) to fire -> a buy proves clamping.
    const history = [tick(100, 1500), tick(103, 900), tick(99, 300), tick(97, 60), tick(97.5, 0)];
    const ctx = ctxWith(history, { config: configWith({ ...defaultParams, lookbackMinutes: 10 }) });
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });
});

describe("DipReversionStrategy — exit management", () => {
  const strategy = new DipReversionStrategy();
  const holding = { quantity: 2, avgEntryPriceUsd: 100 };
  const thirtyMinAgo = new Date(NOW.getTime() - 30 * 60_000);

  it("fires the hard stop immediately, even with only one tick of data (data-gap resilience)", () => {
    const history = [tick(95, 0)]; // hard stop = 100 * (1 - 4%) = 96
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: thirtyMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Hard stop");
    expect(signal?.sizeUsd).toBeCloseTo(190, 5);
  });

  it("holds when price is above the hard stop and below the target", () => {
    const history = [tick(100.1, 0)];
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: thirtyMinAgo }))).toBeNull();
  });

  it("fires the bounce target once price clears it", () => {
    const history = [tick(101.5, 0)]; // target = 100 * (1 + 1.2%) = 101.2
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: thirtyMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Bounce target");
    expect(signal?.sizeUsd).toBeCloseTo(203, 5);
  });

  it("exits via the time stop once the bounce thesis has expired", () => {
    const history = [tick(100.1, 0)];
    const ninetyFiveMinAgo = new Date(NOW.getTime() - 95 * 60_000);
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: ninetyFiveMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Time stop");
  });

  it("holds a healthy position that has hit neither stop, target, nor time limit", () => {
    const history = [tick(100.1, 0)];
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: thirtyMinAgo }))).toBeNull();
  });

  it("skips the time stop gracefully when the entry time is unknown (position opened by another strategy)", () => {
    const history = [tick(100.1, 0)];
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: null }))).toBeNull();
  });
});
