import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import dotenv from "dotenv";

// Plain `import "dotenv/config"` loads `.env` from process.cwd(), which is
// packages/backend when this app runs as an npm workspace script (`npm run dev`/`start`)
// — but the repo's .env lives at the monorepo root, one level up. Every setting used to
// have a fallback that happened to match .env's shipped value, so this went unnoticed
// until BIRDEYE_API_KEY (backtest/birdeyeClient.ts, no fallback) needed a real value the
// live server actually has to read. Resolve the root .env explicitly instead of relying
// on cwd.
dotenv.config({ path: join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", ".env") });

function requireEnv(name: string, fallback?: string): string {
  const value = process.env[name] ?? fallback;
  if (value === undefined) {
    throw new Error(`Missing required environment variable: ${name}`);
  }
  return value;
}

export const config = {
  port: Number(requireEnv("PORT", "4000")),
  databasePath: requireEnv("DATABASE_PATH", "./data/trading-bot.sqlite"),
  solanaDevnetRpcUrl: requireEnv("SOLANA_DEVNET_RPC_URL", "https://api.devnet.solana.com"),
  /** Used only by whale-copy's read-only transaction watcher. Public mainnet-beta is
   * rate-limited; swap in Helius/QuickNode/Triton for reliable whale-copy behavior. */
  solanaMainnetRpcUrl: requireEnv("SOLANA_MAINNET_RPC_URL", "https://api.mainnet-beta.solana.com"),
  priceApiUrl: requireEnv("PRICE_API_URL", "https://hermes.pyth.network/v2/updates/price/latest"),
  pricePollIntervalMs: Number(requireEnv("PRICE_POLL_INTERVAL_MS", "2000")),
  tokenAllowlist: requireEnv(
    "TOKEN_ALLOWLIST",
    "So11111111111111111111111111111111111111,EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
  )
    .split(",")
    .map((mint) => mint.trim())
    .filter(Boolean),
  /** Starting virtual cash balance for the paper-trading ledger. Not a real balance. */
  simulatedStartingCashUsd: 10_000,
} as const;

/** Estimated real round-trip cost of a Jupiter swap on Solana, applied to EVERY
 * simulated fill (see execution/simulator.ts). Added specifically because paper P&L
 * with zero fees is misleading right before real funds go into this account — a
 * strategy that looks profitable against a fee-free simulator can be a guaranteed
 * loser against real costs. Every strategy's paper results now reflect this, not just
 * fee-aware ones.
 *   - solanaTxFeeUsd: the base network fee (~5000 lamports), effectively negligible.
 *   - priorityFeeUsd: a conservative mid-congestion estimate for landing a swap
 *     promptly; real priority fees vary with network conditions and can be much
 *     higher during congestion or much lower when the network is quiet.
 *   - swapFeeBps / slippageBufferBps: per-leg estimate of DEX pool fee + realistic
 *     slippage for a small retail-sized SOL/USDC trade routed through Jupiter. Actual
 *     cost depends on the specific route Jupiter picks; this is a reasonable
 *     middle-of-the-road planning estimate, not a live quote.
 * All four are configurable via env for whoever eventually wires in live execution and
 * wants to calibrate against real observed costs instead of this estimate. */
export const tradingCosts = {
  solanaTxFeeUsd: Number(requireEnv("SOLANA_TX_FEE_USD", "0.0005")),
  priorityFeeUsd: Number(requireEnv("PRIORITY_FEE_USD", "0.005")),
  swapFeeBps: Number(requireEnv("SWAP_FEE_BPS", "10")),
  slippageBufferBps: Number(requireEnv("SLIPPAGE_BUFFER_BPS", "5")),
} as const;

/** One-way (single buy OR sell leg) fee estimate in USD for a trade of `sizeUsd`. */
export function estimateTradeFeeUsd(sizeUsd: number): number {
  const fixedUsd = tradingCosts.solanaTxFeeUsd + tradingCosts.priorityFeeUsd;
  const percentageUsd = (sizeUsd * (tradingCosts.swapFeeBps + tradingCosts.slippageBufferBps)) / 10_000;
  return fixedUsd + percentageUsd;
}

/** Non-negotiable risk defaults (spec section 7). Overridable per-request within safe bounds
 * via the Settings API, but the hard ceilings below are enforced regardless of user input. */
export const riskDefaults = {
  maxPositionPct: 10, // % of total portfolio value per trade
  maxDailyLossPct: 5, // % of start-of-day value; breach auto-pauses the engine
  perTokenExposurePct: 25, // % of total portfolio value in any single token
  defaultStopLossPct: 8, // mandatory stop-loss applied to every position
  maxSlippageBps: 100, // 1%
  maxPriceImpactPct: 3,
} as const;

export const riskHardCeilings = {
  maxPositionPct: 25,
  maxDailyLossPct: 15,
  perTokenExposurePct: 50,
  maxSlippageBps: 300,
  maxPriceImpactPct: 8,
} as const;

/** Manual wallet sends (wallet/txBuilder.ts, routes/wallet.ts) are user-directed transfers
 * to an arbitrary address, not strategy signals — they deliberately bypass
 * riskManager.evaluateSignal() (wrong shape: no position sizing, no per-token exposure,
 * this is a one-off transfer) and get this much simpler, independent ceiling instead. */
export const walletSend = {
  /** Sends at or below this USD-equivalent value need only the standard one-step confirm.
   * Above it, the UI requires an explicit extra "this is unusually large" acknowledgement
   * before the button to actually send is enabled. */
  maxWithoutExtraConfirmUsd: Number(requireEnv("MAX_SEND_WITHOUT_EXTRA_CONFIRM_USD", "50")),
  /** Every send leaves at least this much SOL behind, regardless of what's being sent —
   * fees are always paid in SOL, so a send that would drain below this reserve is
   * rejected before it's even built. */
  minSolReserve: Number(requireEnv("MIN_SOL_RESERVE_FOR_FEES", "0.01")),
  /** How long to wait for on-chain confirmation before giving up and reporting the send
   * as unresolved (not failed — it may still land late; see docs/ARCHITECTURE.md). */
  confirmationTimeoutMs: Number(requireEnv("SEND_CONFIRMATION_TIMEOUT_MS", "60000")),
} as const;
