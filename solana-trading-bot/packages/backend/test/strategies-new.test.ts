import { describe, expect, it, vi, beforeEach } from "vitest";
import type { PriceTick, StrategyConfig } from "@trading-bot/shared";
import { GridStrategy } from "../src/strategy-engine/strategies/grid.js";
import { VolatilityBreakoutStrategy } from "../src/strategy-engine/strategies/volatilityBreakout.js";
import { RsiMacdStrategy } from "../src/strategy-engine/strategies/rsiMacd.js";
import type { StrategyContext } from "../src/strategy-engine/StrategyBase.js";

const TOKEN_MINT = "So11111111111111111111111111111111111111";
const TOKEN_SYMBOL = "SOL";

function tick(priceUsd: number, offsetMs = 0): PriceTick {
  return { tokenMint: TOKEN_MINT, tokenSymbol: TOKEN_SYMBOL, priceUsd, timestamp: new Date(offsetMs).toISOString() };
}

function configFor(strategyId: StrategyConfig["strategyId"], params: Record<string, number>): StrategyConfig {
  return {
    id: "cfg-test",
    strategyId,
    tokenMint: TOKEN_MINT,
    tokenSymbol: TOKEN_SYMBOL,
    params,
    active: true,
    createdAt: new Date(0).toISOString(),
  };
}

// MeanReversionStrategy moved to its own dedicated test/meanReversion.test.ts, matching
// the convention for other time-windowed strategies (dip-reversion, range-scalper) —
// its rebuilt version (real-time window + std-dev threshold, not tick-count) needs the
// same tick()-with-real-timestamps helper those files use, not this file's offsetMs-only
// tick() helper.

describe("GridStrategy", () => {
  const strategy = new GridStrategy();
  const gridParams = { lowerPrice: 100, upperPrice: 200, gridLevels: 10, orderSizeUsd: 100 };

  it("buys when price crosses down through a grid line while flat", () => {
    const history = [tick(155), tick(149)]; // level 5 -> level 4
    const ctx: StrategyContext = {
      config: configFor("grid", gridParams),
      priceHistory: history,
      latestPrice: history[1]!,
      now: new Date(),
      currentPosition: null,
      lastSignalAt: null,
    };
    const signal = strategy.onInterval(ctx);
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(100);
  });

  it("holds when price is outside the grid range", () => {
    const history = [tick(105), tick(50)]; // below lowerPrice
    const ctx: StrategyContext = {
      config: configFor("grid", gridParams),
      priceHistory: history,
      latestPrice: history[1]!,
      now: new Date(),
      currentPosition: null,
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("sells when price crosses up through a grid line above the entry", () => {
    const history = [tick(155), tick(161)]; // level 5 -> level 6
    const ctx: StrategyContext = {
      config: configFor("grid", gridParams),
      priceHistory: history,
      latestPrice: history[1]!,
      now: new Date(),
      currentPosition: { quantity: 1, avgEntryPriceUsd: 149 },
      lastSignalAt: null,
    };
    const signal = strategy.onInterval(ctx);
    expect(signal?.action).toBe("sell");
    expect(signal?.sizeUsd).toBeCloseTo(161, 5);
  });

  it("holds when holding a position and price hasn't crossed a new grid line", () => {
    const history = [tick(151), tick(152)]; // both level 5
    const ctx: StrategyContext = {
      config: configFor("grid", gridParams),
      priceHistory: history,
      latestPrice: history[1]!,
      now: new Date(),
      currentPosition: { quantity: 1, avgEntryPriceUsd: 149 },
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("sits out a range too thin to be worth trading fees, even with a clean level crossing", () => {
    // range 100 -> 100.2 is 0.2% of price, under the 0.3% default minRangePct;
    // gridStep 0.02 means 100.15 -> 100.13 is still a clean level crossing that would
    // otherwise trigger a buy.
    const thinParams = { lowerPrice: 100, upperPrice: 100.2, gridLevels: 10, orderSizeUsd: 100 };
    const history = [tick(100.15), tick(100.13)];
    const ctx: StrategyContext = {
      config: configFor("grid", thinParams),
      priceHistory: history,
      latestPrice: history[1]!,
      now: new Date(),
      currentPosition: null,
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("trades a thin range once minRangePct is configured low enough to allow it", () => {
    const thinParams = { lowerPrice: 100, upperPrice: 100.2, gridLevels: 10, orderSizeUsd: 100, minRangePct: 0.1 };
    const history = [tick(100.15), tick(100.13)];
    const ctx: StrategyContext = {
      config: configFor("grid", thinParams),
      priceHistory: history,
      latestPrice: history[1]!,
      now: new Date(),
      currentPosition: null,
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)?.action).toBe("buy");
  });
});

describe("VolatilityBreakoutStrategy", () => {
  const strategy = new VolatilityBreakoutStrategy();

  it("buys when a move exceeds atrMultiplier times recent average volatility", () => {
    // 14 oscillating ticks (avg abs diff 1) then a jump of 9 -> volatility ~1.571, threshold ~3.14
    const oscillation = [100, 101, 100, 101, 100, 101, 100, 101, 100, 101, 100, 101, 100, 101].map((p) => tick(p));
    const history = [tick(100), ...oscillation, tick(110)];
    const ctx: StrategyContext = {
      config: configFor("volatility-breakout", { atrPeriod: 14, atrMultiplier: 2, positionSizeUsd: 200 }),
      priceHistory: history,
      latestPrice: history[history.length - 1]!,
      now: new Date(),
      currentPosition: null,
      lastSignalAt: null,
    };
    const signal = strategy.onInterval(ctx);
    expect(signal?.action).toBe("buy");
    expect(signal?.sizeUsd).toBe(200);
  });

  it("holds when the move is within normal volatility", () => {
    const oscillation = [100, 101, 100, 101, 100, 101, 100, 101, 100, 101, 100, 101, 100, 101].map((p) => tick(p));
    const history = [tick(100), ...oscillation]; // last move is a normal +/-1 oscillation tick
    const ctx: StrategyContext = {
      config: configFor("volatility-breakout", { atrPeriod: 14, atrMultiplier: 2, positionSizeUsd: 200 }),
      priceHistory: history,
      latestPrice: history[history.length - 1]!,
      now: new Date(),
      currentPosition: null,
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("takes profit when the held position hits the take-profit target", () => {
    const ctx: StrategyContext = {
      config: configFor("volatility-breakout", { atrPeriod: 14, atrMultiplier: 2, positionSizeUsd: 200, takeProfitPct: 10 }),
      priceHistory: [tick(111)],
      latestPrice: tick(111), // +11% above entry of 100
      now: new Date(),
      currentPosition: { quantity: 5, avgEntryPriceUsd: 100 },
      lastSignalAt: null,
    };
    const signal = strategy.onInterval(ctx);
    expect(signal?.action).toBe("sell");
    expect(signal?.sizeUsd).toBeCloseTo(555, 5);
  });
});

describe("RsiMacdStrategy", () => {
  const strategy = new RsiMacdStrategy();
  const params = { rsiPeriod: 14, macdFast: 12, macdSlow: 26, macdSignal: 9, overboughtRsi: 70, positionSizeUsd: 200 };

  it("holds when there isn't enough history for MACD/RSI yet", () => {
    const history = Array.from({ length: 10 }, (_, i) => tick(100 + i));
    const ctx: StrategyContext = {
      config: configFor("rsi-macd", params),
      priceHistory: history,
      latestPrice: history[history.length - 1]!,
      now: new Date(),
      currentPosition: null,
      lastSignalAt: null,
    };
    expect(strategy.onInterval(ctx)).toBeNull();
  });

  it("eventually buys on a bullish MACD crossover during a recovery from a decline", () => {
    // Decline for 40 ticks, then a NOISY recovery (net upward drift with regular small
    // pullbacks) -> MACD's slow-lagging histogram eventually flips positive, while the
    // pullbacks keep RSI's avgLoss from decaying to ~0, so RSI doesn't saturate at 100
    // before that happens (a pure monotonic rise would peg RSI long before MACD reacts).
    const decline = Array.from({ length: 40 }, (_, i) => 100 - i * 0.5);
    const recovery: number[] = [];
    let price = decline[decline.length - 1]!;
    for (let i = 0; i < 40; i++) {
      price += 3;
      recovery.push(price);
      price += 3;
      recovery.push(price);
      price -= 2;
      recovery.push(price);
    }
    const prices = [...decline, ...recovery];

    let sawBuy = false;
    for (let i = 36; i <= prices.length; i++) {
      const history = prices.slice(0, i).map((p, idx) => tick(p, idx * 1000));
      const ctx: StrategyContext = {
        config: configFor("rsi-macd", params),
        priceHistory: history,
        latestPrice: history[history.length - 1]!,
        now: new Date(),
        currentPosition: null,
        lastSignalAt: null,
      };
      const signal = strategy.onInterval(ctx);
      if (signal?.action === "buy") {
        sawBuy = true;
        expect(signal.reason).toContain("Bullish MACD crossover");
        break;
      }
    }
    expect(sawBuy).toBe(true);
  });

  it("eventually sells an existing position once RSI reaches overbought during a sustained rise", () => {
    const rise = Array.from({ length: 60 }, (_, i) => 50 + i * 2); // strong steady uptrend
    let sawSell = false;
    for (let i = 36; i <= rise.length; i++) {
      const history = rise.slice(0, i).map((p, idx) => tick(p, idx * 1000));
      const ctx: StrategyContext = {
        config: configFor("rsi-macd", params),
        priceHistory: history,
        latestPrice: history[history.length - 1]!,
        now: new Date(),
        currentPosition: { quantity: 1, avgEntryPriceUsd: 50 },
        lastSignalAt: null,
      };
      const signal = strategy.onInterval(ctx);
      if (signal?.action === "sell") {
        sawSell = true;
        break;
      }
    }
    expect(sawSell).toBe(true);
  });
});
