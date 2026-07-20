import { sqliteTable, text, real, integer, primaryKey } from "drizzle-orm/sqlite-core";

export const strategyConfigs = sqliteTable("strategy_configs", {
  id: text("id").primaryKey(),
  strategyId: text("strategy_id").notNull(),
  tokenMint: text("token_mint").notNull(),
  tokenSymbol: text("token_symbol").notNull(),
  params: text("params").notNull(), // JSON-encoded Record<string, number>
  // Only set for "whale-copy": the on-chain address this config mirrors. Read-only —
  // never used to sign or send anything on that wallet's behalf.
  watchedWalletAddress: text("watched_wallet_address"),
  active: integer("active", { mode: "boolean" }).notNull().default(false),
  createdAt: text("created_at").notNull(),
});

export const trades = sqliteTable("trades", {
  id: text("id").primaryKey(),
  strategyConfigId: text("strategy_config_id").notNull(),
  strategyId: text("strategy_id").notNull(),
  action: text("action").notNull(), // 'buy' | 'sell'
  tokenMint: text("token_mint").notNull(),
  tokenSymbol: text("token_symbol").notNull(),
  priceUsd: real("price_usd").notNull(),
  sizeUsd: real("size_usd").notNull(),
  sizeToken: real("size_token").notNull(),
  feeUsd: real("fee_usd").notNull().default(0),
  reason: text("reason").notNull(),
  timestamp: text("timestamp").notNull(),
  // Added for real (live) trades — see execution/liveExecutor.ts. Every existing/paper
  // row is unambiguously simulated=true, txHash/network/confirmationSlot=null (they never
  // touched a network).
  simulated: integer("simulated", { mode: "boolean" }).notNull().default(true),
  txHash: text("tx_hash"),
  network: text("network"),
  confirmationSlot: integer("confirmation_slot"),
});

// One row per (STRATEGY CONFIG, simulated) pair, not per token: each configured strategy
// instance has its own independent position/stop-loss in the token it trades, so two
// strategies both trading e.g. SOL can't buy/sell out from under each other. The
// `simulated` half of the key means flipping a config from paper to live (or back) starts
// a fresh position rather than mixing real and virtual holdings in one row — a config's
// dormant paper position stays exactly as it was, untouched, while live starts flat. The
// shared virtual cash pool (portfolio_meta) is paper-only; live has no cash DB row at all
// (it's the wallet's real balance — see execution/liveExecutor.ts).
export const positions = sqliteTable(
  "positions",
  {
    strategyConfigId: text("strategy_config_id").notNull(),
    simulated: integer("simulated", { mode: "boolean" }).notNull().default(true),
    tokenMint: text("token_mint").notNull(),
    tokenSymbol: text("token_symbol").notNull(),
    quantity: real("quantity").notNull().default(0),
    avgEntryPriceUsd: real("avg_entry_price_usd").notNull().default(0),
    stopLossPriceUsd: real("stop_loss_price_usd"),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.strategyConfigId, table.simulated] }),
  }),
);

// Single-row table (id = 'singleton') holding the virtual portfolio's cash/PnL state
// and the engine's global pause flag (the kill switch).
export const portfolioMeta = sqliteTable("portfolio_meta", {
  id: text("id").primaryKey(),
  cashUsd: real("cash_usd").notNull(),
  realizedPnlUsd: real("realized_pnl_usd").notNull().default(0),
  startOfDayValueUsd: real("start_of_day_value_usd").notNull(),
  dayStartDate: text("day_start_date").notNull(), // YYYY-MM-DD
  paused: integer("paused", { mode: "boolean" }).notNull().default(false),
});

// Single-row table (id = 'singleton') holding the user-editable risk limits (Settings
// panel). Values are clamped to riskHardCeilings (config.ts) on every write.
export const riskSettings = sqliteTable("risk_settings", {
  id: text("id").primaryKey(),
  maxPositionPct: real("max_position_pct").notNull(),
  maxDailyLossPct: real("max_daily_loss_pct").notNull(),
  perTokenExposurePct: real("per_token_exposure_pct").notNull(),
  defaultStopLossPct: real("default_stop_loss_pct").notNull(),
  maxSlippageBps: real("max_slippage_bps").notNull(),
  maxPriceImpactPct: real("max_price_impact_pct").notNull(),
});

// At most one row (id = 'singleton'), only once a hot wallet has actually been created —
// its ABSENCE is the normal "no wallet yet" state, not an error, so unlike portfolioMeta/
// riskSettings this is never pre-seeded at startup. Only a non-secret pointer: the
// private key itself lives in the OS keychain (wallet/secretStore.ts), never in this
// database, never in plaintext, never logged. See wallet/hotWallet.ts.
export const walletMeta = sqliteTable("wallet_meta", {
  id: text("id").primaryKey(),
  pubkey: text("pubkey").notNull(),
  createdAt: text("created_at").notNull(),
});

// Single-row table (id = 'singleton') holding the global paper/live + devnet/mainnet
// switches. `tradingMode` is forcibly reset to 'paper' on every server boot (db/client.ts)
// regardless of what was persisted — live trading never silently resumes after a
// crash/restart, re-arming it is a deliberate action every time the process starts. Not
// per-strategy: flipping to live moves every active strategy's execution path together.
export const appMode = sqliteTable("app_mode", {
  id: text("id").primaryKey(),
  tradingMode: text("trading_mode").notNull().default("paper"), // 'paper' | 'live'
  network: text("network").notNull().default("devnet"), // 'devnet' | 'mainnet'
  updatedAt: text("updated_at").notNull(),
});

// One row per manual send from the bot's hot wallet (wallet/txBuilder.ts's sendTransfer,
// routes/wallet.ts's POST /api/wallet/send) — the app's own SIGNED transactions have
// nowhere else to live: unlike strategy trades, a manual send was never a Signal and
// doesn't belong in `trades`. Combined with `trades` (simulated=false rows) by
// GET /api/wallet/history to give one merged view of everything this wallet has actually
// done on-chain — sends AND live swap executions.
export const walletSends = sqliteTable("wallet_sends", {
  id: text("id").primaryKey(),
  tokenMint: text("token_mint").notNull(),
  tokenSymbol: text("token_symbol").notNull(),
  amount: real("amount").notNull(),
  destination: text("destination").notNull(),
  network: text("network").notNull(),
  txHash: text("tx_hash").notNull(),
  confirmationSlot: integer("confirmation_slot"),
  timestamp: text("timestamp").notNull(),
});

// Single-row table (id = 'singleton'), live trading's equivalent of portfolio_meta's
// realizedPnlUsd/startOfDayValueUsd/dayStartDate — but NOT cashUsd, since live has no
// virtual cash: real spendable balance is the wallet's actual on-chain USDC/SOL, queried
// live (see execution/liveExecutor.ts). Lazily created on first live-path use, not
// pre-seeded at boot — the "start of day" baseline should reflect real wallet value at
// first genuine use, not an arbitrary boot-time placeholder.
export const liveWalletMeta = sqliteTable("live_wallet_meta", {
  id: text("id").primaryKey(),
  realizedPnlUsd: real("realized_pnl_usd").notNull().default(0),
  startOfDayValueUsd: real("start_of_day_value_usd").notNull(),
  dayStartDate: text("day_start_date").notNull(),
  updatedAt: text("updated_at").notNull(),
});

// Single-row table (id = 'singleton'). Off by default (`enabled: false`) — this is a
// standing rule that moves real funds out of the wallet with no per-transfer
// confirmation once armed, so nothing here is ever pre-seeded with a real destination;
// the user must explicitly fill in and confirm every field via the Wallet tab (see
// execution/autoSweep.ts, routes/wallet.ts's PUT /api/wallet/auto-sweep). Applies to
// whichever network is currently active in app_mode — there's one wallet, one active
// cluster at a time, same as live trading.
export const autoSweepConfig = sqliteTable("auto_sweep_config", {
  id: text("id").primaryKey(),
  enabled: integer("enabled", { mode: "boolean" }).notNull().default(false),
  tokenMint: text("token_mint").notNull(),
  tokenSymbol: text("token_symbol").notNull(),
  /** Keep this much of `tokenMint` in the wallet; anything above it gets swept to
   * `destination` the next time the check runs. */
  thresholdAmount: real("threshold_amount").notNull(),
  destination: text("destination").notNull(),
  updatedAt: text("updated_at").notNull(),
});
