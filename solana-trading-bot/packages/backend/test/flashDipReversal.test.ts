import { describe, expect, it } from "vitest";
import type { PriceTick, StrategyConfig } from "@trading-bot/shared";
import { FlashDipReversalStrategy } from "../src/strategy-engine/strategies/flashDipReversal.js";
import type { StrategyContext } from "../src/strategy-engine/StrategyBase.js";

const TOKEN_MINT = "So11111111111111111111111111111111111111";
const TOKEN_SYMBOL = "SOL";
const NOW = new Date("2026-01-01T00:00:00.000Z");

function tick(priceUsd: number, secondsAgo: number): PriceTick {
  return { tokenMint: TOKEN_MINT, tokenSymbol: TOKEN_SYMBOL, priceUsd, timestamp: new Date(NOW.getTime() - secondsAgo * 1000).toISOString() };
}

const defaultParams = {
  lookbackMinutes: 20,
  dropThresholdPct: 1,
  concentrationFraction: 0.6,
  targetBouncePct: 1,
  holdMinutes: 45,
  hardStopPct: 2.5,
  reentryCooldownMinutes: 20,
  positionSizeUsd: 150,
};

function configWith(params: Record<string, number>): StrategyConfig {
  return {
    id: "cfg-flash-dip-reversal",
    strategyId: "flash-dip-reversal",
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

/** Spans the full 20min default window (oldest tick exactly 1200s ago, 100% coverage).
 * Flat at $100 for the first 13min20s, then a sharp crash to $95 concentrated entirely
 * within the final ~6-7min (the concentration window: 20/3 ~= 6.67min), then a confirming
 * uptick to $95.5. windowHigh=100 -> dipPct=(100-95.5)/100=4.5%. The short window (last
 * ~6.67min) only sees the crash+recovery, so its own high is also ~100 -> shortDipPct
 * ~= 4.5% too -> concentrationFraction ~= 1.0, comfortably clears the 0.6 threshold. */
function sharpFlashDipWindow(): PriceTick[] {
  return [
    tick(100, 1200), tick(100, 1000), tick(100, 800), tick(100, 600), tick(100, 420),
    tick(100, 400), tick(97, 300), tick(95, 120), tick(95, 60), tick(95.5, 0),
  ];
}

/** Same total window and same total dip magnitude (100 -> 95.5, 4.5%) as
 * sharpFlashDipWindow, but the decline is spread evenly across the whole 20min window
 * instead of concentrated at the end. The short (last ~6.67min) window only captures a
 * small slice of that even decline, so shortDipPct is much smaller than the full dipPct,
 * and concentrationFraction falls well under the 0.6 threshold. */
function gradualDeclineWindow(): PriceTick[] {
  return [
    tick(100, 1200), tick(99, 1000), tick(98.5, 800), tick(98, 600), tick(97.5, 420),
    tick(97, 400), tick(96.5, 300), tick(96, 120), tick(95.7, 60), tick(95.5, 0),
  ];
}

describe("FlashDipReversalStrategy — entry filters", () => {
  const strategy = new FlashDipReversalStrategy();

  it("buys a confirmed, concentrated flash dip with full window coverage", () => {
    const signal = strategy.onInterval(ctxWith(sharpFlashDipWindow()));
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(150);
    expect(signal?.reason).toContain("Flash-dip entry");
  });

  it("holds on a gradual decline of the same total magnitude (not concentrated enough)", () => {
    expect(strategy.onInterval(ctxWith(gradualDeclineWindow()))).toBeNull();
  });

  it("holds when the dip doesn't clear the magnitude threshold at all", () => {
    const history = [
      tick(100, 1200), tick(100, 1000), tick(100, 800), tick(100, 600), tick(100, 420),
      tick(100, 400), tick(99.7, 300), tick(99.5, 120), tick(99.4, 60), tick(99.5, 0),
    ];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when price is still falling (no confirmation tick)", () => {
    // Same shape as sharpFlashDipWindow but the final two ticks are swapped so the latest
    // tick ($95) is BELOW the previous one ($95.5) — still falling, not turning up.
    const history = [
      tick(100, 1200), tick(100, 1000), tick(100, 800), tick(100, 600), tick(100, 420),
      tick(100, 400), tick(97, 300), tick(95.5, 120), tick(95.5, 60), tick(95, 0),
    ];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when there aren't enough ticks yet, even with full time coverage", () => {
    const history = [tick(100, 1200), tick(95, 60), tick(95.5, 0)];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when the available history doesn't yet cover enough of the requested window", () => {
    // Oldest tick only 300s old vs a 1200s (20min) window -> 25% coverage, well under the
    // 80% floor, even though the raw dip in these ticks looks big enough and concentrated.
    const history = [
      tick(100, 300), tick(100, 250), tick(100, 200), tick(100, 150), tick(100, 120),
      tick(97, 90), tick(95, 60), tick(95, 30), tick(95, 20), tick(95.5, 0),
    ];
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds during the re-entry cooldown since the last trade", () => {
    const ctx = ctxWith(sharpFlashDipWindow(), { lastSignalAt: new Date(NOW.getTime() - 5 * 60_000) }); // 5min < 20min cooldown
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("buys again once the cooldown has passed", () => {
    const ctx = ctxWith(sharpFlashDipWindow(), { lastSignalAt: new Date(NOW.getTime() - 25 * 60_000) }); // 25min > 20min cooldown
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });
});

describe("FlashDipReversalStrategy — window clamping (10-45 min)", () => {
  const strategy = new FlashDipReversalStrategy();

  it("clamps lookbackMinutes above 45 down to 45", () => {
    // Oldest tick 2200s (36.7min) old: 81% coverage of a clamped 45min (2700s) window
    // (passes), but only 55% of an (unclamped) 66min (3960s) window (would fail) -> a buy
    // here proves the clamp to 45 was actually applied.
    const history = [
      tick(100, 2200), tick(100, 1800), tick(100, 1400), tick(100, 1000), tick(100, 420),
      tick(100, 400), tick(97, 300), tick(95, 120), tick(95, 60), tick(95.5, 0),
    ];
    const ctx = ctxWith(history, { config: configWith({ ...defaultParams, lookbackMinutes: 66 }) });
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });

  it("clamps lookbackMinutes below 10 up to 10", () => {
    // Oldest tick 500s old: inside a clamped 10min (600s) window (83% coverage, passes),
    // but a hypothetical unclamped 3min (180s) window wouldn't cover it at all -> a buy
    // here proves clamping happened.
    const history = [
      tick(100, 500), tick(100, 420), tick(100, 340), tick(100, 260), tick(100, 200),
      tick(100, 180), tick(97, 120), tick(95, 60), tick(95, 30), tick(95.5, 0),
    ];
    const ctx = ctxWith(history, { config: configWith({ ...defaultParams, lookbackMinutes: 3 }) });
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });
});

describe("FlashDipReversalStrategy — exit management", () => {
  const strategy = new FlashDipReversalStrategy();
  const flatAt100 = [tick(100, 900), tick(100, 800), tick(100, 700), tick(100, 600), tick(100, 500), tick(100, 400), tick(100, 300), tick(100, 200), tick(100, 100), tick(100, 0)];
  const twentyMinAgo = new Date(NOW.getTime() - 20 * 60_000);

  it("fires the hard stop immediately, even with only one tick of data (data-gap resilience)", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 100 };
    const history = [tick(97.4, 0)]; // hard stop = 100 * (1 - 2.5%) = 97.5
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: twentyMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Hard stop");
    expect(signal?.sizeUsd).toBeCloseTo(194.8, 5);
  });

  it("sells once price hits the bounce target", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 95 };
    const history = [tick(96, 0)]; // target = 95 * 1.01 = 95.95
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: twentyMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Bounce target");
    expect(signal?.sizeUsd).toBeCloseTo(192, 5);
  });

  it("holds a healthy position that hasn't hit the stop, the target, or the time limit", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 95 };
    const history = [tick(95.5, 0)]; // above stop, below target
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: twentyMinAgo }))).toBeNull();
  });

  it("exits via the time stop once the bounce thesis has expired", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 95 };
    const history = [tick(95.5, 0)];
    const fiftyMinAgo = new Date(NOW.getTime() - 50 * 60_000); // > 45min holdMinutes
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: fiftyMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Time stop");
  });

  it("skips the time stop gracefully when the entry time is unknown (position opened by another strategy)", () => {
    const holding = { quantity: 2, avgEntryPriceUsd: 95 };
    const history = [tick(95.5, 0)];
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: null }))).toBeNull();
  });

  it("ignores the flat window used only for exit tests as an entry signal (sanity check)", () => {
    expect(strategy.onInterval(ctxWith(flatAt100))).toBeNull();
  });
});
