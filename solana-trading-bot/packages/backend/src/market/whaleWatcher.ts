import { Connection, PublicKey } from "@solana/web3.js";
import { and, eq } from "drizzle-orm";
import { config } from "../config.js";
import { db } from "../db/client.js";
import { strategyConfigs } from "../db/schema.js";

// Read-only: this module only ever calls getSignaturesForAddress / getParsedTransaction
// against mainnet to OBSERVE a watched wallet's public transaction history. It never
// holds a keypair and never signs or sends anything — including on the watched
// wallet's behalf, which it has no ability to do.
//
// Public mainnet-beta RPC has tight rate limits and getParsedTransaction is expensive;
// this polls conservatively (every POLL_INTERVAL_MS, capped signatures per check) and
// swallows/logs failures rather than crashing. For real use, point
// SOLANA_MAINNET_RPC_URL at Helius/QuickNode/Triton (see .env.example).

export interface WhaleTransfer {
  signature: string;
  blockTime: Date;
  tokenMint: string;
  direction: "buy" | "sell";
  tokenAmount: number;
}

const connection = new Connection(config.solanaMainnetRpcUrl, "confirmed");
const MAX_SIGNATURES_TO_SCAN = 5;
const POLL_INTERVAL_MS = 30_000;

const pendingByConfigId = new Map<string, WhaleTransfer[]>();
const lastSeenSignature = new Map<string, string>();
let timer: ReturnType<typeof setInterval> | null = null;

export function start(): void {
  if (timer) return;
  void pollAll();
  timer = setInterval(() => void pollAll(), POLL_INTERVAL_MS);
}

export function stop(): void {
  if (timer) clearInterval(timer);
  timer = null;
}

/** Called synchronously by WhaleCopyStrategy on each price tick; returns and clears any
 * transfers detected for this strategy config since the last drain. Keeps the
 * StrategyBase.onInterval contract synchronous even though the underlying watch is async. */
export function drainPending(strategyConfigId: string): WhaleTransfer[] {
  const pending = pendingByConfigId.get(strategyConfigId) ?? [];
  pendingByConfigId.set(strategyConfigId, []);
  return pending;
}

async function pollAll(): Promise<void> {
  const activeWhaleConfigs = db
    .select()
    .from(strategyConfigs)
    .where(and(eq(strategyConfigs.strategyId, "whale-copy"), eq(strategyConfigs.active, true)))
    .all();

  for (const cfg of activeWhaleConfigs) {
    if (!cfg.watchedWalletAddress) continue;
    try {
      const transfers = await fetchNewTransfers(cfg.id, cfg.watchedWalletAddress, cfg.tokenMint);
      if (transfers.length === 0) continue;
      const existing = pendingByConfigId.get(cfg.id) ?? [];
      pendingByConfigId.set(cfg.id, [...existing, ...transfers]);
    } catch (err) {
      console.warn(`[whaleWatcher] poll failed for ${cfg.watchedWalletAddress}:`, err instanceof Error ? err.message : err);
    }
  }
}

async function fetchNewTransfers(strategyConfigId: string, walletAddress: string, tokenMint: string): Promise<WhaleTransfer[]> {
  let owner: PublicKey;
  try {
    owner = new PublicKey(walletAddress);
  } catch {
    return [];
  }

  const signatures = await connection.getSignaturesForAddress(owner, { limit: MAX_SIGNATURES_TO_SCAN });
  const lastSeen = lastSeenSignature.get(strategyConfigId);
  const transfers: WhaleTransfer[] = [];
  let newestSignature: string | undefined;

  for (const sigInfo of signatures) {
    if (sigInfo.signature === lastSeen) break; // reached already-processed history
    if (!newestSignature) newestSignature = sigInfo.signature;
    if (sigInfo.err || !sigInfo.blockTime) continue;

    const tx = await connection.getParsedTransaction(sigInfo.signature, { maxSupportedTransactionVersion: 0 });
    if (!tx?.meta) continue;

    const pre = tx.meta.preTokenBalances ?? [];
    const post = tx.meta.postTokenBalances ?? [];

    for (const postBalance of post) {
      if (postBalance.owner !== walletAddress || postBalance.mint !== tokenMint) continue;
      const preBalance = pre.find((p) => p.accountIndex === postBalance.accountIndex);
      const preAmount = preBalance?.uiTokenAmount.uiAmount ?? 0;
      const postAmount = postBalance.uiTokenAmount.uiAmount ?? 0;
      const delta = postAmount - preAmount;
      if (Math.abs(delta) < 1e-9) continue;

      transfers.push({
        signature: sigInfo.signature,
        blockTime: new Date(sigInfo.blockTime * 1000),
        tokenMint: postBalance.mint,
        direction: delta > 0 ? "buy" : "sell",
        tokenAmount: Math.abs(delta),
      });
    }
  }

  if (newestSignature) lastSeenSignature.set(strategyConfigId, newestSignature);
  return transfers;
}
