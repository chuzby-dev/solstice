import { randomUUID } from "node:crypto";
import { eq } from "drizzle-orm";
import { PublicKey } from "@solana/web3.js";
import type { WalletTransaction } from "@trading-bot/shared";
import { db } from "../db/client.js";
import { autoSweepConfig, walletSends } from "../db/schema.js";
import { getHotWalletPublicKey } from "../wallet/hotWallet.js";
import { sendTransfer, SOL_MINT } from "../wallet/txBuilder.js";
import { fetchWalletBalances } from "../wallet/walletBalance.js";
import { broadcast } from "../ws/hub.js";
import { getAppMode } from "./appMode.js";

// A standing rule that moves real funds with NO per-transfer confirmation once armed (see
// db/schema.ts's autoSweepConfig comment and routes/wallet.ts's PUT /api/wallet/auto-sweep
// for the enable-time guards) — called from strategy-engine/engine.ts's tick loop, but
// throttled internally so it doesn't hammer the RPC on every ~2s price tick. Mirrors
// liveExecutor.ts's fire-and-forget-from-a-sync-loop shape: engine.ts calls this without
// awaiting, and `inFlight` here (not a per-config Set like liveExecutor's — there's only
// ever one sweep rule) stops two overlapping checks from racing the same slow RPC round
// trip if a check is still pending when the next tick's throttle window has already
// elapsed.
const CHECK_INTERVAL_MS = 60_000;
let lastCheckedAt = 0;
let inFlight = false;

export async function checkAndRunAutoSweep(): Promise<void> {
  const now = Date.now();
  if (now - lastCheckedAt < CHECK_INTERVAL_MS || inFlight) return;
  lastCheckedAt = now;

  const sweepConfig = db.select().from(autoSweepConfig).where(eq(autoSweepConfig.id, "singleton")).get();
  if (!sweepConfig?.enabled) return;

  const pubkey = getHotWalletPublicKey();
  if (!pubkey) return;

  inFlight = true;
  try {
    const { network } = getAppMode();
    const { solBalance, tokenBalances } = await fetchWalletBalances(network, new PublicKey(pubkey));
    const currentAmount = sweepConfig.tokenMint === SOL_MINT ? solBalance : (tokenBalances.find((t) => t.mint === sweepConfig.tokenMint)?.amount ?? 0);

    const excess = currentAmount - sweepConfig.thresholdAmount;
    if (excess <= 0) return;

    // sendTransfer's own validateSolSend/validateSplSend re-check reserve/balance right
    // before signing (see wallet/txBuilder.ts) — this is defense-in-depth, not the only
    // guard. For a SOL sweep specifically, thresholdAmount effectively doubles as the fee
    // reserve: setting it below walletSend.minSolReserve just means every sweep attempt
    // is safely rejected by that check instead of draining the wallet's gas.
    const result = await sendTransfer({ tokenMint: sweepConfig.tokenMint, amount: excess, destination: sweepConfig.destination, network });

    const record: WalletTransaction = {
      id: randomUUID(),
      kind: "send",
      tokenSymbol: sweepConfig.tokenSymbol,
      amount: excess,
      network,
      txHash: result.txHash,
      confirmationSlot: result.confirmationSlot,
      timestamp: new Date().toISOString(),
      destination: sweepConfig.destination,
    };
    db.insert(walletSends)
      .values({
        id: record.id,
        tokenMint: sweepConfig.tokenMint,
        tokenSymbol: sweepConfig.tokenSymbol,
        amount: excess,
        destination: sweepConfig.destination,
        network,
        txHash: result.txHash,
        confirmationSlot: result.confirmationSlot,
        timestamp: record.timestamp,
      })
      .run();
    broadcast({ type: "wallet_send", data: record });

    console.log(`[autoSweep] swept ${excess.toFixed(6)} ${sweepConfig.tokenSymbol} to ${sweepConfig.destination} on ${network} (tx=${result.txHash})`);
  } catch (err) {
    console.error("[autoSweep] check/sweep failed:", err);
  } finally {
    inFlight = false;
  }
}
