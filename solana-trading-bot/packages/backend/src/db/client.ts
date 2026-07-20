import Database from "better-sqlite3";
import { drizzle } from "drizzle-orm/better-sqlite3";
import { eq, sql } from "drizzle-orm";
import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import { config, riskDefaults } from "../config.js";
import * as schema from "./schema.js";

mkdirSync(dirname(config.databasePath), { recursive: true });

const sqlite = new Database(config.databasePath);
sqlite.pragma("journal_mode = WAL");

export const db = drizzle(sqlite, { schema });

// One-time migration: positions moved from one-row-per-token to one-row-per-strategy-
// config (see schema.ts) so concurrent strategies on the same token stop closing each
// other's positions. The old table has no strategy_config_id column, so any existing
// open position can't be honestly attributed to a specific strategy — rather than
// invent an owner, liquidate it back to cash at its last known entry price (a
// conservative valuation; no live price feed is running yet at this point in startup)
// and log it clearly. Computed before the CREATE TABLE block below (which would
// otherwise leave an old-schema table in place, since IF NOT EXISTS is a no-op when a
// table already exists under that name).
let positionsLiquidationCreditUsd = 0;
const positionsTableExists = sqlite.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='positions'").get();
if (positionsTableExists) {
  const positionColumns = sqlite.prepare("PRAGMA table_info(positions)").all() as { name: string }[];
  const hasOldSchema = !positionColumns.some((col) => col.name === "strategy_config_id");
  if (hasOldSchema) {
    const oldPositions = sqlite.prepare("SELECT token_mint, token_symbol, quantity, avg_entry_price_usd FROM positions").all() as {
      token_mint: string;
      token_symbol: string;
      quantity: number;
      avg_entry_price_usd: number;
    }[];
    for (const pos of oldPositions) {
      if (pos.quantity <= 1e-9) continue;
      const valueUsd = pos.quantity * pos.avg_entry_price_usd;
      positionsLiquidationCreditUsd += valueUsd;
      console.warn(
        `[db] migrating to per-strategy position ledgers: existing ${pos.quantity.toFixed(6)} ${pos.token_symbol} ` +
          `(~$${valueUsd.toFixed(2)} at last entry price) could not be attributed to a specific strategy and was ` +
          `liquidated back to cash instead of guessing an owner.`,
      );
    }
    sqlite.exec("DROP TABLE positions");
  }
}

// Second, independent migration on the same table: positions gained a composite
// (strategy_config_id, simulated) primary key (see schema.ts) so a config's paper and
// live positions can coexist as separate rows instead of one shared row. SQLite can't
// ALTER a primary key, so this is drop-and-recreate too — every existing row is
// unambiguously simulated=1 (paper), since live trading didn't exist before this. Runs
// after the migration above (which may already have dropped/recreated this table for an
// unrelated reason) and before the CREATE TABLE block below defines the new schema.
let positionsToReinsert: {
  strategy_config_id: string;
  token_mint: string;
  token_symbol: string;
  quantity: number;
  avg_entry_price_usd: number;
  stop_loss_price_usd: number | null;
}[] = [];
const positionsTableStillExists = sqlite.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='positions'").get();
if (positionsTableStillExists) {
  const positionColumnsNow = sqlite.prepare("PRAGMA table_info(positions)").all() as { name: string }[];
  if (!positionColumnsNow.some((col) => col.name === "simulated")) {
    positionsToReinsert = sqlite
      .prepare("SELECT strategy_config_id, token_mint, token_symbol, quantity, avg_entry_price_usd, stop_loss_price_usd FROM positions")
      .all() as typeof positionsToReinsert;
    sqlite.exec("DROP TABLE positions");
  }
}

sqlite.exec(`
  CREATE TABLE IF NOT EXISTS strategy_configs (
    id TEXT PRIMARY KEY,
    strategy_id TEXT NOT NULL,
    token_mint TEXT NOT NULL,
    token_symbol TEXT NOT NULL,
    params TEXT NOT NULL,
    watched_wallet_address TEXT,
    active INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
  );

  CREATE TABLE IF NOT EXISTS trades (
    id TEXT PRIMARY KEY,
    strategy_config_id TEXT NOT NULL,
    strategy_id TEXT NOT NULL,
    action TEXT NOT NULL,
    token_mint TEXT NOT NULL,
    token_symbol TEXT NOT NULL,
    price_usd REAL NOT NULL,
    size_usd REAL NOT NULL,
    size_token REAL NOT NULL,
    fee_usd REAL NOT NULL DEFAULT 0,
    reason TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    simulated INTEGER NOT NULL DEFAULT 1,
    tx_hash TEXT,
    network TEXT,
    confirmation_slot INTEGER
  );

  CREATE TABLE IF NOT EXISTS positions (
    strategy_config_id TEXT NOT NULL,
    simulated INTEGER NOT NULL DEFAULT 1,
    token_mint TEXT NOT NULL,
    token_symbol TEXT NOT NULL,
    quantity REAL NOT NULL DEFAULT 0,
    avg_entry_price_usd REAL NOT NULL DEFAULT 0,
    stop_loss_price_usd REAL,
    PRIMARY KEY (strategy_config_id, simulated)
  );

  CREATE TABLE IF NOT EXISTS portfolio_meta (
    id TEXT PRIMARY KEY,
    cash_usd REAL NOT NULL,
    realized_pnl_usd REAL NOT NULL DEFAULT 0,
    start_of_day_value_usd REAL NOT NULL,
    day_start_date TEXT NOT NULL,
    paused INTEGER NOT NULL DEFAULT 0
  );

  CREATE TABLE IF NOT EXISTS risk_settings (
    id TEXT PRIMARY KEY,
    max_position_pct REAL NOT NULL,
    max_daily_loss_pct REAL NOT NULL,
    per_token_exposure_pct REAL NOT NULL,
    default_stop_loss_pct REAL NOT NULL,
    max_slippage_bps REAL NOT NULL,
    max_price_impact_pct REAL NOT NULL
  );

  CREATE TABLE IF NOT EXISTS wallet_meta (
    id TEXT PRIMARY KEY,
    pubkey TEXT NOT NULL,
    created_at TEXT NOT NULL
  );

  CREATE TABLE IF NOT EXISTS app_mode (
    id TEXT PRIMARY KEY,
    trading_mode TEXT NOT NULL DEFAULT 'paper',
    network TEXT NOT NULL DEFAULT 'devnet',
    updated_at TEXT NOT NULL
  );

  CREATE TABLE IF NOT EXISTS live_wallet_meta (
    id TEXT PRIMARY KEY,
    realized_pnl_usd REAL NOT NULL DEFAULT 0,
    start_of_day_value_usd REAL NOT NULL,
    day_start_date TEXT NOT NULL,
    updated_at TEXT NOT NULL
  );

  CREATE TABLE IF NOT EXISTS wallet_sends (
    id TEXT PRIMARY KEY,
    token_mint TEXT NOT NULL,
    token_symbol TEXT NOT NULL,
    amount REAL NOT NULL,
    destination TEXT NOT NULL,
    network TEXT NOT NULL,
    tx_hash TEXT NOT NULL,
    confirmation_slot INTEGER,
    timestamp TEXT NOT NULL
  );

  CREATE TABLE IF NOT EXISTS auto_sweep_config (
    id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 0,
    token_mint TEXT NOT NULL,
    token_symbol TEXT NOT NULL,
    threshold_amount REAL NOT NULL,
    destination TEXT NOT NULL,
    updated_at TEXT NOT NULL
  );
`);

// strategy_configs may already exist from before watched_wallet_address was added
// (CREATE TABLE IF NOT EXISTS above won't retrofit existing tables). SQLite has no
// "ADD COLUMN IF NOT EXISTS", so check first.
const strategyConfigColumns = sqlite.prepare("PRAGMA table_info(strategy_configs)").all() as { name: string }[];
if (!strategyConfigColumns.some((col) => col.name === "watched_wallet_address")) {
  sqlite.exec("ALTER TABLE strategy_configs ADD COLUMN watched_wallet_address TEXT");
}

// trades may already exist from before fee_usd was added (see config.ts tradingCosts).
// Existing rows backfill to 0 — they were genuinely fee-free at execution time, this
// isn't retroactively rewriting what happened, only new fills are charged going forward.
const tradeColumns = sqlite.prepare("PRAGMA table_info(trades)").all() as { name: string }[];
if (!tradeColumns.some((col) => col.name === "fee_usd")) {
  sqlite.exec("ALTER TABLE trades ADD COLUMN fee_usd REAL NOT NULL DEFAULT 0");
}

// trades may also predate simulated/tx_hash/network/confirmation_slot (added for live
// trading — see execution/liveExecutor.ts). Existing rows backfill to simulated=1 (they
// genuinely were paper trades) with the rest null (they never touched a network).
if (!tradeColumns.some((col) => col.name === "simulated")) {
  sqlite.exec("ALTER TABLE trades ADD COLUMN simulated INTEGER NOT NULL DEFAULT 1");
  sqlite.exec("ALTER TABLE trades ADD COLUMN tx_hash TEXT");
  sqlite.exec("ALTER TABLE trades ADD COLUMN network TEXT");
  sqlite.exec("ALTER TABLE trades ADD COLUMN confirmation_slot INTEGER");
}

// Carries forward any positions captured before the composite-PK migration above
// (DROP TABLE positions) recreated the table with the new schema.
if (positionsToReinsert.length > 0) {
  const insertPosition = sqlite.prepare(
    "INSERT INTO positions (strategy_config_id, simulated, token_mint, token_symbol, quantity, avg_entry_price_usd, stop_loss_price_usd) VALUES (?, 1, ?, ?, ?, ?, ?)",
  );
  for (const pos of positionsToReinsert) {
    insertPosition.run(pos.strategy_config_id, pos.token_mint, pos.token_symbol, pos.quantity, pos.avg_entry_price_usd, pos.stop_loss_price_usd);
  }
  console.log(`[db] carried forward ${positionsToReinsert.length} existing position(s) as simulated=1 into the new composite-PK positions table`);
}

// One-time migration: "range-scalper-5m" was renamed to "range-scalper" when its window
// became variable (1-15 min) and its logic gained edge filters. Upgrade any existing
// configs in place so they keep trading instead of being silently orphaned. Params are
// re-seeded from the new strategy's defaults, carrying over the settings that still
// exist (windowMinutes, buyZonePct, positionSizeUsd); sellZonePct/takeProfitPct were
// replaced by targetRangePct/stopLossPct and are dropped. Defaults are duplicated here
// rather than imported from the registry to avoid a circular import (registry ->
// whaleCopy -> whaleWatcher -> db/client).
const legacyScalperConfigs = sqlite
  .prepare("SELECT id, params FROM strategy_configs WHERE strategy_id = 'range-scalper-5m'")
  .all() as { id: string; params: string }[];
for (const legacy of legacyScalperConfigs) {
  let oldParams: Record<string, number> = {};
  try {
    oldParams = JSON.parse(legacy.params) as Record<string, number>;
  } catch {
    // fall through with empty oldParams; defaults below take over
  }
  const newParams = {
    windowMinutes: oldParams.windowMinutes ?? 5,
    positionSizeUsd: oldParams.positionSizeUsd ?? 100,
    buyZonePct: oldParams.buyZonePct ?? 25,
    targetRangePct: 65,
    stopLossPct: 0.5,
    minRangePct: 0.3,
    maxTrendEfficiency: 0.35,
    maxHoldMinutes: 10,
  };
  sqlite
    .prepare("UPDATE strategy_configs SET strategy_id = 'range-scalper', params = ? WHERE id = ?")
    .run(JSON.stringify(newParams), legacy.id);
  console.log(`[db] migrated strategy config ${legacy.id} from range-scalper-5m to range-scalper`);
}

const existingMeta = db.select().from(schema.portfolioMeta).where(eq(schema.portfolioMeta.id, "singleton")).get();

if (!existingMeta) {
  db.insert(schema.portfolioMeta)
    .values({
      id: "singleton",
      cashUsd: config.simulatedStartingCashUsd,
      realizedPnlUsd: 0,
      startOfDayValueUsd: config.simulatedStartingCashUsd,
      dayStartDate: new Date().toISOString().slice(0, 10),
      paused: false,
    })
    .run();
}

if (positionsLiquidationCreditUsd > 0) {
  db.update(schema.portfolioMeta)
    .set({ cashUsd: sql`${schema.portfolioMeta.cashUsd} + ${positionsLiquidationCreditUsd}` })
    .where(eq(schema.portfolioMeta.id, "singleton"))
    .run();
  console.log(`[db] credited $${positionsLiquidationCreditUsd.toFixed(2)} back to cash from the position-ledger migration`);
}

const existingRiskSettings = db.select().from(schema.riskSettings).where(eq(schema.riskSettings.id, "singleton")).get();

if (!existingRiskSettings) {
  db.insert(schema.riskSettings)
    .values({ id: "singleton", ...riskDefaults })
    .run();
}

// app_mode is forcibly reset to Paper/Devnet on EVERY boot, regardless of what was
// persisted at last shutdown — live trading must never silently resume after a
// crash/restart (see execution/liveExecutor.ts / docs/ARCHITECTURE.md). Paper and Live
// are now a single derived pair, not two independent fields (network =
// tradingMode==='live' ? mainnet : devnet — see execution/appMode.ts), so resetting
// tradingMode to 'paper' now also means resetting network to 'devnet', full stop. This
// also one-time-corrects any row persisted before that invariant existed: tradingMode
// and network used to be independently settable, so an old row could legitimately be
// paper+mainnet — a combination that's structurally impossible to create anymore, but a
// stale existing row still needs fixing here rather than being left to quietly mismatch
// what the rest of the app now assumes.
const existingAppMode = db.select().from(schema.appMode).where(eq(schema.appMode.id, "singleton")).get();
const nowIso = new Date().toISOString();
if (!existingAppMode) {
  db.insert(schema.appMode).values({ id: "singleton", tradingMode: "paper", network: "devnet", updatedAt: nowIso }).run();
} else if (existingAppMode.tradingMode !== "paper" || existingAppMode.network !== "devnet") {
  db.update(schema.appMode).set({ tradingMode: "paper", network: "devnet", updatedAt: nowIso }).where(eq(schema.appMode.id, "singleton")).run();
  console.log(
    `[db] reset trading mode to paper/devnet on boot (was '${existingAppMode.tradingMode}'/'${existingAppMode.network}') — live trading never auto-resumes after a restart`,
  );
}
