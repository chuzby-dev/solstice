// Types shared between the backend strategy/execution engine and the frontend GUI.
// Real signing exists now (see execution/liveExecutor.ts, wallet/hotWallet.ts) — a `Trade`
// can be genuinely `simulated: false` with a real `txHash`. It's still gated hard: live
// autonomous execution only ever runs against `network: "devnet"` today (see
// docs/ARCHITECTURE.md "Real hot wallet + live trading" — mainnet is a distinct, later,
// deliberately separate step), and `tradingMode` always resets to `"paper"` on server
// boot regardless of what was persisted.

export type RiskLevel = "low" | "medium" | "high";

export type BuiltInStrategyId =
  | "dca"
  | "momentum"
  | "mean-reversion"
  | "grid"
  | "rsi-macd"
  | "volatility-breakout"
  | "whale-copy"
  | "short-window-grid"
  | "range-scalper"
  | "confluence-scalper"
  | "fee-aware-scalper"
  | "dip-reversion"
  | "flash-dip-reversal"
  | "double-bottom-retest";

export type SignalAction = "buy" | "sell" | "hold";

/** Verdict from the most recent real (fee/execution-realistic, tuning/validation-split)
 * backtest against historical price data — see docs/ARCHITECTURE.md "Backtesting".
 * "profitable" means positive on BOTH the tuning and held-out validation windows;
 * "not-profitable" means negative on at least one, or an unvalidated/overfit result;
 * "untested" means not backtestable from price history alone (e.g. whale-copy). Manually
 * updated after each real backtest pass — not computed live, so it can go stale between
 * passes; treat it as a snapshot, not a guarantee. */
export type BacktestVerdict = "profitable" | "not-profitable" | "untested";

/** "paper" = the original, always-safe simulated execution path (execution/simulator.ts).
 * "live" = real signed transactions via the app's own hot wallet
 * (execution/liveExecutor.ts). One GLOBAL switch, not per-strategy — every active
 * strategy moves together. Always resets to "paper" on server boot regardless of what was
 * persisted (see db/client.ts) — live trading never silently resumes after a restart. */
export type TradingMode = "paper" | "live";

/** Which Solana cluster a live trade (or wallet action) targets. Live autonomous trading
 * only runs on devnet as of this build — mainnet requires a separate, later, explicitly
 * confirmed step (see docs/ARCHITECTURE.md). */
export type Network = "devnet" | "mainnet";

/** GET/PUT /api/mode's shape. */
export interface AppModeState {
  tradingMode: TradingMode;
  network: Network;
}

/** Static, non-configurable metadata describing a strategy type for the selector UI. */
export interface StrategyMetadata {
  id: BuiltInStrategyId;
  name: string;
  description: string;
  riskLevel: RiskLevel;
  defaultParams: Record<string, number>;
  /** Human-readable description of each param, for the strategy builder form. */
  paramDescriptions: Record<string, string>;
  backtestVerdict: BacktestVerdict;
}

/** A user-configured, instantiable strategy (a strategy type + specific params + token). */
export interface StrategyConfig {
  id: string; // uuid, unique per configured instance
  strategyId: BuiltInStrategyId;
  tokenMint: string;
  tokenSymbol: string;
  params: Record<string, number>;
  /** Only used by "whale-copy": the on-chain address whose trades this config mirrors.
   * Read-only monitoring — never used to sign or send anything on that wallet's behalf. */
  watchedWalletAddress?: string;
  active: boolean;
  createdAt: string; // ISO timestamp
}

/** Emitted by a strategy's onInterval() when it wants the engine to act. */
export interface Signal {
  strategyConfigId: string;
  strategyId: BuiltInStrategyId;
  action: SignalAction;
  tokenMint: string;
  tokenSymbol: string;
  /** Suggested trade size in USD; the risk manager may shrink or reject this. */
  sizeUsd: number;
  reason: string;
  timestamp: string;
}

/** A trade that has passed risk checks and been recorded by the simulator.
 * strategyId is "risk-manager" for forced exits (e.g. mandatory stop-loss) that are not
 * attributable to a strategy's own signal. */
export interface Trade {
  id: string;
  strategyConfigId: string;
  strategyId: BuiltInStrategyId | "risk-manager";
  action: SignalAction;
  tokenMint: string;
  tokenSymbol: string;
  priceUsd: number;
  sizeUsd: number;
  sizeToken: number;
  /** Estimated real-world trading cost for this leg (Solana tx + priority fee + swap
   * fee + slippage buffer), deducted from cash. See config.ts `estimateTradeFeeUsd`. */
  feeUsd: number;
  reason: string;
  /** false for a real, on-chain live trade (execution/liveExecutor.ts) — true for every
   * paper trade, as it always has been. */
  simulated: boolean;
  /** The real transaction signature for a live trade; always null for a paper trade
   * (never touched a network). */
  txHash: string | null;
  /** Which cluster a live trade was signed against; null for a paper trade. Currently
   * always "devnet" or null — see the file header. */
  network: Network | null;
  /** Slot the transaction was confirmed at. Null for a paper trade, or for a live trade
   * that was submitted but timed out waiting for confirmation (it may still land late —
   * see docs/ARCHITECTURE.md's failure-mode notes; check `txHash` on an explorer). */
  confirmationSlot: number | null;
  timestamp: string;
}

/** A single strategy config's own position in a token. Positions are sub-ledgered per
 * strategy, not per token — two strategies trading the same token show up as two
 * separate Position entries in a PortfolioSnapshot. */
export interface Position {
  strategyConfigId: string;
  tokenMint: string;
  tokenSymbol: string;
  quantity: number;
  avgEntryPriceUsd: number;
  currentPriceUsd: number;
  stopLossPriceUsd: number | null;
  unrealizedPnlUsd: number;
}

export interface PortfolioSnapshot {
  timestamp: string;
  cashUsd: number;
  positions: Position[];
  realizedPnlUsd: number;
  unrealizedPnlUsd: number;
  totalValueUsd: number;
  dailyLossUsd: number;
}

export interface PriceTick {
  tokenMint: string;
  tokenSymbol: string;
  priceUsd: number;
  timestamp: string;
}

/** Non-negotiable risk defaults per spec section 7. All are enforced in riskManager.ts. */
export interface RiskLimits {
  maxPositionPct: number; // max position size as % of total portfolio value
  maxDailyLossPct: number; // % of starting-of-day portfolio value; breach -> auto-pause
  perTokenExposurePct: number; // max % of portfolio in any single token
  defaultStopLossPct: number; // mandatory stop-loss % applied to every position
  maxSlippageBps: number; // basis points
  maxPriceImpactPct: number;
}

export interface RiskCheckResult {
  allowed: boolean;
  adjustedSizeUsd?: number;
  reason?: string;
  triggeredGuard?: keyof RiskLimits | "insufficient_balance" | "liquidity";
}

/** Summary stats for one backtest run — mirrors backend/src/backtest/metrics.ts's
 * BacktestMetrics exactly. round-trip = one full buy→sell (or partial-sell) cycle;
 * win rate / profit factor / avg hold time are null when there are none yet. */
export interface BacktestMetrics {
  totalReturnPct: number;
  totalReturnUsd: number;
  tradeCount: number;
  roundTripCount: number;
  winRate: number | null;
  profitFactor: number | null;
  maxDrawdownPct: number;
  avgHoldMinutes: number | null;
  totalFeesUsd: number;
  feeDragPct: number;
}

/** Response for POST /api/backtest/run — one strategy config replayed against real
 * historical price data, using the submitted params as-is (no tuning). */
export interface BacktestRunResult {
  strategyId: BuiltInStrategyId;
  tokenSymbol: string;
  candleInterval: string;
  candleCount: number;
  metrics: BacktestMetrics;
}

/** One parameter set's result within a tune sweep — `validationMetrics` is null only when
 * the historical range was too short to hold out a validation window at all. */
export interface BacktestTuneTrial {
  params: Record<string, number>;
  metrics: BacktestMetrics;
  validationMetrics: BacktestMetrics | null;
}

/** Response for POST /api/backtest/tune. `tickCountParams` lists which of this strategy's
 * params are literal price-tick lookbacks (see docs/ARCHITECTURE.md "Backtesting") — the
 * sweep never varies these, so `best.params` is always safe to apply to a live config
 * as-is; this list is purely informational for the UI. */
export interface BacktestTuneResult {
  strategyId: BuiltInStrategyId;
  tokenSymbol: string;
  candleInterval: string;
  candleCount: number;
  baseline: BacktestTuneTrial;
  best: BacktestTuneTrial | null;
  tickCountParams: string[];
}

/** Status of the app's own server-custodied hot wallet (real signing, OS-keychain-backed
 * private key) — distinct from the frontend's read-only external wallet-adapter connect
 * (Phantom/Solflare). `exists: false` means no wallet has been created yet. The private
 * key itself is never included here or in any other status/list response — the ONE
 * exception is the explicit, user-confirmed POST /api/wallet/hot/export-key (see
 * HotWalletKeyExport below). */
export interface HotWalletStatus {
  exists: boolean;
  pubkey: string | null;
  createdAt: string | null;
}

/** Response for POST /api/wallet/hot/export-key — the raw private key, base58-encoded
 * (the format Phantom/Solflare's "Import Private Key" accepts). No recovery phrase exists
 * for this wallet (see wallet/hotWallet.ts's exportPrivateKeyBase58 doc: it was generated
 * as a raw keypair, never derived from a BIP39 mnemonic). Anyone holding this string has
 * complete, irreversible control of the wallet — the frontend must never persist it
 * beyond the single reveal (no localStorage, no query cache) and must clear it from
 * memory once the user navigates away or explicitly hides it. */
export interface HotWalletKeyExport {
  pubkey: string | null;
  privateKeyBase58: string;
}

/** Response for POST /api/wallet/send — a real, on-chain signed transfer from the bot's
 * hot wallet, on whichever network is currently active. `confirmationSlot: null` means
 * the send timed out waiting for confirmation, not that it failed — it may still land;
 * check `explorerUrl`. */
export interface WalletSendResult {
  txHash: string;
  confirmationSlot: number | null;
  explorerUrl: string;
}

/** Response for POST /api/wallet/send/preview — a real fee quote and dry-run simulation
 * of the exact transaction a subsequent POST /api/wallet/send would submit, computed
 * without ever signing or broadcasting anything. `simulationError` non-null means the
 * real send would fail for this reason; the UI should warn loudly (not silently block —
 * the on-chain state simulated against can shift between preview and confirm). */
export interface TransferPreview {
  estimatedFeeSol: number;
  simulationError: string | null;
  logs: string[];
}

/** One row in the Wallet tab's transaction history — a merged, chronological view of
 * every real on-chain transaction this wallet has signed: manual sends (`kind: "send"`)
 * and live strategy trades (`kind: "trade"`). Two different underlying DB tables
 * (wallet_sends and trades), unified here so the UI doesn't need to know the difference. */
export interface WalletTransaction {
  id: string;
  kind: "send" | "trade";
  tokenSymbol: string;
  amount: number;
  network: Network;
  txHash: string;
  confirmationSlot: number | null;
  timestamp: string;
  /** Only present for kind: "send". */
  destination?: string;
  /** Only present for kind: "trade". */
  action?: SignalAction;
  /** Only present for kind: "trade". */
  strategyId?: BuiltInStrategyId | "risk-manager";
}

/** GET/PUT /api/wallet/auto-sweep. Off by default (`enabled: false`) — a standing rule
 * that moves real funds with no per-transfer confirmation once armed, so this always
 * starts empty/disabled; the user must explicitly fill in every field and confirm via the
 * Wallet tab. Applies to whichever network is currently active (see AppModeState) — swept
 * amounts are real SOL/USDC, only ever meaningful once real funds are in the wallet. */
export interface AutoSweepConfig {
  enabled: boolean;
  tokenMint: string;
  tokenSymbol: string;
  /** Keep this much of `tokenMint` in the wallet; anything above it is swept to
   * `destination` the next time the check runs. */
  thresholdAmount: number;
  destination: string;
}

export type WsMessage =
  | { type: "price_tick"; data: PriceTick }
  | { type: "portfolio"; data: PortfolioSnapshot }
  | { type: "trade"; data: Trade }
  | { type: "wallet_send"; data: WalletTransaction }
  | { type: "engine_status"; data: { paused: boolean } };
