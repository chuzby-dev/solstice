import { and, desc, eq } from "drizzle-orm";
import type { BuiltInStrategyId, PortfolioSnapshot, PriceTick, Trade, WsMessage } from "@trading-bot/shared";
import { db } from "../db/client.js";
import { strategyConfigs, trades } from "../db/schema.js";
import { priceFeed } from "../market/priceFeed.js";
import { priceCache } from "../market/priceCache.js";
import * as whaleWatcher from "../market/whaleWatcher.js";
import { checkAndApplyStopLoss, executeSignal, getCurrentPosition, getOpenPositionsForToken } from "../execution/simulator.js";
import * as liveExecutor from "../execution/liveExecutor.js";
import { getAppMode } from "../execution/appMode.js";
import { checkAndRunAutoSweep } from "../execution/autoSweep.js";
import { RealJupiterSwapExecutor } from "../execution/swapExecutor.js";
import { getRiskLimits } from "../execution/riskSettings.js";
import { strategyRegistry } from "./registry.js";
import type { StrategyContext } from "./StrategyBase.js";

// engine.ts's live-mode executor — real, submitting, mainnet-only (see
// RealJupiterSwapExecutor's own doc comment in swapExecutor.ts). One shared, stateless
// instance is enough; there is no per-call state to isolate.
const liveSwapExecutor = new RealJupiterSwapExecutor();

type Outcome = { executed: true; trade: Trade; portfolio: PortfolioSnapshot } | { executed: false; reason: string } | null;

function broadcastIfExecuted(outcome: Outcome): void {
  if (outcome?.executed) {
    broadcaster?.({ type: "trade", data: outcome.trade });
    broadcaster?.({ type: "portfolio", data: outcome.portfolio });
  }
}

type Broadcaster = (msg: WsMessage) => void;

let broadcaster: Broadcaster | null = null;
let unsubscribe: (() => void) | null = null;

/** Starts the scheduler loop: subscribes to real-time price ticks and, for each tick,
 * runs the mandatory stop-loss check and every active strategy targeting that token. */
export function startEngine(broadcast: Broadcaster): void {
  broadcaster = broadcast;
  priceFeed.start();
  whaleWatcher.start();
  if (!unsubscribe) {
    unsubscribe = priceFeed.subscribe(handleTick);
  }
}

/** The kill switch's engine-side half: stops the scheduler from reacting to further
 * ticks. Combined with execution/simulator.ts `setPaused`, which blocks execution even
 * if a tick slips through mid-shutdown. */
export function stopEngine(): void {
  unsubscribe?.();
  unsubscribe = null;
  priceFeed.stop();
  whaleWatcher.stop();
}

function handleTick(tick: PriceTick): void {
  broadcaster?.({ type: "price_tick", data: tick });

  // Fire-and-forget, same reasoning as the live-mode dispatch below — internally
  // throttled (see autoSweep.ts) so this is a no-op on all but roughly one tick a minute.
  checkAndRunAutoSweep().catch((err) => console.error("[engine] auto-sweep check failed:", err));

  const limits = getRiskLimits();
  // Read once per tick (not once per config) so every strategy this tick sees a
  // consistent mode, and dispatch to simulator.* (paper) or liveExecutor.* (live)
  // accordingly — the one place in the whole engine that branches on it. `network` is
  // irrelevant here: it's derived entirely from tradingMode (see execution/appMode.ts),
  // so 'live' reaching this point always means mainnet, structurally.
  const isLive = getAppMode().tradingMode === "live";

  // Mandatory stop-loss runs per open position (i.e. per strategy config), since
  // positions are now sub-ledgered per strategy rather than shared per token. The live
  // calls are fire-and-forget: liveExecutor's functions are async (real RPC/signing can
  // take seconds), and awaiting inline here would stall every other token/strategy's tick
  // processing behind one pending live trade. Not awaiting is safe because
  // liveExecutor.ts owns its own in-flight guard per strategyConfigId internally (see its
  // header comment) — a tick that lands while a config's previous live call is still
  // pending is skipped by that guard, not raced against it.
  const openPositions = isLive ? liveExecutor.getOpenPositionsForToken(tick.tokenMint) : getOpenPositionsForToken(tick.tokenMint);
  for (const openPosition of openPositions) {
    if (isLive) {
      liveExecutor
        .checkAndApplyStopLoss(openPosition.strategyConfigId, tick.tokenMint, limits, liveSwapExecutor)
        .then(broadcastIfExecuted)
        .catch((err) => console.error(`[engine] live stop-loss check failed for ${openPosition.strategyConfigId}:`, err));
    } else {
      broadcastIfExecuted(checkAndApplyStopLoss(openPosition.strategyConfigId, tick.tokenMint, limits));
    }
  }

  const activeConfigs = db
    .select()
    .from(strategyConfigs)
    .where(and(eq(strategyConfigs.tokenMint, tick.tokenMint), eq(strategyConfigs.active, true)))
    .all();

  for (const cfg of activeConfigs) {
    const strategyId = cfg.strategyId as BuiltInStrategyId;
    const strategy = strategyRegistry[strategyId];
    if (!strategy) continue;

    // Filtered by mode so a config's cooldown/lastSignalAt logic never mixes paper and
    // live trade history — the two are separate ledgers now (see db/schema.ts's positions
    // composite-PK comment), and this query predates that split.
    const lastTrade = db
      .select()
      .from(trades)
      .where(and(eq(trades.strategyConfigId, cfg.id), eq(trades.simulated, !isLive)))
      .orderBy(desc(trades.timestamp))
      .limit(1)
      .get();

    const ctx: StrategyContext = {
      config: {
        id: cfg.id,
        strategyId,
        tokenMint: cfg.tokenMint,
        tokenSymbol: cfg.tokenSymbol,
        params: JSON.parse(cfg.params) as Record<string, number>,
        watchedWalletAddress: cfg.watchedWalletAddress ?? undefined,
        active: cfg.active,
        createdAt: cfg.createdAt,
      },
      // Was 200, then 600 — now matches priceCache's own HISTORY_LIMIT (6000, ~200min at
      // the default 2s poll) since that data is already being retained anyway. Raised
      // again specifically for Dip Reversion's 90-180min lookback (see
      // strategy-engine/strategies/dipReversion.ts and docs/ARCHITECTURE.md
      // "Backtesting"); purely additive for every other strategy — more available history
      // can only let an indicator compute where it previously returned null, never forces
      // a different signal.
      priceHistory: priceCache.recent(tick.tokenMint, 6000),
      latestPrice: tick,
      now: new Date(),
      currentPosition: isLive ? liveExecutor.getCurrentPosition(cfg.id) : getCurrentPosition(cfg.id),
      lastSignalAt: lastTrade ? new Date(lastTrade.timestamp) : null,
    };

    const signal = strategy.onInterval(ctx);
    if (!signal) continue;

    if (isLive) {
      liveExecutor
        .executeSignal(signal, limits, liveSwapExecutor)
        .then((outcome) => {
          broadcastIfExecuted(outcome);
          if (!outcome.executed) console.log(`[engine] live signal from ${cfg.id} not executed: ${outcome.reason}`);
        })
        .catch((err) => console.error(`[engine] live signal execution failed for ${cfg.id}:`, err));
    } else {
      const outcome = executeSignal(signal, limits);
      broadcastIfExecuted(outcome);
      if (!outcome.executed) console.log(`[engine] signal from ${cfg.id} not executed: ${outcome.reason}`);
    }
  }
}
