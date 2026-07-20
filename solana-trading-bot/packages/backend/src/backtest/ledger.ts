import { randomUUID } from "node:crypto";
import type { RiskLimits, Trade } from "@trading-bot/shared";
import { computeStopLossPrice } from "../execution/riskManager.js";
import { estimateTradeFeeUsd } from "../config.js";

const DUST_QUANTITY = 1e-9;

interface LedgerPosition {
  quantity: number;
  avgEntryPriceUsd: number;
  stopLossPriceUsd: number | null;
}

export interface LedgerFill {
  trade: Trade;
  /** Realized P&L of this fill, net of its own fee. Only set on sells (null on buys),
   * matching how execution/simulator.ts only ever books realizedPnlUsd on the sell leg. */
  realizedDeltaUsd: number | null;
  /** Minutes since the position was opened, set only on a sell that fully flattens it
   * (mirrors "one open position at a time" — every strategy in this codebase holds a
   * single sub-ledgered position per config, see db/schema.ts). */
  holdMinutes: number | null;
}

/**
 * In-memory portfolio ledger for one strategy config's backtest run. Deliberately NOT a
 * reuse of execution/simulator.ts — that module is hard-wired to the live SQLite `db` and
 * `priceCache` singletons (module-level imports, no injection seam), so reusing it as-is
 * would mean either mutating the live paper-trading ledger or spawning a subprocess per
 * backtest run. Instead this replicates simulator.ts's `applyFill()` /
 * `checkAndApplyStopLoss()` math exactly (see packages/backend/src/execution/simulator.ts
 * lines 155-254 and 321-346 as of this writing): same fee-folded-into-cost-basis on buy,
 * same stop-loss-price-set-once-while-flat, same realized-P&L-minus-fee on sell, same
 * daily-loss baseline rollover on calendar-day change. `riskManager.evaluateSignal()` and
 * `computeStopLossPrice()` themselves are pure and imported unchanged, not duplicated.
 */
export class BacktestLedger {
  cashUsd: number;
  realizedPnlUsd = 0;
  totalFeesUsd = 0;
  position: LedgerPosition = { quantity: 0, avgEntryPriceUsd: 0, stopLossPriceUsd: null };
  fills: LedgerFill[] = [];
  lastTradeAt: Date | null = null;

  private dayStartDate: string;
  private startOfDayValueUsd: number;
  private positionOpenedAt: string | null = null;

  constructor(
    startingCashUsd: number,
    private readonly tokenMint: string,
    private readonly tokenSymbol: string,
    private readonly strategyConfigId: string,
    private readonly strategyId: Trade["strategyId"],
    firstTimestamp: string,
  ) {
    this.cashUsd = startingCashUsd;
    this.dayStartDate = firstTimestamp.slice(0, 10);
    this.startOfDayValueUsd = startingCashUsd;
  }

  get currentPosition(): { quantity: number; avgEntryPriceUsd: number } | null {
    return this.position.quantity > DUST_QUANTITY
      ? { quantity: this.position.quantity, avgEntryPriceUsd: this.position.avgEntryPriceUsd }
      : null;
  }

  get trades(): Trade[] {
    return this.fills.map((f) => f.trade);
  }

  portfolioValueUsd(priceUsd: number): number {
    return this.cashUsd + this.position.quantity * priceUsd;
  }

  /** Mirrors execution/simulator.ts's rolloverDayIfNeeded — rolls the daily-loss baseline
   * forward when the calendar day (UTC date of the tick) changes, so maxDailyLossPct
   * compares against the right baseline across a multi-day backtest instead of the
   * baseline silently going stale. Called before every trade decision, same as live. */
  rolloverDayIfNeeded(timestamp: string, priceUsd: number): void {
    const day = timestamp.slice(0, 10);
    if (day === this.dayStartDate) return;
    this.dayStartDate = day;
    this.startOfDayValueUsd = this.portfolioValueUsd(priceUsd);
  }

  /** Matches executeSignal's `startOfDayValueUsd: snapshot.totalValueUsd + snapshot.dailyLossUsd`
   * exactly (see simulator.ts:286) rather than exposing the raw baseline directly. */
  dailyLossEvalInputs(priceUsd: number): { dailyLossUsd: number; startOfDayValueUsd: number } {
    const totalValueUsd = this.portfolioValueUsd(priceUsd);
    const dailyLossUsd = Math.max(0, this.startOfDayValueUsd - totalValueUsd);
    return { dailyLossUsd, startOfDayValueUsd: totalValueUsd + dailyLossUsd };
  }

  private recordFill(
    action: "buy" | "sell",
    priceUsd: number,
    sizeUsd: number,
    sizeToken: number,
    feeUsd: number,
    reason: string,
    timestamp: string,
    realizedDeltaUsd: number | null,
    holdMinutes: number | null,
    strategyIdOverride?: Trade["strategyId"],
  ): void {
    const trade: Trade = {
      id: randomUUID(),
      strategyConfigId: this.strategyConfigId,
      strategyId: strategyIdOverride ?? this.strategyId,
      action,
      tokenMint: this.tokenMint,
      tokenSymbol: this.tokenSymbol,
      priceUsd,
      sizeUsd,
      sizeToken,
      feeUsd,
      reason,
      simulated: true,
      txHash: null,
      network: null,
      confirmationSlot: null,
      timestamp,
    };
    this.fills.push({ trade, realizedDeltaUsd, holdMinutes });
    this.totalFeesUsd += feeUsd;
    this.lastTradeAt = new Date(timestamp);
  }

  applyBuy(priceUsd: number, sizeUsd: number, limits: RiskLimits, reason: string, timestamp: string): void {
    const sizeToken = sizeUsd / priceUsd;
    const feeUsd = estimateTradeFeeUsd(sizeUsd);
    const wasFlat = this.position.quantity <= DUST_QUANTITY;
    const newQuantity = this.position.quantity + sizeToken;
    const effectiveCostUsd = sizeUsd + feeUsd;
    const newAvgEntry = (this.position.quantity * this.position.avgEntryPriceUsd + effectiveCostUsd) / newQuantity;
    const stopLossPriceUsd = wasFlat ? computeStopLossPrice(priceUsd, limits) : this.position.stopLossPriceUsd;

    if (wasFlat) this.positionOpenedAt = timestamp;
    this.position = { quantity: newQuantity, avgEntryPriceUsd: newAvgEntry, stopLossPriceUsd };
    this.cashUsd -= sizeUsd + feeUsd;
    this.recordFill("buy", priceUsd, sizeUsd, sizeToken, feeUsd, reason, timestamp, null, null);
  }

  applySell(priceUsd: number, sizeUsd: number, reason: string, timestamp: string, strategyIdOverride?: Trade["strategyId"]): void {
    const sizeToken = sizeUsd / priceUsd;
    const feeUsd = estimateTradeFeeUsd(sizeUsd);
    const sellQuantity = Math.min(sizeToken, this.position.quantity);
    const realizedDelta = (priceUsd - this.position.avgEntryPriceUsd) * sellQuantity - feeUsd;
    const remainingQuantity = this.position.quantity - sellQuantity;
    const isFlat = remainingQuantity <= DUST_QUANTITY;

    let holdMinutes: number | null = null;
    if (isFlat && this.positionOpenedAt) {
      holdMinutes = (new Date(timestamp).getTime() - new Date(this.positionOpenedAt).getTime()) / 60_000;
      this.positionOpenedAt = null;
    }

    this.position = isFlat ? { quantity: 0, avgEntryPriceUsd: 0, stopLossPriceUsd: null } : { ...this.position, quantity: remainingQuantity };
    this.cashUsd += sizeUsd - feeUsd;
    this.realizedPnlUsd += realizedDelta;
    this.recordFill("sell", priceUsd, sizeUsd, sizeToken, feeUsd, reason, timestamp, realizedDelta, holdMinutes, strategyIdOverride);
  }

  /** Mirrors checkAndApplyStopLoss (simulator.ts:321-346): mandatory, independent of any
   * strategy signal, bypasses evaluateSignal entirely. Returns true if it fired. */
  checkStopLoss(priceUsd: number, timestamp: string): boolean {
    if (this.position.quantity <= DUST_QUANTITY || this.position.stopLossPriceUsd === null) return false;
    if (priceUsd > this.position.stopLossPriceUsd) return false;

    this.rolloverDayIfNeeded(timestamp, priceUsd);
    const sizeUsd = this.position.quantity * priceUsd;
    this.applySell(priceUsd, sizeUsd, `Mandatory stop-loss triggered: price ${priceUsd} <= stop ${this.position.stopLossPriceUsd.toFixed(4)}`, timestamp, "risk-manager");
    return true;
  }
}
