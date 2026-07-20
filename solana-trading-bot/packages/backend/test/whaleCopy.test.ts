import { describe, expect, it, vi, beforeEach } from "vitest";
import type { PriceTick, StrategyConfig } from "@trading-bot/shared";
import type { StrategyContext } from "../src/strategy-engine/StrategyBase.js";

vi.mock("../src/market/whaleWatcher.js", () => ({
  drainPending: vi.fn(),
}));

const { drainPending } = await import("../src/market/whaleWatcher.js");
const { WhaleCopyStrategy } = await import("../src/strategy-engine/strategies/whaleCopy.js");

const TOKEN_MINT = "So11111111111111111111111111111111111111";
const TOKEN_SYMBOL = "SOL";
const WATCHED_WALLET = "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin";

function tick(priceUsd: number): PriceTick {
  return { tokenMint: TOKEN_MINT, tokenSymbol: TOKEN_SYMBOL, priceUsd, timestamp: new Date().toISOString() };
}

function config(params: Record<string, number>): StrategyConfig {
  return {
    id: "cfg-whale",
    strategyId: "whale-copy",
    tokenMint: TOKEN_MINT,
    tokenSymbol: TOKEN_SYMBOL,
    params,
    watchedWalletAddress: WATCHED_WALLET,
    active: true,
    createdAt: new Date(0).toISOString(),
  };
}

function baseCtx(overrides: Partial<StrategyContext> = {}): StrategyContext {
  return {
    config: config({ lagSeconds: 30, copyRatioPct: 10, maxSizeUsd: 200 }),
    priceHistory: [tick(10)],
    latestPrice: tick(10),
    now: new Date(),
    currentPosition: null,
    lastSignalAt: null,
    ...overrides,
  };
}

describe("WhaleCopyStrategy", () => {
  const strategy = new WhaleCopyStrategy();

  beforeEach(() => {
    vi.mocked(drainPending).mockReset();
  });

  it("holds when there's nothing pending", () => {
    vi.mocked(drainPending).mockReturnValue([]);
    expect(strategy.onInterval(baseCtx())).toBeNull();
  });

  it("holds when the only pending transfer is still within the lag window", () => {
    vi.mocked(drainPending).mockReturnValue([
      { signature: "sig1", blockTime: new Date(Date.now() - 5_000), tokenMint: TOKEN_MINT, direction: "buy", tokenAmount: 100 },
    ]);
    expect(strategy.onInterval(baseCtx())).toBeNull();
  });

  it("mirrors a buy once it clears the lag window, scaled by copyRatioPct and capped at maxSizeUsd", () => {
    vi.mocked(drainPending).mockReturnValue([
      { signature: "sig1", blockTime: new Date(Date.now() - 60_000), tokenMint: TOKEN_MINT, direction: "buy", tokenAmount: 100 },
    ]);
    // wantSizeUsd = 100 tokens * $10 * 10% = $100, below the $200 cap
    const signal = strategy.onInterval(baseCtx());
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBeCloseTo(100, 5);
  });

  it("caps the mirrored size at maxSizeUsd for a large whale trade", () => {
    vi.mocked(drainPending).mockReturnValue([
      { signature: "sig1", blockTime: new Date(Date.now() - 60_000), tokenMint: TOKEN_MINT, direction: "buy", tokenAmount: 1000 },
    ]);
    // wantSizeUsd = 1000 * $10 * 10% = $1000, capped to $200
    const signal = strategy.onInterval(baseCtx());
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(200);
  });

  it("ignores a mirrored sell when there's no existing position to sell", () => {
    vi.mocked(drainPending).mockReturnValue([
      { signature: "sig1", blockTime: new Date(Date.now() - 60_000), tokenMint: TOKEN_MINT, direction: "sell", tokenAmount: 50 },
    ]);
    expect(strategy.onInterval(baseCtx({ currentPosition: null }))).toBeNull();
  });

  it("mirrors a sell capped to the size of the held position", () => {
    vi.mocked(drainPending).mockReturnValue([
      { signature: "sig1", blockTime: new Date(Date.now() - 60_000), tokenMint: TOKEN_MINT, direction: "sell", tokenAmount: 1000 },
    ]);
    // wantSizeUsd = 1000 * $10 * 10% = $1000, but held position is only 2 * $10 = $20
    const signal = strategy.onInterval(baseCtx({ currentPosition: { quantity: 2, avgEntryPriceUsd: 9 } }));
    expect(signal?.action).toBe("sell");
    expect(signal?.sizeUsd).toBeCloseTo(20, 5);
  });
});
