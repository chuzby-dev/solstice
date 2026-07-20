import { describe, expect, it } from "vitest";
import type { PriceTick, StrategyConfig } from "@trading-bot/shared";
import { DoubleBottomRetestStrategy } from "../src/strategy-engine/strategies/doubleBottomRetest.js";
import type { StrategyContext } from "../src/strategy-engine/StrategyBase.js";

const TOKEN_MINT = "So11111111111111111111111111111111111111";
const TOKEN_SYMBOL = "SOL";
const NOW = new Date("2026-01-01T00:00:00.000Z");

function tick(priceUsd: number, secondsAgo: number): PriceTick {
  return { tokenMint: TOKEN_MINT, tokenSymbol: TOKEN_SYMBOL, priceUsd, timestamp: new Date(NOW.getTime() - secondsAgo * 1000).toISOString() };
}

const defaultParams = {
  lookbackMinutes: 90,
  bouncePct: 1,
  retestTolerancePct: 0.3,
  targetBouncePct: 1,
  holdMinutes: 60,
  hardStopPct: 2.5,
  reentryCooldownMinutes: 60,
  positionSizeUsd: 150,
};

function configWith(params: Record<string, number>): StrategyConfig {
  return {
    id: "cfg-double-bottom-retest",
    strategyId: "double-bottom-retest",
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

/** Spans the full 90min default window (oldest tick exactly 5400s ago, 100% coverage).
 * Flat at $100, dips to a first low of $98 (within the first-low search range, the older
 * 60min of the 90min window), bounces back up to $100 (a 2.04% bounce, clears the 1%
 * threshold), then pulls back down and retests $98 almost exactly ($98.0, 0% off) without
 * ever dropping below the 0.15% breakdown buffer, with a final confirming uptick from
 * $97.9 to $98.0. */
function validDoubleBottomWindow(): PriceTick[] {
  return [
    tick(100, 5400), tick(100, 4500), tick(98, 3600), tick(100, 2700), tick(99.5, 1800),
    tick(98.7, 1200), tick(98.2, 600), tick(97.9, 60), tick(98.0, 0),
  ];
}

describe("DoubleBottomRetestStrategy — entry filters", () => {
  const strategy = new DoubleBottomRetestStrategy();

  it("buys a confirmed double-bottom retest with full window coverage", () => {
    const signal = strategy.onInterval(ctxWith(validDoubleBottomWindow()));
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(150);
    expect(signal?.reason).toContain("Double-bottom entry");
  });

  it("holds when the bounce off the first low doesn't clear the threshold", () => {
    // Same shape, but the bounce only reaches $98.5 (0.51% off the $98 low) instead of $100.
    const history = [
      tick(100, 5400), tick(100, 4500), tick(98, 3600), tick(98.5, 2700), tick(98.3, 1800),
      tick(98.2, 1200), tick(98.1, 600), tick(97.9, 60), tick(98.0, 0),
    ];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when price breaks down through the first low instead of retesting it", () => {
    // Same shape as the valid window, but one tick after the bounce (at 1200s ago) drops to
    // $97.5, well below the 0.15%-buffered breakdown floor of ~$97.85.
    const history = [
      tick(100, 5400), tick(100, 4500), tick(98, 3600), tick(100, 2700), tick(99.5, 1800),
      tick(97.5, 1200), tick(98.2, 600), tick(97.9, 60), tick(98.0, 0),
    ];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when the current price isn't close enough to the first low to count as a retest", () => {
    // Valid bounce and no breakdown, but the current price ($99) is 1.02% away from the
    // $98 first low, well outside the 0.3% retest tolerance.
    const history = [
      tick(100, 5400), tick(100, 4500), tick(98, 3600), tick(100, 2700), tick(99.5, 1800),
      tick(98.7, 1200), tick(98.5, 600), tick(98.5, 60), tick(99, 0),
    ];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when price is still falling into the retest (no confirmation tick)", () => {
    // Same as the valid window but the final two ticks are swapped so the latest tick
    // ($97.95) is BELOW the previous one ($98.0) — still falling, not turning up. Both
    // values stay within the 0.3% retest band, isolating the confirmation-tick check.
    const history = [
      tick(100, 5400), tick(100, 4500), tick(98, 3600), tick(100, 2700), tick(99.5, 1800),
      tick(98.7, 1200), tick(98.2, 600), tick(98.0, 60), tick(97.95, 0),
    ];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when there aren't enough ticks yet, even with full time coverage", () => {
    const history = [tick(100, 5400), tick(98, 60)];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when the available history doesn't yet cover enough of the requested window", () => {
    // Oldest tick only 3000s old vs a 5400s (90min) window -> 56% coverage, well under the
    // 80% floor, even though the shape crammed into that span looks like a valid retest.
    const history = [
      tick(100, 3000), tick(100, 2600), tick(98, 2200), tick(100, 1800), tick(99.5, 1200),
      tick(98.7, 800), tick(98.2, 400), tick(97.9, 60), tick(98.0, 0),
    ];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds during the re-entry cooldown since the last trade", () => {
    const ctx = ctxWith(validDoubleBottomWindow(), { lastSignalAt: new Date(NOW.getTime() - 15 * 60_000) }); // 15min < 60min cooldown
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("buys again once the cooldown has passed", () => {
    const ctx = ctxWith(validDoubleBottomWindow(), { lastSignalAt: new Date(NOW.getTime() - 90 * 60_000) }); // 90min > 60min cooldown
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });
});

describe("DoubleBottomRetestStrategy — window clamping (45-150 min)", () => {
  const strategy = new DoubleBottomRetestStrategy();

  it("clamps lookbackMinutes above 150 down to 150", () => {
    // Oldest tick 7500s (125min) old: 83% coverage of a clamped 150min (9000s) window
    // (passes), but only 62.5% of an (unclamped) 200min (12000s) window (would fail) -> a
    // buy here proves the clamp to 150 was actually applied.
    const history = [
      tick(100, 7500), tick(100, 6800), tick(98, 6000), tick(100, 5000), tick(99.5, 1800),
      tick(98.7, 1200), tick(98.2, 600), tick(97.9, 60), tick(98.0, 0),
    ];
    const ctx = ctxWith(history, { config: configWith({ ...defaultParams, lookbackMinutes: 200 }) });
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });

  it("clamps lookbackMinutes below 45 up to 45", () => {
    // Full pattern spread across 2400s (88.9% coverage of a clamped 45min/2700s window,
    // passes). An unclamped 10min (600s) window would only see the last 4 ticks (600s and
    // newer), whose own first-low search finds a shallower low at $98.2 with a bounce of
    // only 0% (the low IS the bounce high in that truncated slice) -> fails the bounce
    // threshold under the unclamped window, so a buy here proves clamping happened.
    const history = [
      tick(100, 2400), tick(100, 2100), tick(98, 1800), tick(100, 1500), tick(99.5, 900),
      tick(98.7, 600), tick(98.2, 300), tick(97.9, 60), tick(98.0, 0),
    ];
    const ctx = ctxWith(history, { config: configWith({ ...defaultParams, lookbackMinutes: 10 }) });
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });
});

describe("DoubleBottomRetestStrategy — exit management", () => {
  const strategy = new DoubleBottomRetestStrategy();
  const sixtyMinAgo = new Date(NOW.getTime() - 60 * 60_000);

  it("fires the hard stop immediately, even with only one tick of data (data-gap resilience)", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 100 };
    const history = [tick(97.4, 0)]; // hard stop = 100 * (1 - 2.5%) = 97.5
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: sixtyMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Hard stop");
    expect(signal?.sizeUsd).toBeCloseTo(194.8, 5);
  });

  it("sells once price hits the bounce target", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 98 };
    const history = [tick(99, 0)]; // target = 98 * 1.01 = 98.98
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: sixtyMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Bounce target");
    expect(signal?.sizeUsd).toBeCloseTo(198, 5);
  });

  it("holds a healthy position that hasn't hit the stop, the target, or the time limit", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 98 };
    const history = [tick(98.3, 0)]; // above stop, below target
    const thirtyMinAgo = new Date(NOW.getTime() - 30 * 60_000);
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: thirtyMinAgo }))).toBeNull();
  });

  it("exits via the time stop once the retest thesis has expired", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 98 };
    const history = [tick(98.3, 0)];
    const seventyMinAgo = new Date(NOW.getTime() - 70 * 60_000); // > 60min holdMinutes
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: seventyMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Time stop");
  });

  it("skips the time stop gracefully when the entry time is unknown (position opened by another strategy)", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 98 };
    const history = [tick(98.3, 0)];
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: null }))).toBeNull();
  });
});
