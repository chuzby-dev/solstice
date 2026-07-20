import { randomUUID } from "node:crypto";
import { and, eq } from "drizzle-orm";
import type { PortfolioSnapshot, Position, RiskLimits, Signal, Trade } from "@trading-bot/shared";
import { db } from "../db/client.js";
import { portfolioMeta, positions, trades } from "../db/schema.js";
import { priceCache } from "../market/priceCache.js";
import { computeStopLossPrice, evaluateSignal, type RiskEvaluationInput } from "./riskManager.js";
import { estimateTradeFeeUsd } from "../config.js";

// This module is the ENTIRE execution surface for Phase 1. It only ever mutates the
// local SQLite virtual ledger (cash/positions/trades tables). There is no wallet
// keypair anywhere in this file, no RPC write call, no `sendTransaction`. Trades here
// can never move real funds.
//
// Positions are sub-ledgered per STRATEGY CONFIG, not per token (see db/schema.ts):
// two active strategies both trading SOL hold fully independent positions and can't
// buy/sell out from under each other. Cash remains one shared pool across the whole
// portfolio, and the per-token exposure risk guard still aggregates across strategies
// (see getTotalTokenExposureUsd) so it stays a true portfolio-level cap.
//
// Every fill is charged a realistic estimated fee (config.ts `estimateTradeFeeUsd` —
// Solana tx fee + priority fee + swap fee + slippage buffer). This matters: paper P&L
// against a fee-free simulator would look better than reality, which is actively
// misleading for anyone about to fund a real account based on these numbers. The
// buy-side fee is folded into the position's cost basis (not just deducted from cash)
// so it's honestly reflected in both unrealized P&L immediately and realized P&L once
// the position closes — otherwise "Realized P&L" would silently under-report true
// round-trip cost by exactly the entry fee.

const ASSUMED_LIQUIDITY_USD = 500_000; // phase-1 simplification, see riskManager.ts
const SIMULATED_SLIPPAGE_BPS = 10;
const DUST_QUANTITY = 1e-9;

export type SignalOutcome =
  | { executed: true; trade: Trade; portfolio: PortfolioSnapshot }
  | { executed: false; reason: string };

function todayDateString(): string {
  return new Date().toISOString().slice(0, 10);
}

function getMeta() {
  const meta = db.select().from(portfolioMeta).where(eq(portfolioMeta.id, "singleton")).get();
  if (!meta) throw new Error("portfolio_meta singleton row missing; db not initialized correctly");
  return meta;
}

// positions is keyed by (strategy_config_id, simulated) since execution/liveExecutor.ts
// started sub-ledgering live positions in the same table (see db/schema.ts) — every query
// in this file filters simulated=true explicitly so a config's live position (if any)
// never gets read, valued, or overwritten by the paper path, and vice versa.
function getPosition(strategyConfigId: string, tokenMint: string, tokenSymbol: string) {
  return (
    db
      .select()
      .from(positions)
      .where(and(eq(positions.strategyConfigId, strategyConfigId), eq(positions.simulated, true)))
      .get() ?? {
      strategyConfigId,
      tokenMint,
      tokenSymbol,
      quantity: 0,
      avgEntryPriceUsd: 0,
      stopLossPriceUsd: null as number | null,
    }
  );
}

function valuePosition(pos: { tokenMint: string; quantity: number; avgEntryPriceUsd: number }): number {
  const latest = priceCache.latest(pos.tokenMint);
  const price = latest?.priceUsd ?? pos.avgEntryPriceUsd;
  return pos.quantity * price;
}

/** Total USD value held in a token across EVERY strategy's sub-ledger — used for the
 * per-token exposure risk guard, which must stay a portfolio-wide cap even though
 * positions themselves are now tracked per strategy. */
export function getTotalTokenExposureUsd(tokenMint: string): number {
  const rows = db
    .select()
    .from(positions)
    .where(and(eq(positions.tokenMint, tokenMint), eq(positions.simulated, true)))
    .all()
    .filter((p) => p.quantity > DUST_QUANTITY);
  return rows.reduce((sum, p) => sum + valuePosition(p), 0);
}

/** All open PAPER positions (across every strategy config) in a given token — used to run
 * the mandatory stop-loss check against each one independently on every price tick. */
export function getOpenPositionsForToken(tokenMint: string): (typeof positions.$inferSelect)[] {
  return db
    .select()
    .from(positions)
    .where(and(eq(positions.tokenMint, tokenMint), eq(positions.simulated, true)))
    .all()
    .filter((p) => p.quantity > DUST_QUANTITY);
}

export function getPortfolioSnapshot(): PortfolioSnapshot {
  const meta = getMeta();
  const allPositions = db
    .select()
    .from(positions)
    .where(eq(positions.simulated, true))
    .all()
    .filter((p) => p.quantity > DUST_QUANTITY);

  const positionViews: Position[] = allPositions.map((p) => {
    const latest = priceCache.latest(p.tokenMint);
    const currentPriceUsd = latest?.priceUsd ?? p.avgEntryPriceUsd;
    return {
      strategyConfigId: p.strategyConfigId,
      tokenMint: p.tokenMint,
      tokenSymbol: p.tokenSymbol,
      quantity: p.quantity,
      avgEntryPriceUsd: p.avgEntryPriceUsd,
      currentPriceUsd,
      stopLossPriceUsd: p.stopLossPriceUsd,
      unrealizedPnlUsd: (currentPriceUsd - p.avgEntryPriceUsd) * p.quantity,
    };
  });

  const positionsValueUsd = positionViews.reduce((sum, p) => sum + p.quantity * p.currentPriceUsd, 0);
  const totalValueUsd = meta.cashUsd + positionsValueUsd;
  const unrealizedPnlUsd = positionViews.reduce((sum, p) => sum + p.unrealizedPnlUsd, 0);
  const dailyLossUsd = Math.max(0, meta.startOfDayValueUsd - totalValueUsd);

  return {
    timestamp: new Date().toISOString(),
    cashUsd: meta.cashUsd,
    positions: positionViews,
    realizedPnlUsd: meta.realizedPnlUsd,
    unrealizedPnlUsd,
    totalValueUsd,
    dailyLossUsd,
  };
}

/** Rolls the start-of-day baseline forward if the calendar day has changed. Must run
 * before any trade so the daily-loss guard compares against the right baseline. */
function rolloverDayIfNeeded(): void {
  const meta = getMeta();
  const today = todayDateString();
  if (meta.dayStartDate === today) return;
  const snapshot = getPortfolioSnapshot();
  db.update(portfolioMeta)
    .set({ dayStartDate: today, startOfDayValueUsd: snapshot.totalValueUsd })
    .where(eq(portfolioMeta.id, "singleton"))
    .run();
}

/** Read-only view of a strategy config's own position, for that strategy to consult. */
export function getCurrentPosition(strategyConfigId: string): { quantity: number; avgEntryPriceUsd: number } | null {
  const row = db
    .select()
    .from(positions)
    .where(and(eq(positions.strategyConfigId, strategyConfigId), eq(positions.simulated, true)))
    .get();
  if (!row || row.quantity <= DUST_QUANTITY) return null;
  return { quantity: row.quantity, avgEntryPriceUsd: row.avgEntryPriceUsd };
}

export function isPaused(): boolean {
  return getMeta().paused;
}

/** The kill switch. Halts all future simulated execution until resumed. */
export function setPaused(paused: boolean): void {
  db.update(portfolioMeta).set({ paused }).where(eq(portfolioMeta.id, "singleton")).run();
}

function applyFill(params: {
  action: "buy" | "sell";
  tokenMint: string;
  tokenSymbol: string;
  priceUsd: number;
  sizeUsd: number;
  strategyConfigId: string;
  strategyId: Trade["strategyId"];
  reason: string;
  limits: RiskLimits;
}): { trade: Trade; portfolio: PortfolioSnapshot } {
  const meta = getMeta();
  const pos = getPosition(params.strategyConfigId, params.tokenMint, params.tokenSymbol);
  const sizeToken = params.sizeUsd / params.priceUsd;
  const feeUsd = estimateTradeFeeUsd(params.sizeUsd);

  if (params.action === "buy") {
    const wasFlat = pos.quantity <= DUST_QUANTITY;
    const newQuantity = pos.quantity + sizeToken;
    // Fold the fee into the cost basis (not just a cash deduction) so it's honestly
    // reflected in this position's P&L rather than a silent drain invisible until you
    // look at total cash.
    const effectiveCostUsd = params.sizeUsd + feeUsd;
    const newAvgEntry = (pos.quantity * pos.avgEntryPriceUsd + effectiveCostUsd) / newQuantity;
    const stopLossPriceUsd = wasFlat ? computeStopLossPrice(params.priceUsd, params.limits) : pos.stopLossPriceUsd;

    db.insert(positions)
      .values({
        strategyConfigId: params.strategyConfigId,
        simulated: true,
        tokenMint: params.tokenMint,
        tokenSymbol: params.tokenSymbol,
        quantity: newQuantity,
        avgEntryPriceUsd: newAvgEntry,
        stopLossPriceUsd,
      })
      .onConflictDoUpdate({
        target: [positions.strategyConfigId, positions.simulated],
        set: { quantity: newQuantity, avgEntryPriceUsd: newAvgEntry, stopLossPriceUsd, tokenSymbol: params.tokenSymbol },
      })
      .run();

    db.update(portfolioMeta).set({ cashUsd: meta.cashUsd - params.sizeUsd - feeUsd }).where(eq(portfolioMeta.id, "singleton")).run();
  } else {
    const sellQuantity = Math.min(sizeToken, pos.quantity);
    const realizedDelta = (params.priceUsd - pos.avgEntryPriceUsd) * sellQuantity - feeUsd;
    const remainingQuantity = pos.quantity - sellQuantity;
    const isFlat = remainingQuantity <= DUST_QUANTITY;

    db.insert(positions)
      .values({
        strategyConfigId: params.strategyConfigId,
        simulated: true,
        tokenMint: params.tokenMint,
        tokenSymbol: params.tokenSymbol,
        quantity: isFlat ? 0 : remainingQuantity,
        avgEntryPriceUsd: isFlat ? 0 : pos.avgEntryPriceUsd,
        stopLossPriceUsd: isFlat ? null : pos.stopLossPriceUsd,
      })
      .onConflictDoUpdate({
        target: [positions.strategyConfigId, positions.simulated],
        set: {
          quantity: isFlat ? 0 : remainingQuantity,
          avgEntryPriceUsd: isFlat ? 0 : pos.avgEntryPriceUsd,
          stopLossPriceUsd: isFlat ? null : pos.stopLossPriceUsd,
        },
      })
      .run();

    db.update(portfolioMeta)
      .set({ cashUsd: meta.cashUsd + params.sizeUsd - feeUsd, realizedPnlUsd: meta.realizedPnlUsd + realizedDelta })
      .where(eq(portfolioMeta.id, "singleton"))
      .run();
  }

  const trade: Trade = {
    id: randomUUID(),
    strategyConfigId: params.strategyConfigId,
    strategyId: params.strategyId,
    action: params.action,
    tokenMint: params.tokenMint,
    tokenSymbol: params.tokenSymbol,
    priceUsd: params.priceUsd,
    sizeUsd: params.sizeUsd,
    sizeToken,
    feeUsd,
    reason: params.reason,
    simulated: true,
    txHash: null,
    network: null,
    confirmationSlot: null,
    timestamp: new Date().toISOString(),
  };
  db.insert(trades).values(trade).run();

  // Audit log per spec section 2: every "signed" (here: simulated) trade logged with
  // timestamp, strategy source, and reasoning.
  console.log(
    `[simulator] ${trade.action.toUpperCase()} ${trade.sizeToken.toFixed(6)} ${trade.tokenSymbol} @ $${trade.priceUsd} ` +
      `(fee=$${feeUsd.toFixed(4)}, strategy=${trade.strategyId}, reason="${trade.reason}")`,
  );

  return { trade, portfolio: getPortfolioSnapshot() };
}

/** Runs a strategy's signal through the risk manager and, if allowed, executes it against
 * the virtual ledger. This is the only path by which a Signal can become a Trade. */
export function executeSignal(signal: Signal, limits: RiskLimits): SignalOutcome {
  if (isPaused()) {
    return { executed: false, reason: "Trading is paused (kill switch active)" };
  }

  rolloverDayIfNeeded();

  const latest = priceCache.latest(signal.tokenMint);
  if (!latest) {
    return { executed: false, reason: `No price data available for ${signal.tokenSymbol} yet` };
  }

  const snapshot = getPortfolioSnapshot();
  const pos = getPosition(signal.strategyConfigId, signal.tokenMint, signal.tokenSymbol);

  const evalInput: RiskEvaluationInput = {
    action: signal.action === "hold" ? "buy" : signal.action,
    requestedSizeUsd: signal.sizeUsd,
    priceUsd: latest.priceUsd,
    totalPortfolioValueUsd: snapshot.totalValueUsd,
    cashUsd: snapshot.cashUsd,
    // Per-token exposure is a portfolio-wide cap: aggregate across every strategy's
    // sub-ledger in this token, not just this signal's own config.
    currentTokenExposureUsd: getTotalTokenExposureUsd(signal.tokenMint),
    // Sell-size capping, in contrast, must use only THIS config's own holdings — a
    // strategy can only sell what its own sub-ledger actually has.
    currentPositionQuantity: pos.quantity,
    dailyLossUsd: snapshot.dailyLossUsd,
    startOfDayValueUsd: snapshot.totalValueUsd + snapshot.dailyLossUsd, // pre-loss baseline
    limits,
    assumedLiquidityUsd: ASSUMED_LIQUIDITY_USD,
    simulatedSlippageBps: SIMULATED_SLIPPAGE_BPS,
  };

  if (signal.action === "hold") {
    return { executed: false, reason: "Signal was hold" };
  }

  const result = evaluateSignal(evalInput);
  if (!result.allowed) {
    console.log(`[simulator] REJECTED ${signal.action} ${signal.tokenSymbol} (strategy=${signal.strategyId}): ${result.reason}`);
    return { executed: false, reason: result.reason ?? "Rejected by risk manager" };
  }

  const { trade, portfolio } = applyFill({
    action: signal.action,
    tokenMint: signal.tokenMint,
    tokenSymbol: signal.tokenSymbol,
    priceUsd: latest.priceUsd,
    sizeUsd: result.adjustedSizeUsd ?? signal.sizeUsd,
    strategyConfigId: signal.strategyConfigId,
    strategyId: signal.strategyId,
    reason: result.reason ? `${signal.reason} (${result.reason})` : signal.reason,
    limits,
  });

  return { executed: true, trade, portfolio };
}

/** Mandatory protective stop-loss (spec section 7), enforced independent of any
 * strategy's own signal. Called once per open position (per strategy config) in a
 * token on every price tick for that token — see engine.ts, which iterates
 * getOpenPositionsForToken(). */
export function checkAndApplyStopLoss(strategyConfigId: string, tokenMint: string, limits: RiskLimits): SignalOutcome | null {
  if (isPaused()) return null;

  const latest = priceCache.latest(tokenMint);
  if (!latest) return null;

  const pos = getPosition(strategyConfigId, tokenMint, latest.tokenSymbol);
  if (pos.quantity <= DUST_QUANTITY || pos.stopLossPriceUsd === null) return null;
  if (latest.priceUsd > pos.stopLossPriceUsd) return null;

  rolloverDayIfNeeded();

  const { trade, portfolio } = applyFill({
    action: "sell",
    tokenMint,
    tokenSymbol: pos.tokenSymbol,
    priceUsd: latest.priceUsd,
    sizeUsd: pos.quantity * latest.priceUsd,
    strategyConfigId,
    strategyId: "risk-manager",
    reason: `Mandatory stop-loss triggered: price ${latest.priceUsd} <= stop ${pos.stopLossPriceUsd.toFixed(4)}`,
    limits,
  });

  return { executed: true, trade, portfolio };
}
