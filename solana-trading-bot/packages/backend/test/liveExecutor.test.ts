import { describe, expect, it, beforeEach, vi } from "vitest";
import Database from "better-sqlite3";
import { drizzle } from "drizzle-orm/better-sqlite3";
import type { RiskLimits, Signal } from "@trading-bot/shared";
import * as schema from "../src/db/schema.js";
import type { SwapExecutor, SwapFillResult } from "../src/execution/swapExecutor.js";

// liveExecutor.ts imports the real `db` singleton (opens the actual dev SQLite file as an
// import-time side effect, same issue documented in hotWallet.test.ts) — mocked here with
// an isolated :memory: instance so these tests can never reach the real dev DB. No real RPC
// connection is exercised either: every test injects a FakeSwapExecutor/FakeBalanceProvider
// instead of the real Jupiter-adjacent devnet signing path, which is exercised for real
// during the Stage 3 devnet verification (self-ping on devnet), not in CI.

const sqlite = new Database(":memory:");
sqlite.exec(`
  CREATE TABLE positions (
    strategy_config_id TEXT NOT NULL, simulated INTEGER NOT NULL DEFAULT 1,
    token_mint TEXT NOT NULL, token_symbol TEXT NOT NULL,
    quantity REAL NOT NULL DEFAULT 0, avg_entry_price_usd REAL NOT NULL DEFAULT 0,
    stop_loss_price_usd REAL, PRIMARY KEY (strategy_config_id, simulated)
  );
  CREATE TABLE trades (
    id TEXT PRIMARY KEY, strategy_config_id TEXT NOT NULL, strategy_id TEXT NOT NULL,
    action TEXT NOT NULL, token_mint TEXT NOT NULL, token_symbol TEXT NOT NULL,
    price_usd REAL NOT NULL, size_usd REAL NOT NULL, size_token REAL NOT NULL,
    fee_usd REAL NOT NULL DEFAULT 0, reason TEXT NOT NULL, timestamp TEXT NOT NULL,
    simulated INTEGER NOT NULL DEFAULT 1, tx_hash TEXT, network TEXT, confirmation_slot INTEGER
  );
  CREATE TABLE portfolio_meta (
    id TEXT PRIMARY KEY, cash_usd REAL NOT NULL, realized_pnl_usd REAL NOT NULL DEFAULT 0,
    start_of_day_value_usd REAL NOT NULL, day_start_date TEXT NOT NULL, paused INTEGER NOT NULL DEFAULT 0
  );
  CREATE TABLE live_wallet_meta (
    id TEXT PRIMARY KEY, realized_pnl_usd REAL NOT NULL DEFAULT 0,
    start_of_day_value_usd REAL NOT NULL, day_start_date TEXT NOT NULL, updated_at TEXT NOT NULL
  );
  CREATE TABLE app_mode (
    id TEXT PRIMARY KEY, trading_mode TEXT NOT NULL DEFAULT 'paper', network TEXT NOT NULL DEFAULT 'devnet', updated_at TEXT NOT NULL
  );
  -- tradingMode/network here are just what getAppMode() reads for the Trade.network field
  -- (see applyFill) — the live+mainnet invariant itself is enforced by execution/appMode.ts,
  -- not exercised by these tests, so this row is only seeded to match that invariant.
  INSERT INTO app_mode (id, trading_mode, network, updated_at) VALUES ('singleton', 'live', 'mainnet', '2000-01-01T00:00:00.000Z');
`);
const testDb = drizzle(sqlite, { schema });

vi.mock("../src/db/client.js", () => ({ db: testDb }));

const { executeSignal, checkAndApplyStopLoss, getCurrentPosition, getTotalTokenExposureUsd, getPortfolioSnapshot, isLiveTradeInFlight } =
  await import("../src/execution/liveExecutor.js");
const { isPaused, setPaused } = await import("../src/execution/simulator.js");
const { priceCache } = await import("../src/market/priceCache.js");

const TOKEN_MINT = "So11111111111111111111111111111111111111";
const TOKEN_SYMBOL = "SOL";
const CONFIG_ID = "cfg-live-1";

const limits: RiskLimits = {
  maxPositionPct: 10,
  maxDailyLossPct: 5,
  perTokenExposurePct: 25,
  defaultStopLossPct: 8,
  maxSlippageBps: 100,
  maxPriceImpactPct: 3,
};

function pushTick(priceUsd: number): void {
  priceCache.push({ tokenMint: TOKEN_MINT, tokenSymbol: TOKEN_SYMBOL, priceUsd, timestamp: new Date().toISOString() });
}

function makeSignal(overrides: Partial<Signal> = {}): Signal {
  return {
    strategyConfigId: CONFIG_ID,
    strategyId: "momentum",
    action: "buy",
    tokenMint: TOKEN_MINT,
    tokenSymbol: TOKEN_SYMBOL,
    sizeUsd: 1000,
    reason: "test signal",
    timestamp: new Date().toISOString(),
    ...overrides,
  };
}

/** Deterministic fill: fee is fixed (not the real config.ts formula) so expected numbers in
 * assertions are simple to hand-compute. */
class FakeSwapExecutor implements SwapExecutor {
  calls: { action: "buy" | "sell"; sizeUsd: number; priceUsd: number }[] = [];
  private counter = 0;

  async swap(params: { action: "buy" | "sell"; sizeUsd: number; priceUsd: number }): Promise<SwapFillResult> {
    this.calls.push(params);
    this.counter += 1;
    return { sizeToken: params.sizeUsd / params.priceUsd, feeUsd: 0.5, txHash: `fake-tx-${this.counter}`, confirmationSlot: 1000 + this.counter };
  }
}

/** Never resolves until `resolveNext()` is called — used to prove the in-flight guard
 * actually blocks a second call while the real signing round trip is still pending.
 * `callStarted` resolves the instant `swap()` is invoked (and its resolver is queued
 * synchronously before that), so a test can `await callStarted` instead of guessing how
 * many microtask ticks executeSignal needs to reach the swap call. */
class SlowSwapExecutor implements SwapExecutor {
  private pending: (() => void)[] = [];
  private notifyCalled: (() => void) | null = null;
  callStarted = new Promise<void>((resolve) => {
    this.notifyCalled = resolve;
  });

  swap(params: { action: "buy" | "sell"; sizeUsd: number; priceUsd: number }): Promise<SwapFillResult> {
    this.notifyCalled?.();
    return new Promise((resolve) => {
      this.pending.push(() => resolve({ sizeToken: params.sizeUsd / params.priceUsd, feeUsd: 0.5, txHash: "slow-tx", confirmationSlot: null }));
    });
  }

  resolveNext(): void {
    this.pending.shift()?.();
  }
}

class FakeBalanceProvider {
  constructor(private usdcBalance: number) {}
  async getBalances() {
    return { solBalance: 0, usdcBalance: this.usdcBalance };
  }
}

/** Offers real-market-data risk-mapping numbers, the way RealJupiterSwapExecutor does —
 * proves executeSignal actually asks for and uses them instead of the Phase-1 placeholder
 * constants when the executor can provide them. */
class FakeSwapExecutorWithPreview extends FakeSwapExecutor {
  previewCalls: { action: "buy" | "sell"; tokenMint: string; sizeUsd: number; priceUsd: number }[] = [];

  async previewRiskMapping(params: { action: "buy" | "sell"; tokenMint: string; sizeUsd: number; priceUsd: number }) {
    this.previewCalls.push(params);
    return { assumedLiquidityUsd: 1000, simulatedSlippageBps: 5 };
  }
}

class ThrowingPreviewExecutor extends FakeSwapExecutor {
  async previewRiskMapping(): Promise<never> {
    throw new Error("network down");
  }
}

beforeEach(() => {
  sqlite.exec("DELETE FROM positions; DELETE FROM trades; DELETE FROM portfolio_meta; DELETE FROM live_wallet_meta;");
  sqlite.exec(
    "INSERT INTO portfolio_meta (id, cash_usd, realized_pnl_usd, start_of_day_value_usd, day_start_date, paused) VALUES ('singleton', 10000, 0, 10000, '2000-01-01', 0)",
  );
});

describe("liveExecutor.executeSignal — kill switch", () => {
  it("refuses to trade while paused, and resumes once unpaused", async () => {
    setPaused(true);
    pushTick(10);
    const result = await executeSignal(makeSignal(), limits, new FakeSwapExecutor(), new FakeBalanceProvider(10_000));
    expect(result).toEqual({ executed: false, reason: "Trading is paused (kill switch active)" });

    setPaused(false);
    expect(isPaused()).toBe(false);
  });
});

describe("liveExecutor.executeSignal — real risk-mapping preview", () => {
  it("uses the executor's previewRiskMapping numbers instead of the Phase-1 placeholders when available", async () => {
    pushTick(10);
    const executor = new FakeSwapExecutorWithPreview();
    const result = await executeSignal(makeSignal({ sizeUsd: 1000 }), limits, executor, new FakeBalanceProvider(10_000));

    expect(result.executed).toBe(true);
    if (!result.executed) throw new Error("expected executed");
    // maxPositionPct (10% of $10,000 = $1,000) would normally bind (see the plain
    // FakeSwapExecutor test below), but the preview's assumedLiquidityUsd=1,000 makes
    // maxPriceImpactPct (3% of 1,000 = $30) the tighter cap — proof the real numbers,
    // not the 500,000 placeholder, actually drove this decision.
    expect(result.trade.sizeUsd).toBe(30);
    expect(executor.previewCalls).toEqual([{ action: "buy", tokenMint: TOKEN_MINT, sizeUsd: 1000, priceUsd: 10 }]);
  });

  it("falls back to the Phase-1 placeholders when previewRiskMapping fails, rather than aborting the signal", async () => {
    pushTick(10);
    const executor = new ThrowingPreviewExecutor();
    const result = await executeSignal(makeSignal({ sizeUsd: 1000 }), limits, executor, new FakeBalanceProvider(10_000));

    expect(result.executed).toBe(true);
    if (!result.executed) throw new Error("expected executed");
    expect(result.trade.sizeUsd).toBe(1000); // placeholder liquidity is huge, so maxPositionPct binds as usual
  });

  it("never calls previewRiskMapping when the executor doesn't implement it (plain FakeSwapExecutor)", async () => {
    pushTick(10);
    const executor = new FakeSwapExecutor();
    const result = await executeSignal(makeSignal({ sizeUsd: 1000 }), limits, executor, new FakeBalanceProvider(10_000));

    expect(result.executed).toBe(true);
    if (!result.executed) throw new Error("expected executed");
    expect(result.trade.sizeUsd).toBe(1000);
  });
});

describe("liveExecutor.executeSignal — risk-manager gating", () => {
  it("executes a buy within limits, signs a real fill, and persists the trade/position", async () => {
    pushTick(10);
    const executor = new FakeSwapExecutor();
    const result = await executeSignal(makeSignal({ sizeUsd: 1000 }), limits, executor, new FakeBalanceProvider(10_000));

    expect(result.executed).toBe(true);
    if (!result.executed) throw new Error("expected executed");
    expect(result.trade.sizeUsd).toBe(1000); // exactly at the 10% cap, no shrink
    expect(result.trade.simulated).toBe(false);
    expect(result.trade.txHash).toBe("fake-tx-1");
    expect(result.trade.network).toBe("mainnet"); // reflects the real current network, not a stale constant
    expect(executor.calls).toHaveLength(1);

    const pos = getCurrentPosition(CONFIG_ID);
    expect(pos).not.toBeNull();
    expect(pos!.quantity).toBeCloseTo(100, 6); // 1000 / 10
    expect(pos!.avgEntryPriceUsd).toBeCloseTo(10.005, 6); // (1000 + 0.5 fee) / 100
  });

  it("shrinks an oversized buy to the maxPositionPct cap before signing", async () => {
    pushTick(10);
    const executor = new FakeSwapExecutor();
    const result = await executeSignal(makeSignal({ sizeUsd: 5000 }), limits, executor, new FakeBalanceProvider(10_000));

    expect(result.executed).toBe(true);
    if (!result.executed) throw new Error("expected executed");
    expect(result.trade.sizeUsd).toBe(1000); // 10% of 10,000 total portfolio value
    expect(executor.calls[0]?.sizeUsd).toBe(1000); // the shrunk size, not the requested one
  });

  it("rejects outright when the per-token exposure cap is already full, without ever signing", async () => {
    pushTick(10);
    sqlite.exec(
      `INSERT INTO positions (strategy_config_id, simulated, token_mint, token_symbol, quantity, avg_entry_price_usd, stop_loss_price_usd)
       VALUES ('other-cfg', 0, '${TOKEN_MINT}', '${TOKEN_SYMBOL}', 400, 10, 9.2)`,
      // 400 * 10 = 4,000 exposure; total portfolio value = 10,000 cash + 4,000 position = 14,000;
      // 25% cap = 3,500 < 4,000 already-held exposure, so no room remains.
    );
    const executor = new FakeSwapExecutor();
    const result = await executeSignal(makeSignal(), limits, executor, new FakeBalanceProvider(10_000));

    expect(result.executed).toBe(false);
    expect(executor.calls).toHaveLength(0);
  });

  it("sets the stop-loss price once on a fresh position and holds it fixed across a second buy", async () => {
    pushTick(10);
    await executeSignal(makeSignal({ sizeUsd: 500 }), limits, new FakeSwapExecutor(), new FakeBalanceProvider(10_000));
    const afterFirst = getCurrentPosition(CONFIG_ID)!;

    pushTick(11);
    await executeSignal(makeSignal({ sizeUsd: 500 }), limits, new FakeSwapExecutor(), new FakeBalanceProvider(10_000));

    const stopLossRow = sqlite.prepare("SELECT stop_loss_price_usd FROM positions WHERE strategy_config_id = ? AND simulated = 0").get(CONFIG_ID) as {
      stop_loss_price_usd: number;
    };
    expect(stopLossRow.stop_loss_price_usd).toBeCloseTo(9.2, 5); // 10 * (1 - 8%), unchanged by the price-11 second buy
    expect(afterFirst.avgEntryPriceUsd).toBeGreaterThan(0);
  });
});

describe("liveExecutor.executeSignal — sell and realized P&L", () => {
  it("folds the fee into cost basis on buy, then books realized P&L (net of fee) on a full-flatten sell", async () => {
    pushTick(10);
    await executeSignal(makeSignal({ action: "buy", sizeUsd: 1000 }), limits, new FakeSwapExecutor(), new FakeBalanceProvider(10_000));

    pushTick(12);
    const sellResult = await executeSignal(
      makeSignal({ action: "sell", sizeUsd: 1200 }),
      limits,
      new FakeSwapExecutor(),
      new FakeBalanceProvider(10_000),
    );

    expect(sellResult.executed).toBe(true);
    if (!sellResult.executed) throw new Error("expected executed");
    expect(sellResult.trade.action).toBe("sell");
    expect(getCurrentPosition(CONFIG_ID)).toBeNull(); // fully flattened

    const meta = sqlite.prepare("SELECT realized_pnl_usd FROM live_wallet_meta WHERE id = 'singleton'").get() as { realized_pnl_usd: number };
    // avgEntry = (1000 + 0.5 fee) / 100 = 10.005; realized = (12 - 10.005) * 100 - 0.5 fee = 199.0
    expect(meta.realized_pnl_usd).toBeCloseTo(199.0, 4);
  });
});

describe("liveExecutor.checkAndApplyStopLoss", () => {
  it("fires a mandatory sell when price drops to or below the stop-loss, attributed to risk-manager", async () => {
    pushTick(10);
    await executeSignal(makeSignal({ sizeUsd: 1000 }), limits, new FakeSwapExecutor(), new FakeBalanceProvider(10_000));

    pushTick(9); // stop is 9.2 (10 * (1 - 8%)); 9 <= 9.2 triggers
    const outcome = await checkAndApplyStopLoss(CONFIG_ID, TOKEN_MINT, limits, new FakeSwapExecutor(), new FakeBalanceProvider(10_000));

    expect(outcome?.executed).toBe(true);
    if (!outcome?.executed) throw new Error("expected executed");
    expect(outcome.trade.strategyId).toBe("risk-manager");
    expect(outcome.trade.action).toBe("sell");
    expect(getCurrentPosition(CONFIG_ID)).toBeNull();
  });

  it("does nothing when price is above the stop-loss", async () => {
    pushTick(10);
    await executeSignal(makeSignal({ sizeUsd: 1000 }), limits, new FakeSwapExecutor(), new FakeBalanceProvider(10_000));

    pushTick(9.5); // above the 9.2 stop
    const outcome = await checkAndApplyStopLoss(CONFIG_ID, TOKEN_MINT, limits, new FakeSwapExecutor(), new FakeBalanceProvider(10_000));
    expect(outcome).toBeNull();
    expect(getCurrentPosition(CONFIG_ID)).not.toBeNull();
  });
});

describe("liveExecutor — in-flight guard", () => {
  it("skips a second signal for the same config while the first fill is still pending", async () => {
    pushTick(10);
    const slow = new SlowSwapExecutor();
    const firstPromise = executeSignal(makeSignal({ sizeUsd: 500 }), limits, slow, new FakeBalanceProvider(10_000));

    await slow.callStarted; // executeSignal has reached the swap call and is now genuinely pending
    expect(isLiveTradeInFlight(CONFIG_ID)).toBe(true);

    const second = await executeSignal(makeSignal({ sizeUsd: 500 }), limits, new FakeSwapExecutor(), new FakeBalanceProvider(10_000));
    expect(second).toEqual({ executed: false, reason: "A live trade for this strategy is already in flight" });

    slow.resolveNext();
    const first = await firstPromise;
    expect(first.executed).toBe(true);
    expect(isLiveTradeInFlight(CONFIG_ID)).toBe(false);
  });

  it("also skips the mandatory stop-loss check while a signal for the same config is in flight", async () => {
    pushTick(10);
    await executeSignal(makeSignal({ sizeUsd: 1000 }), limits, new FakeSwapExecutor(), new FakeBalanceProvider(10_000));

    pushTick(9); // would otherwise trigger the stop
    const slow = new SlowSwapExecutor();
    const pendingSell = executeSignal(makeSignal({ action: "sell", sizeUsd: 100 }), limits, slow, new FakeBalanceProvider(10_000));
    await slow.callStarted;

    const stopOutcome = await checkAndApplyStopLoss(CONFIG_ID, TOKEN_MINT, limits, new FakeSwapExecutor(), new FakeBalanceProvider(10_000));
    expect(stopOutcome).toBeNull();

    slow.resolveNext();
    await pendingSell;
  });
});

describe("liveExecutor.getTotalTokenExposureUsd / getPortfolioSnapshot", () => {
  it("aggregates live exposure across strategy configs and excludes paper positions", async () => {
    pushTick(10);
    sqlite.exec(
      `INSERT INTO positions (strategy_config_id, simulated, token_mint, token_symbol, quantity, avg_entry_price_usd, stop_loss_price_usd)
       VALUES ('cfg-a', 0, '${TOKEN_MINT}', '${TOKEN_SYMBOL}', 10, 10, 9.2),
              ('cfg-b', 0, '${TOKEN_MINT}', '${TOKEN_SYMBOL}', 5, 10, 9.2),
              ('cfg-paper', 1, '${TOKEN_MINT}', '${TOKEN_SYMBOL}', 1000, 10, 9.2)`,
    );
    expect(getTotalTokenExposureUsd(TOKEN_MINT)).toBeCloseTo(150, 5); // (10 + 5) * 10, paper row excluded

    const snapshot = await getPortfolioSnapshot(new FakeBalanceProvider(500));
    expect(snapshot.cashUsd).toBe(500);
    expect(snapshot.positions.every((p) => p.tokenMint === TOKEN_MINT)).toBe(true);
    expect(snapshot.totalValueUsd).toBeCloseTo(500 + 150, 5);
  });
});
