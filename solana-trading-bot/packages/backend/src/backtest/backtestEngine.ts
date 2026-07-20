import { randomUUID } from "node:crypto";
import type { BuiltInStrategyId, PriceTick, RiskLimits, StrategyConfig, Trade } from "@trading-bot/shared";
import { PriceCache } from "../market/priceCache.js";
import { strategyRegistry } from "../strategy-engine/registry.js";
import type { StrategyContext } from "../strategy-engine/StrategyBase.js";
import { evaluateSignal, type RiskEvaluationInput } from "../execution/riskManager.js";
import { BacktestLedger, type LedgerFill } from "./ledger.js";

// Mirrors strategy-engine/engine.ts's `priceCache.recent(tick.tokenMint, 6000)` exactly,
// so a strategy sees the same amount of lookback history in backtest as it would live.
const CTX_PRICE_HISTORY_TICKS = 6000;

// Mirrors execution/simulator.ts:30-31 (ASSUMED_LIQUIDITY_USD / SIMULATED_SLIPPAGE_BPS) —
// both Phase-1 placeholders in the live simulator too, not a backtest-only simplification.
const ASSUMED_LIQUIDITY_USD = 500_000;
const SIMULATED_SLIPPAGE_BPS = 10;

export interface EquityPoint {
  timestamp: string;
  totalValueUsd: number;
}

export interface BacktestResult {
  strategyId: BuiltInStrategyId;
  params: Record<string, number>;
  startingCashUsd: number;
  endingTotalValueUsd: number;
  fills: LedgerFill[];
  trades: Trade[];
  equityCurve: EquityPoint[];
  realizedPnlUsd: number;
  totalFeesUsd: number;
}

/** Replays a chronological PriceTick series through one strategy config, reusing the real
 * `onInterval` and `evaluateSignal` logic unchanged (see ledger.ts's header comment for why
 * the execution/ledger layer itself is a separate in-memory implementation). Mirrors
 * strategy-engine/engine.ts's per-tick ordering: mandatory stop-loss check first, then the
 * strategy's own signal. `ctx.now` is always the replayed tick's own timestamp, never wall
 * clock — every strategy with time-based logic (cooldowns, time stops, DCA intervals)
 * depends on this being the simulated time. */
export function runBacktest(strategyId: BuiltInStrategyId, params: Record<string, number>, ticks: PriceTick[], limits: RiskLimits, startingCashUsd: number): BacktestResult {
  const strategy = strategyRegistry[strategyId];
  if (!strategy) throw new Error(`Unknown strategy: ${strategyId}`);
  if (ticks.length === 0) throw new Error("No historical ticks supplied to runBacktest");

  const tokenMint = ticks[0]!.tokenMint;
  const tokenSymbol = ticks[0]!.tokenSymbol;
  const configId = randomUUID();
  const config: StrategyConfig = {
    id: configId,
    strategyId,
    tokenMint,
    tokenSymbol,
    params,
    active: true,
    createdAt: ticks[0]!.timestamp,
  };

  const priceCache = new PriceCache();
  const ledger = new BacktestLedger(startingCashUsd, tokenMint, tokenSymbol, configId, strategyId, ticks[0]!.timestamp);
  const equityCurve: EquityPoint[] = [];

  for (const tick of ticks) {
    priceCache.push(tick);

    ledger.checkStopLoss(tick.priceUsd, tick.timestamp);

    const ctx: StrategyContext = {
      config,
      priceHistory: priceCache.recent(tick.tokenMint, CTX_PRICE_HISTORY_TICKS),
      latestPrice: tick,
      now: new Date(tick.timestamp),
      currentPosition: ledger.currentPosition,
      lastSignalAt: ledger.lastTradeAt,
    };

    const signal = strategy.onInterval(ctx);
    if (signal && signal.action !== "hold") {
      ledger.rolloverDayIfNeeded(tick.timestamp, tick.priceUsd);
      const { dailyLossUsd, startOfDayValueUsd } = ledger.dailyLossEvalInputs(tick.priceUsd);

      const evalInput: RiskEvaluationInput = {
        action: signal.action,
        requestedSizeUsd: signal.sizeUsd,
        priceUsd: tick.priceUsd,
        totalPortfolioValueUsd: ledger.portfolioValueUsd(tick.priceUsd),
        cashUsd: ledger.cashUsd,
        currentTokenExposureUsd: ledger.position.quantity * tick.priceUsd,
        currentPositionQuantity: ledger.position.quantity,
        dailyLossUsd,
        startOfDayValueUsd,
        limits,
        assumedLiquidityUsd: ASSUMED_LIQUIDITY_USD,
        simulatedSlippageBps: SIMULATED_SLIPPAGE_BPS,
      };

      const result = evaluateSignal(evalInput);
      if (result.allowed) {
        const sizeUsd = result.adjustedSizeUsd ?? signal.sizeUsd;
        if (signal.action === "buy") ledger.applyBuy(tick.priceUsd, sizeUsd, limits, signal.reason, tick.timestamp);
        else ledger.applySell(tick.priceUsd, sizeUsd, signal.reason, tick.timestamp);
      }
    }

    equityCurve.push({ timestamp: tick.timestamp, totalValueUsd: ledger.portfolioValueUsd(tick.priceUsd) });
  }

  return {
    strategyId,
    params,
    startingCashUsd,
    endingTotalValueUsd: equityCurve[equityCurve.length - 1]!.totalValueUsd,
    fills: ledger.fills,
    trades: ledger.trades,
    equityCurve,
    realizedPnlUsd: ledger.realizedPnlUsd,
    totalFeesUsd: ledger.totalFeesUsd,
  };
}
