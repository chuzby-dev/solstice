import { afterEach, describe, expect, it, vi } from "vitest";
import Database from "better-sqlite3";
import { drizzle } from "drizzle-orm/better-sqlite3";
import * as schema from "../src/db/schema.js";

// swapExecutor.ts's RealJupiterSwapExecutor now genuinely submits to mainnet, so — same
// reasoning as hotWallet.test.ts/liveExecutor.test.ts — it must never touch the real dev
// SQLite file (getHotWalletPublicKey() reads db/client.js's singleton as an import-time
// side effect). Mocked here with an isolated :memory: instance holding just the tables
// this module's dependency chain touches.
const sqlite = new Database(":memory:");
sqlite.exec(`
  CREATE TABLE wallet_meta (id TEXT PRIMARY KEY, pubkey TEXT NOT NULL, created_at TEXT NOT NULL);
  CREATE TABLE risk_settings (
    id TEXT PRIMARY KEY, max_position_pct REAL NOT NULL, max_daily_loss_pct REAL NOT NULL,
    per_token_exposure_pct REAL NOT NULL, default_stop_loss_pct REAL NOT NULL,
    max_slippage_bps REAL NOT NULL, max_price_impact_pct REAL NOT NULL
  );
  INSERT INTO risk_settings (id, max_position_pct, max_daily_loss_pct, per_token_exposure_pct, default_stop_loss_pct, max_slippage_bps, max_price_impact_pct)
  VALUES ('singleton', 10, 5, 25, 8, 100, 3);
`);
const testDb = drizzle(sqlite, { schema });
vi.mock("../src/db/client.js", () => ({ db: testDb }));

const { assumedLiquidityFromQuote, MockSwapExecutor, RealJupiterSwapExecutor, simulatedSlippageBpsFromQuote } = await import(
  "../src/execution/swapExecutor.js"
);

// Real network calls (fetchJupiterQuote, buildAndSignJupiterSwap, and a full
// RealJupiterSwapExecutor.swap() with a wallet present) are exercised for real against
// live mainnet data during manual verification (see scripts/verify-jupiter-quote.ts, run
// once and discarded), not in CI — same reasoning as txBuilder.test.ts for anything
// needing a live RPC/API. What's tested here instead: the pure risk-mapping formulas, and
// that the real executor fails closed with no network call at all when there's nothing to
// sign with.

describe("RealJupiterSwapExecutor — fails closed with no hot wallet", () => {
  it("throws before making any network call when no hot wallet has been created", async () => {
    sqlite.exec("DELETE FROM wallet_meta");
    await expect(new RealJupiterSwapExecutor().swap({ action: "buy", tokenMint: "So11111111111111111111111111111111111111", sizeUsd: 100, priceUsd: 100 })).rejects.toThrow(
      /No hot wallet/,
    );
  });
});

describe("RealJupiterSwapExecutor.previewRiskMapping", () => {
  const originalFetch = global.fetch;
  const SOL_MINT = "So11111111111111111111111111111111111111";
  const USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

  afterEach(() => {
    global.fetch = originalFetch;
  });

  it("derives real risk-mapping numbers from a mocked quote, and requests USDC->SOL for a buy", async () => {
    let requestedUrl = "";
    global.fetch = vi.fn(async (url: string | URL) => {
      requestedUrl = url.toString();
      return {
        ok: true,
        json: async () => ({
          inputMint: USDC_MINT,
          outputMint: SOL_MINT,
          inAmount: "100000000", // 100 USDC (6 decimals)
          outAmount: "1000000000", // 1 SOL (9 decimals)
          otherAmountThreshold: "990000000", // 1% worse than quoted -> 100 bps
          priceImpactPct: "0.005", // 0.5%
          slippageBps: 100,
        }),
      } as Response;
    }) as unknown as typeof fetch;

    const preview = await new RealJupiterSwapExecutor().previewRiskMapping({ action: "buy", tokenMint: SOL_MINT, sizeUsd: 100, priceUsd: 100 });

    expect(preview.assumedLiquidityUsd).toBeCloseTo(100 / (0.5 / 100), 5); // 20,000
    expect(preview.simulatedSlippageBps).toBe(100);
    expect(requestedUrl).toContain(`inputMint=${USDC_MINT}`);
    expect(requestedUrl).toContain(`outputMint=${SOL_MINT}`);
    expect(requestedUrl).toContain("amount=100000000"); // $100 in USDC's 6 decimals
  });

  it("requests SOL->USDC for a sell, sized from sizeUsd/priceUsd in SOL's 9 decimals", async () => {
    let requestedUrl = "";
    global.fetch = vi.fn(async (url: string | URL) => {
      requestedUrl = url.toString();
      return {
        ok: true,
        json: async () => ({
          inputMint: SOL_MINT,
          outputMint: USDC_MINT,
          inAmount: "1000000000",
          outAmount: "100000000",
          otherAmountThreshold: "100000000",
          priceImpactPct: "0",
          slippageBps: 100,
        }),
      } as Response;
    }) as unknown as typeof fetch;

    await new RealJupiterSwapExecutor().previewRiskMapping({ action: "sell", tokenMint: SOL_MINT, sizeUsd: 200, priceUsd: 100 });

    expect(requestedUrl).toContain(`inputMint=${SOL_MINT}`);
    expect(requestedUrl).toContain(`outputMint=${USDC_MINT}`);
    expect(requestedUrl).toContain("amount=2000000000"); // (200 / 100) SOL * 1e9
  });
});

describe("MockSwapExecutor", () => {
  it("is exported as a class distinct from RealJupiterSwapExecutor (devnet stand-in vs. real mainnet path)", () => {
    expect(MockSwapExecutor).not.toBe(RealJupiterSwapExecutor);
    expect(new MockSwapExecutor()).toBeInstanceOf(MockSwapExecutor);
  });
});

describe("assumedLiquidityFromQuote", () => {
  it("back-computes implied liquidity from requested size and price impact", () => {
    // $1,000 trade causing 0.5% impact implies ~$200,000 of liquidity at that depth.
    expect(assumedLiquidityFromQuote(1000, 0.5)).toBeCloseTo(200_000, 5);
  });

  it("returns +Infinity for zero/negative measured impact rather than dividing by zero", () => {
    expect(assumedLiquidityFromQuote(1000, 0)).toBe(Number.POSITIVE_INFINITY);
    expect(assumedLiquidityFromQuote(1000, -1)).toBe(Number.POSITIVE_INFINITY);
  });
});

describe("simulatedSlippageBpsFromQuote", () => {
  it("computes worst-case slippage in bps from outAmount vs. the guaranteed minimum", () => {
    // 1% shortfall between quoted out and the guaranteed minimum = 100 bps.
    expect(simulatedSlippageBpsFromQuote(1_000_000n, 990_000n)).toBe(100);
  });

  it("is zero when the guaranteed minimum equals the quoted amount", () => {
    expect(simulatedSlippageBpsFromQuote(1_000_000n, 1_000_000n)).toBe(0);
  });

  it("returns zero rather than dividing by zero when outAmount is zero", () => {
    expect(simulatedSlippageBpsFromQuote(0n, 0n)).toBe(0);
  });
});
