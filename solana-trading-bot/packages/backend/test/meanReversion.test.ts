import { describe, expect, it } from "vitest";
import type { PriceTick, StrategyConfig } from "@trading-bot/shared";
import { MeanReversionStrategy } from "../src/strategy-engine/strategies/meanReversion.js";
import type { StrategyContext } from "../src/strategy-engine/StrategyBase.js";

const TOKEN_MINT = "So11111111111111111111111111111111111111";
const TOKEN_SYMBOL = "SOL";
const NOW = new Date("2026-01-01T00:00:00.000Z");

function tick(priceUsd: number, secondsAgo: number): PriceTick {
  return { tokenMint: TOKEN_MINT, tokenSymbol: TOKEN_SYMBOL, priceUsd, timestamp: new Date(NOW.getTime() - secondsAgo * 1000).toISOString() };
}

const defaultParams = {
  windowMinutes: 60,
  entryStdDevs: 2,
  hardStopPct: 5,
  maxHoldMinutes: 120,
  reentryCooldownMinutes: 30,
  positionSizeUsd: 150,
};

function configWith(params: Record<string, number>): StrategyConfig {
  return {
    id: "cfg-mean-reversion",
    strategyId: "mean-reversion",
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

/** Spans the full 60min default window (oldest tick exactly 3600s ago, 100% coverage).
 * Nine ticks at $100 anchor the mean; a dip to $85 then a confirming uptick to $86 —
 * mean ~= 97.36, stdDev ~= 5.6, entryStdDevs=2 -> lower band ~= 86.17, so $86 clears it
 * (86 <= 86.17) with room for float slop, and 86 > 85 confirms the turn. */
function validDipWindow(): PriceTick[] {
  return [tick(100, 3600), tick(100, 3200), tick(100, 2800), tick(100, 2400), tick(100, 2000), tick(100, 1600), tick(100, 1200), tick(100, 800), tick(100, 400), tick(85, 60), tick(86, 0)];
}

describe("MeanReversionStrategy — entry filters", () => {
  const strategy = new MeanReversionStrategy();

  it("buys a confirmed dip that clears the std-dev threshold with full window coverage", () => {
    const signal = strategy.onInterval(ctxWith(validDipWindow()));
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(150);
    expect(signal?.reason).toContain("Reversion entry");
  });

  it("holds when price is below the mean but not far enough below it", () => {
    // Same 9x$100 anchor, shallower dip to $97 then confirming uptick to $97.5 ->
    // mean ~= 99.5, stdDev ~= 1.08, lower band ~= 97.33; $97.5 stays above that.
    const history = [tick(100, 3600), tick(100, 3200), tick(100, 2800), tick(100, 2400), tick(100, 2000), tick(100, 1600), tick(100, 1200), tick(100, 800), tick(100, 400), tick(97, 60), tick(97.5, 0)];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when price is in dip territory but still falling (no confirmation tick)", () => {
    // Same magnitude dip as validDipWindow, but the last two ticks are swapped so the
    // latest tick ($85) is BELOW the previous one ($86) -- still falling, not turning up.
    const history = [tick(100, 3600), tick(100, 3200), tick(100, 2800), tick(100, 2400), tick(100, 2000), tick(100, 1600), tick(100, 1200), tick(100, 800), tick(100, 400), tick(86, 60), tick(85, 0)];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds on a perfectly flat window (zero std dev) rather than treating it as a free entry", () => {
    // Without the explicit stdDev<=0 guard, a flat series makes the lower band equal the
    // mean itself, and price==mean would trivially satisfy "price <= lower".
    const flat = Array.from({ length: 11 }, (_, i) => tick(100, (10 - i) * 300));
    expect(strategy.onInterval(ctxWith(flat))).toBeNull();
  });

  it("holds when there aren't enough ticks yet, even with full time coverage", () => {
    const history = [tick(100, 3600), tick(100, 1800), tick(85, 60), tick(86, 0)];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when the available history doesn't yet cover enough of the requested window", () => {
    // Oldest tick only 1000s old vs a 3600s (60min) window -> 28% coverage, well under
    // the 80% floor, even though the raw dip in these ticks looks big enough.
    const history = [tick(100, 1000), tick(100, 900), tick(100, 800), tick(100, 700), tick(100, 600), tick(100, 500), tick(100, 400), tick(100, 300), tick(100, 200), tick(85, 60), tick(86, 0)];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds during the re-entry cooldown since the last trade", () => {
    const ctx = ctxWith(validDipWindow(), { lastSignalAt: new Date(NOW.getTime() - 10 * 60_000) }); // 10min < 30min cooldown
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("buys again once the cooldown has passed", () => {
    const ctx = ctxWith(validDipWindow(), { lastSignalAt: new Date(NOW.getTime() - 40 * 60_000) }); // 40min > 30min cooldown
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });
});

describe("MeanReversionStrategy — window clamping (5-180 min)", () => {
  const strategy = new MeanReversionStrategy();

  it("clamps windowMinutes above 180 down to 180", () => {
    // Oldest tick 9000s (150min) old: 83% coverage of a clamped 180min (10800s) window
    // (passes), but only 50% of an (unclamped) 300min (18000s) window (would fail) -> a
    // buy here proves the clamp to 180 was actually applied.
    const history = [tick(100, 9000), tick(100, 8000), tick(100, 7000), tick(100, 6000), tick(100, 5000), tick(100, 4000), tick(100, 3000), tick(100, 2000), tick(100, 500), tick(85, 60), tick(86, 0)];
    const ctx = ctxWith(history, { config: configWith({ ...defaultParams, windowMinutes: 300 }) });
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });

  it("clamps windowMinutes below 5 up to 5", () => {
    // Oldest tick 250s old: inside a clamped 5min (300s) window (83% coverage, passes),
    // but a hypothetical unclamped 2min (120s) window would only contain ~5 of these 11
    // ticks -- too few to reach MIN_WINDOW_TICKS -- so a buy proves clamping happened.
    const history = [tick(100, 250), tick(100, 220), tick(100, 190), tick(100, 160), tick(100, 130), tick(100, 100), tick(100, 70), tick(100, 40), tick(100, 20), tick(85, 10), tick(86, 0)];
    const ctx = ctxWith(history, { config: configWith({ ...defaultParams, windowMinutes: 2 }) });
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });
});

describe("MeanReversionStrategy — exit management", () => {
  const strategy = new MeanReversionStrategy();
  const flatAt100 = [tick(100, 900), tick(100, 800), tick(100, 700), tick(100, 600), tick(100, 500), tick(100, 400), tick(100, 300), tick(100, 200), tick(100, 100), tick(100, 0)];
  const thirtyMinAgo = new Date(NOW.getTime() - 30 * 60_000);

  it("fires the hard stop immediately, even with only one tick of data (data-gap resilience)", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 100 };
    const history = [tick(95, 0)]; // hard stop = 100 * (1 - 5%) = 95
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: thirtyMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Hard stop");
    expect(signal?.sizeUsd).toBeCloseTo(190, 5);
  });

  it("sells once price reverts back up to the window mean", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 90 };
    const signal = strategy.onInterval(ctxWith(flatAt100, { currentPosition: holding, lastSignalAt: thirtyMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Reverted to");
    expect(signal?.sizeUsd).toBeCloseTo(200, 5);
  });

  it("holds a healthy position that hasn't hit the stop, the mean, or the time limit", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 90 };
    // 9 ticks at 100 + latest at 99 -> mean = 99.9; price (99) stays below it.
    const history = [tick(100, 900), tick(100, 800), tick(100, 700), tick(100, 600), tick(100, 500), tick(100, 400), tick(100, 300), tick(100, 200), tick(100, 100), tick(99, 0)];
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: thirtyMinAgo }))).toBeNull();
  });

  it("exits via the time stop once the reversion thesis has expired", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 90 };
    const history = [tick(100, 900), tick(100, 800), tick(100, 700), tick(100, 600), tick(100, 500), tick(100, 400), tick(100, 300), tick(100, 200), tick(100, 100), tick(99, 0)];
    const oneFiftyMinAgo = new Date(NOW.getTime() - 150 * 60_000); // > 120min maxHoldMinutes
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: oneFiftyMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Time stop");
  });

  it("skips the time stop gracefully when the entry time is unknown (position opened by another strategy)", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 90 };
    const history = [tick(100, 900), tick(100, 800), tick(100, 700), tick(100, 600), tick(100, 500), tick(100, 400), tick(100, 300), tick(100, 200), tick(100, 100), tick(99, 0)];
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: null }))).toBeNull();
  });
});
