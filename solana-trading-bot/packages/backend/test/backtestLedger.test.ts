import { describe, expect, it } from "vitest";
import type { RiskLimits } from "@trading-bot/shared";
import { BacktestLedger } from "../src/backtest/ledger.js";

const limits: RiskLimits = {
  maxPositionPct: 10,
  maxDailyLossPct: 5,
  perTokenExposurePct: 25,
  defaultStopLossPct: 8,
  maxSlippageBps: 100,
  maxPriceImpactPct: 3,
};

const MINT = "So11111111111111111111111111111111111111";
// Matches config.ts's tradingCosts defaults (solanaTxFeeUsd 0.0005 + priorityFeeUsd 0.005,
// swapFeeBps 10 + slippageBufferBps 5) — see estimateTradeFeeUsd.
const fee = (sizeUsd: number) => 0.0055 + sizeUsd * 0.0015;

function makeLedger(startingCashUsd = 10_000, firstTimestamp = "2026-01-01T00:00:00.000Z"): BacktestLedger {
  return new BacktestLedger(startingCashUsd, MINT, "SOL", "cfg-1", "momentum", firstTimestamp);
}

describe("BacktestLedger.applyBuy", () => {
  it("deducts cash by size + fee and opens a position with a stop-loss", () => {
    const ledger = makeLedger();
    ledger.applyBuy(100, 1000, limits, "test buy", "2026-01-01T00:01:00.000Z");

    expect(ledger.cashUsd).toBeCloseTo(10_000 - 1000 - fee(1000), 6);
    expect(ledger.position.quantity).toBeCloseTo(10, 6);
    expect(ledger.position.stopLossPriceUsd).toBeCloseTo(100 * 0.92, 6);
    expect(ledger.currentPosition?.quantity).toBeCloseTo(10, 6);
  });

  it("folds the fee into cost basis so avgEntryPriceUsd exceeds the raw fill price", () => {
    const ledger = makeLedger();
    ledger.applyBuy(100, 1000, limits, "test buy", "2026-01-01T00:01:00.000Z");

    expect(ledger.position.avgEntryPriceUsd).toBeGreaterThan(100);
    expect(ledger.position.avgEntryPriceUsd).toBeCloseTo(100 + fee(1000) / 10, 6);
  });

  it("blends avg entry across two buys and keeps the original stop-loss", () => {
    const ledger = makeLedger();
    ledger.applyBuy(100, 1000, limits, "buy 1", "2026-01-01T00:01:00.000Z");
    const stopAfterFirst = ledger.position.stopLossPriceUsd;
    ledger.applyBuy(110, 1000, limits, "buy 2", "2026-01-01T00:02:00.000Z");

    expect(ledger.position.stopLossPriceUsd).toBe(stopAfterFirst);
    expect(ledger.position.quantity).toBeCloseTo(10 + 1000 / 110, 6);
  });
});

describe("BacktestLedger.applySell", () => {
  it("realizes P&L net of the sell fee and returns cash", () => {
    const ledger = makeLedger();
    ledger.applyBuy(100, 1000, limits, "buy", "2026-01-01T00:01:00.000Z");
    const cashAfterBuy = ledger.cashUsd;

    ledger.applySell(120, ledger.position.quantity * 120, "sell all", "2026-01-01T00:10:00.000Z");

    expect(ledger.position.quantity).toBe(0);
    expect(ledger.currentPosition).toBeNull();
    expect(ledger.realizedPnlUsd).toBeGreaterThan(0); // bought near $100+fee, sold at $120
    expect(ledger.cashUsd).toBeGreaterThan(cashAfterBuy);
  });

  it("records holdMinutes only on the fill that fully flattens the position", () => {
    const ledger = makeLedger();
    ledger.applyBuy(100, 1000, limits, "buy", "2026-01-01T00:00:00.000Z");
    ledger.applySell(110, ledger.position.quantity * 110, "sell all", "2026-01-01T00:05:00.000Z");

    const sellFill = ledger.fills.find((f) => f.trade.action === "sell")!;
    expect(sellFill.holdMinutes).toBeCloseTo(5, 6);
    expect(sellFill.realizedDeltaUsd).not.toBeNull();
  });
});

describe("BacktestLedger.checkStopLoss", () => {
  it("does nothing when flat", () => {
    const ledger = makeLedger();
    expect(ledger.checkStopLoss(50, "2026-01-01T00:00:00.000Z")).toBe(false);
  });

  it("does nothing while price stays above the stop", () => {
    const ledger = makeLedger();
    ledger.applyBuy(100, 1000, limits, "buy", "2026-01-01T00:00:00.000Z");
    expect(ledger.checkStopLoss(95, "2026-01-01T00:01:00.000Z")).toBe(false);
    expect(ledger.currentPosition).not.toBeNull();
  });

  it("force-sells the full position once price hits the stop, tagged risk-manager", () => {
    const ledger = makeLedger();
    ledger.applyBuy(100, 1000, limits, "buy", "2026-01-01T00:00:00.000Z");
    const stop = ledger.position.stopLossPriceUsd!;
    const fired = ledger.checkStopLoss(stop, "2026-01-01T00:01:00.000Z");

    expect(fired).toBe(true);
    expect(ledger.currentPosition).toBeNull();
    const lastFill = ledger.fills[ledger.fills.length - 1]!;
    expect(lastFill.trade.strategyId).toBe("risk-manager");
    expect(lastFill.trade.action).toBe("sell");
  });
});

describe("BacktestLedger day rollover", () => {
  it("resets the daily-loss baseline when the calendar day changes", () => {
    const ledger = makeLedger(10_000, "2026-01-01T00:00:00.000Z");
    ledger.applyBuy(100, 1000, limits, "buy", "2026-01-01T01:00:00.000Z");

    ledger.rolloverDayIfNeeded("2026-01-01T02:00:00.000Z", 80); // same day, no reset
    expect(ledger.dailyLossEvalInputs(80).dailyLossUsd).toBeGreaterThan(0);

    ledger.rolloverDayIfNeeded("2026-01-02T00:00:00.000Z", 80); // new day, baseline resets
    expect(ledger.dailyLossEvalInputs(80).dailyLossUsd).toBe(0);
  });
});
