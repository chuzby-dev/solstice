import { Connection, PublicKey, SystemProgram, Transaction, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID, getAssociatedTokenAddress, createAssociatedTokenAccountInstruction, createTransferInstruction, getAccount, getMint } from "@solana/spl-token";
import type { Network, TransferPreview } from "@trading-bot/shared";
import { config, walletSend } from "../config.js";
import { getHotWalletPublicKey, signLegacyTransaction } from "./hotWallet.js";

// Manual send/transfer pipeline for the bot's own hot wallet (wallet/hotWallet.ts).
// Network-parameterized (defaulting to devnet, matching every existing caller — the
// manual Send form in the Wallet tab is still devnet-only by product choice, not a
// technical limit) so wallet/autoSweep.ts can reuse this exact build->sign->submit->
// confirm pipeline for real mainnet transfers instead of a second copy.
const devnetConnection = new Connection(config.solanaDevnetRpcUrl, "confirmed");
const mainnetConnection = new Connection(config.solanaMainnetRpcUrl, "confirmed");

function connectionForNetwork(network: Network): Connection {
  return network === "mainnet" ? mainnetConnection : devnetConnection;
}

// The app's own native-SOL sentinel, matching the SAME (slightly-off — see
// backtest/birdeyeClient.ts's header comment) constant used everywhere else in this
// codebase (TOKEN_ALLOWLIST, priceFeed.ts). Only ever used here for comparison, never
// passed into an SPL-token function as if it were a real mint account — SOL transfers use
// SystemProgram, which has no mint at all.
export const SOL_MINT = "So11111111111111111111111111111111111111";

export class InvalidDestinationError extends Error {
  constructor(destination: string) {
    super(`"${destination}" is not a valid Solana address`);
    this.name = "InvalidDestinationError";
  }
}

export class InsufficientBalanceError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "InsufficientBalanceError";
  }
}

export interface SendResult {
  txHash: string;
  confirmationSlot: number | null;
}

/** Pure — no RPC. Throws `InvalidDestinationError` rather than letting `PublicKey`'s own
 * exception (whose message/type isn't ours to guarantee) leak past this module. */
export function parseDestination(destination: string): PublicKey {
  try {
    return new PublicKey(destination);
  } catch {
    throw new InvalidDestinationError(destination);
  }
}

/** Pure — no RPC. A send that would leave the wallet below `walletSend.minSolReserve` is
 * rejected before anything is built, regardless of what's being sent (fees are always
 * SOL-denominated). For a SOL send, the reserve check already covers the fee itself, since
 * it's tiny relative to the reserve's own margin. */
export function validateSolSend(balanceSol: number, amountSol: number): void {
  if (amountSol <= 0) throw new InsufficientBalanceError("Amount must be greater than zero");
  if (amountSol + walletSend.minSolReserve > balanceSol) {
    throw new InsufficientBalanceError(
      `Sending ${amountSol} SOL would leave less than the required ${walletSend.minSolReserve} SOL reserve for fees (balance: ${balanceSol} SOL)`,
    );
  }
}

/** Pure — no RPC. */
export function validateSplSend(tokenBalance: number, amountTokens: number, solBalanceForFees: number): void {
  if (amountTokens <= 0) throw new InsufficientBalanceError("Amount must be greater than zero");
  if (amountTokens > tokenBalance) {
    throw new InsufficientBalanceError(`Insufficient token balance: requested ${amountTokens}, have ${tokenBalance}`);
  }
  if (solBalanceForFees < walletSend.minSolReserve) {
    throw new InsufficientBalanceError(`Insufficient SOL for fees: need at least ${walletSend.minSolReserve} SOL reserved, have ${solBalanceForFees}`);
  }
}

function hotWalletPubkeyOrThrow(): PublicKey {
  const pubkey = getHotWalletPublicKey();
  if (!pubkey) throw new Error("No hot wallet has been created yet");
  return new PublicKey(pubkey);
}

async function buildSolTransfer(conn: Connection, from: PublicKey, to: PublicKey, amountSol: number): Promise<Transaction> {
  const balanceLamports = await conn.getBalance(from);
  validateSolSend(balanceLamports / LAMPORTS_PER_SOL, amountSol);

  const tx = new Transaction();
  tx.add(SystemProgram.transfer({ fromPubkey: from, toPubkey: to, lamports: Math.round(amountSol * LAMPORTS_PER_SOL) }));
  return tx;
}

async function buildSplTransfer(conn: Connection, from: PublicKey, to: PublicKey, mint: PublicKey, amountTokens: number): Promise<Transaction> {
  const [mintInfo, sourceAta, solBalanceLamports] = await Promise.all([getMint(conn, mint), getAssociatedTokenAddress(mint, from), conn.getBalance(from)]);

  const sourceAccount = await getAccount(conn, sourceAta);
  const tokenBalance = Number(sourceAccount.amount) / 10 ** mintInfo.decimals;
  validateSplSend(tokenBalance, amountTokens, solBalanceLamports / LAMPORTS_PER_SOL);

  const destAta = await getAssociatedTokenAddress(mint, to);
  const tx = new Transaction();

  const destAtaInfo = await conn.getAccountInfo(destAta);
  if (!destAtaInfo) {
    // Destination has no token account for this mint yet — create it in the same
    // transaction (paid for by the sender, standard convention). Extra rent cost, worth
    // surfacing to the user before they confirm (see routes/wallet.ts's estimate step).
    tx.add(createAssociatedTokenAccountInstruction(from, destAta, to, mint));
  }

  const rawAmount = BigInt(Math.round(amountTokens * 10 ** mintInfo.decimals));
  tx.add(createTransferInstruction(sourceAta, destAta, from, rawAmount));
  return tx;
}

async function confirmWithTimeout(conn: Connection, signature: string, blockhash: string, lastValidBlockHeight: number): Promise<number | null> {
  const timeout = new Promise<"timeout">((resolve) => setTimeout(() => resolve("timeout"), walletSend.confirmationTimeoutMs));
  const confirmation = conn.confirmTransaction({ signature, blockhash, lastValidBlockHeight }, "confirmed");

  const result = await Promise.race([confirmation, timeout]);
  if (result === "timeout") {
    // Not necessarily failed — it may still land late. Caller gets the signature back
    // either way so the user can look it up; we just can't say "confirmed" yet. See
    // docs/ARCHITECTURE.md's failure-mode table: never blind-resend on a timeout, that's
    // a double-send risk if the original was merely slow, not dropped.
    return null;
  }
  if (result.value.err) {
    throw new Error(`Transaction failed on-chain: ${JSON.stringify(result.value.err)}`);
  }

  const status = await conn.getSignatureStatuses([signature]);
  return status.value[0]?.slot ?? null;
}

/** Builds, signs (via hotWallet — the private key never leaves that module), submits, and
 * confirms a transfer from the bot's hot wallet. `tokenMint === SOL_MINT` sends native SOL
 * via SystemProgram; any other allowlisted mint sends as an SPL token transfer, creating
 * the destination's associated token account first if it doesn't exist yet. `network`
 * defaults to devnet only as a safety fallback if a caller forgets to pass it — every real
 * caller (the manual Send form and wallet/autoSweep.ts) passes the currently-active
 * network explicitly, since a send must reach whichever cluster the wallet is actually
 * holding real funds on. Was silently devnet-only until routes/wallet.ts started passing
 * `network` — see that file's history if you're wondering why a "real" mainnet send used
 * to land on devnet instead. */
export async function sendTransfer(params: { tokenMint: string; amount: number; destination: string; network?: Network }): Promise<SendResult> {
  const conn = connectionForNetwork(params.network ?? "devnet");
  const from = hotWalletPubkeyOrThrow();
  const to = parseDestination(params.destination);

  const tx =
    params.tokenMint === SOL_MINT ? await buildSolTransfer(conn, from, to, params.amount) : await buildSplTransfer(conn, from, to, new PublicKey(params.tokenMint), params.amount);

  // Fetch a fresh blockhash immediately before signing — never reuse one from earlier in
  // this request, even a few seconds old (see docs/ARCHITECTURE.md's failure-mode table).
  const { blockhash, lastValidBlockHeight } = await conn.getLatestBlockhash("confirmed");
  tx.recentBlockhash = blockhash;
  tx.feePayer = from;
  signLegacyTransaction(tx);

  const txHash = await conn.sendRawTransaction(tx.serialize());
  const confirmationSlot = await confirmWithTimeout(conn, txHash, blockhash, lastValidBlockHeight);

  return { txHash, confirmationSlot };
}

/** Builds the exact same transaction `sendTransfer` would, but stops before signing and
 * submitting — instead asks the RPC for a real fee quote (`getFeeForMessage`) and runs a
 * real dry-run simulation (`simulateTransaction`) against current on-chain state. This is
 * the "review" step of a Solflare/Phantom-style send flow: surfaces the real fee and any
 * reason the transaction would actually fail (bad destination token account, insufficient
 * balance discovered mid-build, etc.) BEFORE the user commits to an irreversible send.
 * Deliberately unsigned — a preview has no reason to touch the hot wallet's key at all,
 * and an unsigned transaction with `sigVerify` left at its default simulates exactly as
 * accurately as a signed one would (the message content, not the signature, is what's
 * executed against state). */
export async function previewTransfer(params: { tokenMint: string; amount: number; destination: string; network?: Network }): Promise<TransferPreview> {
  const conn = connectionForNetwork(params.network ?? "devnet");
  const from = hotWalletPubkeyOrThrow();
  const to = parseDestination(params.destination);

  const tx =
    params.tokenMint === SOL_MINT ? await buildSolTransfer(conn, from, to, params.amount) : await buildSplTransfer(conn, from, to, new PublicKey(params.tokenMint), params.amount);

  const { blockhash } = await conn.getLatestBlockhash("confirmed");
  tx.recentBlockhash = blockhash;
  tx.feePayer = from;

  const feeResult = await conn.getFeeForMessage(tx.compileMessage(), "confirmed");
  const estimatedFeeSol = (feeResult.value ?? 5000) / LAMPORTS_PER_SOL; // 5000 lamports: the standard base fee, used only if the RPC can't quote (rare)

  const simulation = await conn.simulateTransaction(tx);
  const simulationError = simulation.value.err ? `Simulation failed: ${JSON.stringify(simulation.value.err)}` : null;

  return { estimatedFeeSol, simulationError, logs: simulation.value.logs ?? [] };
}

// Trivial, non-zero so it's a real transfer instruction rather than a degenerate one some
// validators/tools might special-case — the exact amount is irrelevant since it returns to
// the same account (only the fee is actually spent).
const SELF_PING_LAMPORTS = 5000;

/** A minimal real SOL transfer from the hot wallet to itself: same build → sign → submit →
 * confirm pipeline as `sendTransfer`, but with no destination/balance validation needed
 * (the funds never leave the wallet). This is `MockSwapExecutor`'s stand-in "fill" — it
 * proves live signing/submission/confirmation genuinely works on devnet for every trade
 * `liveExecutor.ts` places, without needing Jupiter or a real counterparty. Deliberately
 * always devnet — this is a devnet-only testing tool, never given a network parameter. */
export async function sendSelfPing(): Promise<SendResult> {
  const from = hotWalletPubkeyOrThrow();

  const tx = new Transaction();
  tx.add(SystemProgram.transfer({ fromPubkey: from, toPubkey: from, lamports: SELF_PING_LAMPORTS }));

  const { blockhash, lastValidBlockHeight } = await devnetConnection.getLatestBlockhash("confirmed");
  tx.recentBlockhash = blockhash;
  tx.feePayer = from;
  signLegacyTransaction(tx);

  const txHash = await devnetConnection.sendRawTransaction(tx.serialize());
  const confirmationSlot = await confirmWithTimeout(devnetConnection, txHash, blockhash, lastValidBlockHeight);

  return { txHash, confirmationSlot };
}

export function explorerUrl(txHash: string, network: Network = "devnet"): string {
  return `https://explorer.solana.com/tx/${txHash}?cluster=${network === "mainnet" ? "mainnet-beta" : "devnet"}`;
}
