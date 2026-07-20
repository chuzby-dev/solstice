import { describe, expect, it } from "vitest";
import type { PriceTick, StrategyConfig } from "@trading-bot/shared";
import { FeeAwareScalperStrategy } from "../src/strategy-engine/strategies/feeAwareScalper.js";
import type { StrategyContext } from "../src/strategy-engine/StrategyBase.js";

const TOKEN_MINT = "So11111111111111111111111111111111111111";
const TOKEN_SYMBOL = "SOL";
const NOW = new Date("2026-01-01T00:00:00.000Z");

function tick(priceUsd: number, index: number): PriceTick {
  return { tokenMint: TOKEN_MINT, tokenSymbol: TOKEN_SYMBOL, priceUsd, timestamp: new Date(NOW.getTime() - (1000 - index) * 1000).toISOString() };
}

const defaultParams = {
  positionSizeUsd: 20,
  smaPeriod: 12,
  dipPct: 0.08,
  minProfitMultiple: 1.5,
  stopLossMultiple: 1.0,
  maxHoldMinutes: 3,
};

// With defaults (config.ts fallback fees: 0.0005 tx + 0.005 priority + 15bps swap+slippage
// on $20): oneLegFee = 0.0055 + 20*0.0015 = 0.0355, roundTripFee = 0.071,
// requiredProfitPct = (0.071*1.5)/20*100 = 0.5325%, stopLossPct = 0.071/20*100 = 0.355%.
const REQUIRED_PROFIT_PCT = 0.5325;
const STOP_LOSS_PCT = 0.355;

function configWith(params: Record<string, number>): StrategyConfig {
  return {
    id: "cfg-fee-scalper",
    strategyId: "fee-aware-scalper",
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

/** 10 flat ticks at 100 + a dip to 99.00 + a confirmation uptick to 99.20.
 * SMA12 = (10*100 + 99.00 + 99.20)/12 = 99.85. dipThreshold = 99.85*0.9992 = 99.7701
 * (99.20 qualifies). requiredTargetPrice = 99.20*1.005325 = 99.7284; SMA (99.85)
 * clears it, so the setup passes the fee-derived reversion check. */
function validDipWindow(): PriceTick[] {
  const flat = Array.from({ length: 10 }, () => 100);
  return [...flat, 99.0, 99.2].map((p, i) => tick(p, i));
}

describe("FeeAwareScalperStrategy — entry", () => {
  const strategy = new FeeAwareScalperStrategy();

  it("holds when there isn't enough history for the SMA yet", () => {
    const history = Array.from({ length: 8 }, (_, i) => tick(100, i));
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("buys a confirmed dip whose realistic reversion clears the fee-derived profit target", () => {
    const signal = strategy.onInterval(ctxWith(validDipWindow()));
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(20);
    expect(signal?.reason).toContain("Fee-aware entry");
    expect(signal?.reason).toContain("round-trip cost");
  });

  it("holds when the dip doesn't even clear the cheap pre-filter threshold", () => {
    // SMA12 = (10*100 + 99.90 + 99.95)/12 = 99.9875; dipThreshold = 99.9875*0.9992 = 99.9075;
    // latest 99.95 > 99.9075 -> doesn't qualify as a dip at all.
    const flat = Array.from({ length: 10 }, () => 100);
    const history = [...flat, 99.9, 99.95].map((p, i) => tick(p, i));
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when the dip qualifies but the realistic reversion wouldn't clear round-trip costs", () => {
    // SMA12 = (10*100 + 99.30 + 99.40)/12 = 99.8917; dipThreshold = 99.8117 (99.40 qualifies);
    // requiredTargetPrice = 99.40*1.005325 = 99.9294 > SMA (99.8917) -> the "reasonable"
    // bounce back to the SMA wouldn't even cover costs with margin, so skip it.
    const flat = Array.from({ length: 10 }, () => 100);
    const history = [...flat, 99.3, 99.4].map((p, i) => tick(p, i));
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });

  it("holds when price is still falling (no confirmation tick)", () => {
    const flat = Array.from({ length: 10 }, () => 100);
    const history = [...flat, 99.2, 99.0].map((p, i) => tick(p, i)); // still dropping
    expect(strategy.onInterval(ctxWith(history))).toBeNull();
  });
});

describe("FeeAwareScalperStrategy — exit", () => {
  const strategy = new FeeAwareScalperStrategy();
  const oneMinAgo = new Date(NOW.getTime() - 60_000);

  it("takes profit once price clears the fee-derived target", () => {
    const history = [tick(100.6, 0)]; // target = 100 * 1.005325 = 100.5325
    const holding = { quantity: 0.2, avgEntryPriceUsd: 100 };
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: oneMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Take-profit");
    expect(signal?.reason).toContain(`${REQUIRED_PROFIT_PCT}`.slice(0, 4));
  });

  it("holds just below the fee-derived target", () => {
    const history = [tick(100.5, 0)]; // below 100.5325
    const holding = { quantity: 0.2, avgEntryPriceUsd: 100 };
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: oneMinAgo }))).toBeNull();
  });

  it("cuts the loss at the fee-derived stop", () => {
    const history = [tick(99.6, 0)]; // stop = 100 * (1 - 0.355%) = 99.645
    const holding = { quantity: 0.2, avgEntryPriceUsd: 100 };
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: oneMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Stop-loss");
    expect(signal?.reason).toContain(`${STOP_LOSS_PCT}`);
  });

  it("holds just above the fee-derived stop", () => {
    const history = [tick(99.7, 0)]; // above 99.645
    const holding = { quantity: 0.2, avgEntryPriceUsd: 100 };
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: oneMinAgo }))).toBeNull();
  });

  it("exits via the time stop once held too long, even mid-range", () => {
    const history = [tick(100.0, 0)]; // exactly at entry, neither target nor stop
    const holding = { quantity: 0.2, avgEntryPriceUsd: 100 };
    const fourMinAgo = new Date(NOW.getTime() - 4 * 60_000); // > 3min maxHoldMinutes
    const signal = strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: fourMinAgo }));
    expect(signal?.action).toBe("sell");
    expect(signal?.reason).toContain("Time stop");
  });

  it("skips the time stop gracefully when the entry time is unknown", () => {
    const history = [tick(100.0, 0)];
    const holding = { quantity: 0.2, avgEntryPriceUsd: 100 };
    expect(strategy.onInterval(ctxWith(history, { currentPosition: holding, lastSignalAt: null }))).toBeNull();
  });
});
