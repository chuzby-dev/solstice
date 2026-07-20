import { randomUUID } from "node:crypto";
import { and, eq } from "drizzle-orm";
import { Connection, PublicKey, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import type { PortfolioSnapshot, Position, RiskLimits, Signal, Trade } from "@trading-bot/shared";
import { db } from "../db/client.js";
import { liveWalletMeta, positions, trades } from "../db/schema.js";
import { priceCache } from "../market/priceCache.js";
import { getHotWalletPublicKey } from "../wallet/hotWallet.js";
import { getAppMode } from "./appMode.js";
import { computeStopLossPrice, evaluateSignal, type RiskEvaluationInput } from "./riskManager.js";
import { isPaused } from "./simulator.js";
import type { SwapExecutor } from "./swapExecutor.js";
import { config } from "../config.js";

// The live counterpart to execution/simulator.ts — same applyFill/executeSignal/
// checkAndApplyStopLoss shape, same risk-manager gating, same mandatory stop-loss, but
// every fill goes through a real, injected `SwapExecutor` (Stage 3: MockSwapExecutor,
// devnet self-ping; Stage 4: RealJupiterSwapExecutor) instead of in-memory math, and every
// number that matters is async because it's either an RPC call or a signed transaction.
//
// "Cash" has no DB row here (unlike paper's portfolio_meta.cashUsd) — it's the wallet's
// real on-chain USDC balance, queried live and short-TTL cached (see
// RpcWalletBalanceProvider below). SOL is dual-purpose: gas asset AND a tradable position
// like any other token, sub-ledgered per strategy config exactly like paper positions are
// (see db/schema.ts's positions composite-PK comment) — this is a per-strategy attribution
// of the wallet's real holdings, not a second source of truth for the balance itself.
//
// The kill switch is shared with the paper path (isPaused()/setPaused() live in
// simulator.ts, backed by the one portfolio_meta.paused flag) — there is only ONE kill
// switch for the whole engine, not a separate one per mode.
//
// In-flight guard: a strategyConfigId is marked in-flight synchronously the moment
// executeSignal/checkAndApplyStopLoss is entered (before any await) and released in a
// `finally` once the whole operation (including the real RPC round trip) settles. A second
// call for the same config — whether another signal or the mandatory stop-loss check —
// is skipped outright rather than racing a second fill on top of an unconfirmed one. This
// lives here (not in engine.ts, which doesn't dispatch to this module until Stage 5) so
// it's provable now via integration tests against a slow FakeSwapExecutor.

const DUST_QUANTITY = 1e-9;
// Same Phase-1 simplification simulator.ts uses (no real Jupiter quote to derive these
// from yet — that's swapExecutor.ts's RealJupiterSwapExecutor, Stage 4). Duplicated
// rather than imported since simulator.ts keeps them module-private by design.
const ASSUMED_LIQUIDITY_USD = 500_000;
const SIMULATED_SLIPPAGE_BPS = 10;

// Hardcoded devnet-only, same as wallet/txBuilder.ts and wallet/walletRoutes.ts — no
// network toggle exists yet, so there is no code path here that could reach mainnet.
const connection = new Connection(config.solanaDevnetRpcUrl, "confirmed");
const USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const BALANCE_CACHE_TTL_MS = 5000;

export type SignalOutcome =
  | { executed: true; trade: Trade; portfolio: PortfolioSnapshot }
  | { executed: false; reason: string };

export interface WalletBalances {
  solBalance: number;
  usdcBalance: number;
}

export interface WalletBalanceProvider {
  getBalances(): Promise<WalletBalances>;
}

function hotWalletPubkeyOrThrow(): PublicKey {
  const pubkey = getHotWalletPublicKey();
  if (!pubkey) throw new Error("No hot wallet has been created yet");
  return new PublicKey(pubkey);
}

/** Real devnet RPC balance lookup, short-TTL cached (same idea as market/priceCache.ts) so
 * a burst of ticks in the same second doesn't hammer the RPC endpoint. Injectable — tests
 * use a fake instead of this, the same way hotWallet.ts's tests use InMemorySecretStore
 * instead of the real OS keychain. */
class RpcWalletBalanceProvider implements WalletBalanceProvider {
  private cache: { value: WalletBalances; expiresAt: number } | null = null;

  async getBalances(): Promise<WalletBalances> {
    if (this.cache && this.cache.expiresAt > Date.now()) return this.cache.value;

    const pubkey = hotWalletPubkeyOrThrow();
    const [lamports, tokenAccounts] = await Promise.all([
      connection.getBalance(pubkey),
      connection.getParsedTokenAccountsByOwner(pubkey, { programId: TOKEN_PROGRAM_ID }),
    ]);
    const usdcAccount = tokenAccounts.value.find((acc) => acc.account.data.parsed.info.mint === USDC_MINT);
    const value: WalletBalances = {
      solBalance: lamports / LAMPORTS_PER_SOL,
      usdcBalance: usdcAccount ? Number(usdcAccount.account.data.parsed.info.tokenAmount.uiAmountString ?? 0) : 0,
    };
    this.cache = { value, expiresAt: Date.now() + BALANCE_CACHE_TTL_MS };
    return value;
  }
}

export const defaultBalanceProvider: WalletBalanceProvider = new RpcWalletBalanceProvider();

const inFlight = new Set<string>();

/** Exposed for tests/diagnostics — whether a strategy config currently has a live trade
 * mid-flight (guarded against a second concurrent signal or stop-loss check). */
export function isLiveTradeInFlight(strategyConfigId: string): boolean {
  return inFlight.has(strategyConfigId);
}

// positions/trades are the SAME tables the paper path uses, keyed by (strategyConfigId,
// simulated) — every query here filters simulated=false explicitly so a config's paper
// position never gets read, valued, or overwritten by the live path, and vice versa.
function getPosition(strategyConfigId: string, tokenMint: string, tokenSymbol: string) {
  return (
    db
      .select()
      .from(positions)
      .where(and(eq(positions.strategyConfigId, strategyConfigId), eq(positions.simulated, false)))
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

/** Total USD value held live in a token across every strategy's sub-ledger — the live
 * counterpart of simulator.ts's getTotalTokenExposureUsd, used for the same per-token
 * exposure risk guard. */
export function getTotalTokenExposureUsd(tokenMint: string): number {
  const rows = db
    .select()
    .from(positions)
    .where(and(eq(positions.tokenMint, tokenMint), eq(positions.simulated, false)))
    .all()
    .filter((p) => p.quantity > DUST_QUANTITY);
  return rows.reduce((sum, p) => sum + valuePosition(p), 0);
}

export function getOpenPositionsForToken(tokenMint: string): (typeof positions.$inferSelect)[] {
  return db
    .select()
    .from(positions)
    .where(and(eq(positions.tokenMint, tokenMint), eq(positions.simulated, false)))
    .all()
    .filter((p) => p.quantity > DUST_QUANTITY);
}

export function getCurrentPosition(strategyConfigId: string): { quantity: number; avgEntryPriceUsd: number } | null {
  const row = db
    .select()
    .from(positions)
    .where(and(eq(positions.strategyConfigId, strategyConfigId), eq(positions.simulated, false)))
    .get();
  if (!row || row.quantity <= DUST_QUANTITY) return null;
  return { quantity: row.quantity, avgEntryPriceUsd: row.avgEntryPriceUsd };
}

/** Returns the existing live_wallet_meta singleton, or lazily creates it — seeded with
 * `totalValueUsd` as day-one's baseline, since (per db/schema.ts) this row deliberately
 * isn't pre-seeded at boot: the start-of-day baseline should reflect real wallet value at
 * first genuine live use, not an arbitrary boot-time placeholder. */
function getOrInitLiveMeta(totalValueUsd: number) {
  const existing = db.select().from(liveWalletMeta).where(eq(liveWalletMeta.id, "singleton")).get();
  if (existing) return existing;

  const now = new Date().toISOString();
  const row = { id: "singleton", realizedPnlUsd: 0, startOfDayValueUsd: totalValueUsd, dayStartDate: now.slice(0, 10), updatedAt: now };
  db.insert(liveWalletMeta).values(row).run();
  return row;
}

export async function getPortfolioSnapshot(balanceProvider: WalletBalanceProvider = defaultBalanceProvider): Promise<PortfolioSnapshot> {
  const { usdcBalance } = await balanceProvider.getBalances();

  const allPositions = db
    .select()
    .from(positions)
    .where(eq(positions.simulated, false))
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
  const totalValueUsd = usdcBalance + positionsValueUsd;
  const meta = getOrInitLiveMeta(totalValueUsd);
  const unrealizedPnlUsd = positionViews.reduce((sum, p) => sum + p.unrealizedPnlUsd, 0);
  const dailyLossUsd = Math.max(0, meta.startOfDayValueUsd - totalValueUsd);

  return {
    timestamp: new Date().toISOString(),
    cashUsd: usdcBalance,
    positions: positionViews,
    realizedPnlUsd: meta.realizedPnlUsd,
    unrealizedPnlUsd,
    totalValueUsd,
    dailyLossUsd,
  };
}

/** Rolls the live start-of-day baseline forward if the calendar day has changed — mirrors
 * simulator.ts's rolloverDayIfNeeded exactly, just against live_wallet_meta/real balances
 * instead of portfolio_meta/virtual cash. */
async function rolloverDayIfNeeded(balanceProvider: WalletBalanceProvider): Promise<void> {
  const snapshot = await getPortfolioSnapshot(balanceProvider); // also lazily initializes the meta row
  const meta = db.select().from(liveWalletMeta).where(eq(liveWalletMeta.id, "singleton")).get()!;
  const today = new Date().toISOString().slice(0, 10);
  if (meta.dayStartDate === today) return;

  db.update(liveWalletMeta)
    .set({ dayStartDate: today, startOfDayValueUsd: snapshot.totalValueUsd, updatedAt: new Date().toISOString() })
    .where(eq(liveWalletMeta.id, "singleton"))
    .run();
}

async function applyFill(params: {
  action: "buy" | "sell";
  tokenMint: string;
  tokenSymbol: string;
  priceUsd: number;
  sizeUsd: number;
  strategyConfigId: string;
  strategyId: Trade["strategyId"];
  reason: string;
  limits: RiskLimits;
  executor: SwapExecutor;
  balanceProvider: WalletBalanceProvider;
}): Promise<{ trade: Trade; portfolio: PortfolioSnapshot }> {
  const pos = getPosition(params.strategyConfigId, params.tokenMint, params.tokenSymbol);
  const fill = await params.executor.swap({ action: params.action, tokenMint: params.tokenMint, sizeUsd: params.sizeUsd, priceUsd: params.priceUsd });

  if (params.action === "buy") {
    const wasFlat = pos.quantity <= DUST_QUANTITY;
    const newQuantity = pos.quantity + fill.sizeToken;
    // Fee folded into cost basis, same reasoning as simulator.ts's applyFill.
    const effectiveCostUsd = params.sizeUsd + fill.feeUsd;
    const newAvgEntry = (pos.quantity * pos.avgEntryPriceUsd + effectiveCostUsd) / newQuantity;
    const stopLossPriceUsd = wasFlat ? computeStopLossPrice(params.priceUsd, params.limits) : pos.stopLossPriceUsd;

    db.insert(positions)
      .values({
        strategyConfigId: params.strategyConfigId,
        simulated: false,
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
  } else {
    const meta = db.select().from(liveWalletMeta).where(eq(liveWalletMeta.id, "singleton")).get();
    if (!meta) throw new Error("live_wallet_meta singleton row missing — getPortfolioSnapshot() must run before the first live fill");

    const sellQuantity = Math.min(fill.sizeToken, pos.quantity);
    const realizedDelta = (params.priceUsd - pos.avgEntryPriceUsd) * sellQuantity - fill.feeUsd;
    const remainingQuantity = pos.quantity - sellQuantity;
    const isFlat = remainingQuantity <= DUST_QUANTITY;

    db.insert(positions)
      .values({
        strategyConfigId: params.strategyConfigId,
        simulated: false,
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

    db.update(liveWalletMeta)
      .set({ realizedPnlUsd: meta.realizedPnlUsd + realizedDelta, updatedAt: new Date().toISOString() })
      .where(eq(liveWalletMeta.id, "singleton"))
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
    sizeToken: fill.sizeToken,
    feeUsd: fill.feeUsd,
    reason: params.reason,
    simulated: false,
    txHash: fill.txHash,
    // Was hardcoded to "devnet" from Stage 3 (when MockSwapExecutor was the only live
    // executor) — RealJupiterSwapExecutor genuinely submits to mainnet now, and
    // tradingMode='live' is only ever reachable with network='mainnet' (see
    // execution/appMode.ts), so this must reflect the real current network, not a stale
    // constant.
    network: getAppMode().network,
    confirmationSlot: fill.confirmationSlot,
    timestamp: new Date().toISOString(),
  };
  db.insert(trades).values(trade).run();

  console.log(
    `[liveExecutor] ${trade.action.toUpperCase()} ${trade.sizeToken.toFixed(6)} ${trade.tokenSymbol} @ $${trade.priceUsd} ` +
      `(fee=$${fill.feeUsd.toFixed(4)}, strategy=${trade.strategyId}, tx=${trade.txHash})`,
  );

  return { trade, portfolio: await getPortfolioSnapshot(params.balanceProvider) };
}

/** Live counterpart to simulator.ts's executeSignal — same risk-manager gating, same
 * "only path a Signal can become a Trade" role, but async and backed by a real signed
 * transaction via `executor`. */
export async function executeSignal(
  signal: Signal,
  limits: RiskLimits,
  executor: SwapExecutor,
  balanceProvider: WalletBalanceProvider = defaultBalanceProvider,
): Promise<SignalOutcome> {
  if (isPaused()) {
    return { executed: false, reason: "Trading is paused (kill switch active)" };
  }
  if (isLiveTradeInFlight(signal.strategyConfigId)) {
    return { executed: false, reason: "A live trade for this strategy is already in flight" };
  }

  inFlight.add(signal.strategyConfigId);
  try {
    await rolloverDayIfNeeded(balanceProvider);

    const latest = priceCache.latest(signal.tokenMint);
    if (!latest) {
      return { executed: false, reason: `No price data available for ${signal.tokenSymbol} yet` };
    }
    if (signal.action === "hold") {
      return { executed: false, reason: "Signal was hold" };
    }

    const snapshot = await getPortfolioSnapshot(balanceProvider);
    const pos = getPosition(signal.strategyConfigId, signal.tokenMint, signal.tokenSymbol);

    // Real market data for the signal's REQUESTED size, if the executor can offer it (see
    // SwapExecutor.previewRiskMapping's doc) — falls back to the Phase-1 placeholders for
    // any executor that can't (e.g. MockSwapExecutor), or if the real lookup itself fails
    // (network hiccup fetching a quote is not a reason to fall through to an *unchecked*
    // trade — but it also shouldn't crash the whole signal attempt when a conservative
    // placeholder-based check can run instead).
    let riskMapping: { assumedLiquidityUsd: number; simulatedSlippageBps: number } = {
      assumedLiquidityUsd: ASSUMED_LIQUIDITY_USD,
      simulatedSlippageBps: SIMULATED_SLIPPAGE_BPS,
    };
    if (executor.previewRiskMapping) {
      try {
        riskMapping = await executor.previewRiskMapping({
          action: signal.action,
          tokenMint: signal.tokenMint,
          sizeUsd: signal.sizeUsd,
          priceUsd: latest.priceUsd,
        });
      } catch (err) {
        console.error(`[liveExecutor] real risk-mapping preview failed for ${signal.strategyConfigId}, falling back to placeholders:`, err);
      }
    }

    const evalInput: RiskEvaluationInput = {
      action: signal.action,
      requestedSizeUsd: signal.sizeUsd,
      priceUsd: latest.priceUsd,
      totalPortfolioValueUsd: snapshot.totalValueUsd,
      cashUsd: snapshot.cashUsd,
      currentTokenExposureUsd: getTotalTokenExposureUsd(signal.tokenMint),
      currentPositionQuantity: pos.quantity,
      dailyLossUsd: snapshot.dailyLossUsd,
      startOfDayValueUsd: snapshot.totalValueUsd + snapshot.dailyLossUsd,
      limits,
      assumedLiquidityUsd: riskMapping.assumedLiquidityUsd,
      simulatedSlippageBps: riskMapping.simulatedSlippageBps,
    };

    const result = evaluateSignal(evalInput);
    if (!result.allowed) {
      console.log(`[liveExecutor] REJECTED ${signal.action} ${signal.tokenSymbol} (strategy=${signal.strategyId}): ${result.reason}`);
      return { executed: false, reason: result.reason ?? "Rejected by risk manager" };
    }

    const { trade, portfolio } = await applyFill({
      action: signal.action,
      tokenMint: signal.tokenMint,
      tokenSymbol: signal.tokenSymbol,
      priceUsd: latest.priceUsd,
      sizeUsd: result.adjustedSizeUsd ?? signal.sizeUsd,
      strategyConfigId: signal.strategyConfigId,
      strategyId: signal.strategyId,
      reason: result.reason ? `${signal.reason} (${result.reason})` : signal.reason,
      limits,
      executor,
      balanceProvider,
    });

    return { executed: true, trade, portfolio };
  } finally {
    inFlight.delete(signal.strategyConfigId);
  }
}

/** Live counterpart to simulator.ts's checkAndApplyStopLoss — mandatory, independent of
 * any strategy's own signal, bypasses evaluateSignal entirely. Skips (rather than queues)
 * if this config already has a live trade in flight, same as executeSignal. */
export async function checkAndApplyStopLoss(
  strategyConfigId: string,
  tokenMint: string,
  limits: RiskLimits,
  executor: SwapExecutor,
  balanceProvider: WalletBalanceProvider = defaultBalanceProvider,
): Promise<SignalOutcome | null> {
  if (isPaused()) return null;
  if (isLiveTradeInFlight(strategyConfigId)) return null;

  const latest = priceCache.latest(tokenMint);
  if (!latest) return null;

  const pos = getPosition(strategyConfigId, tokenMint, latest.tokenSymbol);
  if (pos.quantity <= DUST_QUANTITY || pos.stopLossPriceUsd === null) return null;
  if (latest.priceUsd > pos.stopLossPriceUsd) return null;

  inFlight.add(strategyConfigId);
  try {
    await rolloverDayIfNeeded(balanceProvider);

    const { trade, portfolio } = await applyFill({
      action: "sell",
      tokenMint,
      tokenSymbol: pos.tokenSymbol,
      priceUsd: latest.priceUsd,
      sizeUsd: pos.quantity * latest.priceUsd,
      strategyConfigId,
      strategyId: "risk-manager",
      reason: `Mandatory stop-loss triggered: price ${latest.priceUsd} <= stop ${pos.stopLossPriceUsd.toFixed(4)}`,
      limits,
      executor,
      balanceProvider,
    });

    return { executed: true, trade, portfolio };
  } finally {
    inFlight.delete(strategyConfigId);
  }
}
