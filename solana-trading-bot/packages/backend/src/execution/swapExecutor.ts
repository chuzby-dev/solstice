import { Connection, VersionedTransaction } from "@solana/web3.js";
import { config, estimateTradeFeeUsd, walletSend } from "../config.js";
import { getHotWalletPublicKey, signVersionedTransaction } from "../wallet/hotWallet.js";
import { sendSelfPing } from "../wallet/txBuilder.js";
import { getRiskLimits } from "./riskSettings.js";

/** Result of one fill, in the same shape liveExecutor.ts's bookkeeping needs to replicate
 * simulator.ts's applyFill math exactly (fee folded into cost basis, etc.) — `priceUsd`
 * itself is NOT part of this result because every executor fills at the price it was
 * quoted/passed (the caller's latest tick), not a price of its own choosing. */
export interface SwapFillResult {
  sizeToken: number;
  feeUsd: number;
  txHash: string;
  confirmationSlot: number | null;
}

export interface RiskMappingPreview {
  assumedLiquidityUsd: number;
  simulatedSlippageBps: number;
}

export interface SwapExecutor {
  swap(params: { action: "buy" | "sell"; tokenMint: string; sizeUsd: number; priceUsd: number }): Promise<SwapFillResult>;
  /** Real-market-data risk-mapping inputs for a signal's REQUESTED (not-yet-shrunk) size,
   * fetched before the risk manager decides whether/how to shrink it — see
   * liveExecutor.ts's executeSignal. Optional: an executor with no real market data to
   * offer (MockSwapExecutor) simply omits this, and the caller falls back to the same
   * Phase-1 placeholder assumptions execution/simulator.ts uses for paper. */
  previewRiskMapping?(params: { action: "buy" | "sell"; tokenMint: string; sizeUsd: number; priceUsd: number }): Promise<RiskMappingPreview>;
}

/** Stage 3's stand-in for `RealJupiterSwapExecutor`. Jupiter's Swap API has no devnet
 * liquidity to route against — there is no way to prove a REAL swap against real market
 * depth on devnet. What CAN be proven for real on devnet is everything else a live trade
 * touches: signing with the actual hot wallet key, submitting and confirming a real
 * transaction, and every layer of liveExecutor.ts's bookkeeping (risk gating, in-flight
 * guard, position/trade persistence, stop-loss). This executor computes the exact same
 * fee/size economics the paper simulator already uses (see execution/simulator.ts's
 * applyFill and backtest/ledger.ts's applyBuy/applySell), but backs the "fill" with a real
 * signed-and-submitted devnet transaction (`sendSelfPing`) instead of an in-memory number —
 * so a MockSwapExecutor-driven trade genuinely exercises the live signing pipeline
 * end-to-end, not just simulated math with a devnet label on it. Devnet-only by
 * construction: it calls `sendSelfPing`, which is hardcoded to txBuilder.ts's devnet
 * connection (see that file's header comment) — there is no path here that could reach
 * mainnet. Used only by tests/manual devnet verification now — engine.ts's live path uses
 * `RealJupiterSwapExecutor` (below). */
export class MockSwapExecutor implements SwapExecutor {
  async swap(params: { action: "buy" | "sell"; sizeUsd: number; priceUsd: number }): Promise<SwapFillResult> {
    const sizeToken = params.sizeUsd / params.priceUsd;
    const feeUsd = estimateTradeFeeUsd(params.sizeUsd);
    const { txHash, confirmationSlot } = await sendSelfPing();
    return { sizeToken, feeUsd, txHash, confirmationSlot };
  }
}

// --- Real Jupiter integration (mainnet-only) ---
//
// Jupiter has no devnet liquidity, so this is the one piece of the live pipeline that can
// only be proven against real mainnet market data. `RealJupiterSwapExecutor.swap()` now
// genuinely submits a signed transaction to mainnet and waits for confirmation — this is
// the real trade-execution path. It is only ever reachable via engine.ts's live-mode
// dispatch, which itself only runs when a human has: (1) created the hot wallet, (2)
// typed an explicit confirmation phrase to switch network to mainnet, and (3) reviewed
// every active strategy's backtest verdict and acknowledged enabling live mode (see
// components/Wallet.tsx's ModeControl). Nothing here bypasses the risk manager, the
// mandatory stop-loss, or the kill switch — see execution/liveExecutor.ts.

// Jupiter deprecated the old quote-api.jup.ag/v6/* domain (October 2025) in favor of a
// unified api.jup.ag/swap/v1/* gateway. Keyless requests still work here, just capped at a
// low rate limit (0.5 RPS) — fine for this app's low-frequency trading cadence; an API key
// would only be needed to raise that ceiling, not to unlock functionality.
const JUPITER_QUOTE_URL = "https://api.jup.ag/swap/v1/quote";
const JUPITER_SWAP_URL = "https://api.jup.ag/swap/v1/swap";

// This app only ever trades a non-USDC token (currently just SOL) against USDC — see
// config.ts's TOKEN_ALLOWLIST and every other module's "cash is USD-denominated" model.
// Decimals are the real, well-known values for these two specific mainnet mints, not
// looked up dynamically — matches SOL_MINT's existing hardcoded-constant precedent
// elsewhere in this codebase (wallet/txBuilder.ts).
const USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const SOL_DECIMALS = 9;
const USDC_DECIMALS = 6;

// Mainnet-only, separate from every other Connection in this codebase (all of which are
// hardcoded to devnet — see wallet/txBuilder.ts, wallet/walletRoutes.ts,
// execution/liveExecutor.ts's balance provider). This is the one and only Connection in
// the app that can reach mainnet.
const connection = new Connection(config.solanaMainnetRpcUrl, "confirmed");

export interface JupiterQuote {
  inputMint: string;
  outputMint: string;
  inAmount: string;
  outAmount: string;
  otherAmountThreshold: string;
  /** Decimal fraction, e.g. "0.0042" means 0.42% — NOT already a percent. */
  priceImpactPct: string;
  slippageBps: number;
}

export interface SignedSwapTransaction {
  transaction: VersionedTransaction;
  quote: JupiterQuote;
  lastValidBlockHeight: number;
}

/** Which mint is spent vs. received, and how much of the input mint (in its raw base
 * units) a `{action, tokenMint, sizeUsd, priceUsd}` request corresponds to. Shared by
 * `previewRiskMapping` and `swap` so both quote for the exact same trade shape. */
function resolveQuoteParams(params: { action: "buy" | "sell"; tokenMint: string; sizeUsd: number; priceUsd: number }): {
  inputMint: string;
  outputMint: string;
  amountRaw: string;
} {
  const isBuy = params.action === "buy";
  const inputMint = isBuy ? USDC_MINT : params.tokenMint;
  const outputMint = isBuy ? params.tokenMint : USDC_MINT;
  const amountRaw = isBuy
    ? Math.round(params.sizeUsd * 10 ** USDC_DECIMALS)
    : Math.round((params.sizeUsd / params.priceUsd) * 10 ** SOL_DECIMALS);
  return { inputMint, outputMint, amountRaw: String(amountRaw) };
}

/** Real, read-only mainnet market data — fetching a quote doesn't move anything, the same
 * way checking a price does not. */
export async function fetchJupiterQuote(params: { inputMint: string; outputMint: string; amountRaw: string; slippageBps: number }): Promise<JupiterQuote> {
  const url = new URL(JUPITER_QUOTE_URL);
  url.searchParams.set("inputMint", params.inputMint);
  url.searchParams.set("outputMint", params.outputMint);
  url.searchParams.set("amount", params.amountRaw);
  url.searchParams.set("slippageBps", String(params.slippageBps));

  const res = await fetch(url);
  if (!res.ok) throw new Error(`Jupiter quote request failed: ${res.status} ${await res.text()}`);
  return (await res.json()) as JupiterQuote;
}

/** Back-computes an "assumed liquidity" figure from a real quote's own price impact,
 * replacing simulator.ts's fixed `ASSUMED_LIQUIDITY_USD` placeholder — see
 * docs/ARCHITECTURE.md's known-limitations note. `priceImpactPctAsPercent` must already be
 * scaled to percent units (e.g. 1 for 1%), matching how RiskLimits.maxPriceImpactPct is
 * expressed — NOT the raw decimal fraction Jupiter's API returns. */
export function assumedLiquidityFromQuote(requestedSizeUsd: number, priceImpactPctAsPercent: number): number {
  if (priceImpactPctAsPercent <= 0) return Number.POSITIVE_INFINITY; // no measurable impact at this size
  return requestedSizeUsd / (priceImpactPctAsPercent / 100);
}

/** A worst-case slippage figure straight from the quote's own guaranteed-minimum-out
 * (`otherAmountThreshold`), replacing simulator.ts's fixed `SIMULATED_SLIPPAGE_BPS`
 * placeholder. This is the same bound Jupiter's swap transaction itself enforces
 * on-chain — a fill worse than this reverts atomically by construction, so risk-manager
 * rejection on this figure is a pre-check, not the only thing standing between a signal
 * and a bad fill. */
export function simulatedSlippageBpsFromQuote(outAmount: bigint, otherAmountThreshold: bigint): number {
  if (outAmount <= 0n) return 0;
  const shortfall = outAmount - otherAmountThreshold;
  return Number((shortfall * 10_000n) / outAmount);
}

/** Fetches the swap transaction Jupiter itself constructs for `quote` (which instructions,
 * which accounts, route — all Jupiter's own decisions, not ours) and signs it with the hot
 * wallet's real key. Does not submit — that's the caller's job (see
 * RealJupiterSwapExecutor.swap() below), kept separate so the build/sign step stays
 * independently testable without a live RPC connection. */
export async function buildAndSignJupiterSwap(quote: JupiterQuote, userPublicKey: string): Promise<SignedSwapTransaction> {
  const res = await fetch(JUPITER_SWAP_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ quoteResponse: quote, userPublicKey, wrapAndUnwrapSol: true }),
  });
  if (!res.ok) throw new Error(`Jupiter swap-transaction request failed: ${res.status} ${await res.text()}`);
  const { swapTransaction, lastValidBlockHeight } = (await res.json()) as { swapTransaction: string; lastValidBlockHeight: number };

  const transaction = VersionedTransaction.deserialize(Buffer.from(swapTransaction, "base64"));
  signVersionedTransaction(transaction);
  return { transaction, quote, lastValidBlockHeight };
}

async function confirmSwapWithTimeout(signature: string, blockhash: string, lastValidBlockHeight: number): Promise<number | null> {
  const timeout = new Promise<"timeout">((resolve) => setTimeout(() => resolve("timeout"), walletSend.confirmationTimeoutMs));
  const confirmation = connection.confirmTransaction({ signature, blockhash, lastValidBlockHeight }, "confirmed");

  const result = await Promise.race([confirmation, timeout]);
  if (result === "timeout") {
    // Not necessarily failed — it may still land late, same reasoning as
    // wallet/txBuilder.ts's confirmWithTimeout. Caller gets the signature back either way.
    return null;
  }
  if (result.value.err) {
    throw new Error(`Live swap transaction failed on-chain: ${JSON.stringify(result.value.err)}`);
  }

  const status = await connection.getSignatureStatuses([signature]);
  return status.value[0]?.slot ?? null;
}

/** The real, submitting live-trade executor. Every call: fetches a real quote, re-checks
 * that real quote's own slippage/price-impact against the CURRENT configured risk limits
 * (a second, real-market-data check — the risk manager's own pass in liveExecutor.ts runs
 * BEFORE this executor is invoked, against numbers from `previewRiskMapping` below rather
 * than blind Phase-1 placeholders, but that preview is necessarily for the signal's
 * originally-*requested* size; if the risk manager then shrinks the size, this is the
 * re-quote-at-the-adjusted-amount step, and it aborts before signing anything if even
 * these final real numbers are worse than what was assumed), then builds, signs, submits,
 * and confirms. Aborting here (throwing) is fail-closed — nothing was ever signed or
 * sent — mirroring how a preflight-checked `sendRawTransaction` against an unfunded or
 * underfunded wallet fails safely on its own before anything moves. */
export class RealJupiterSwapExecutor implements SwapExecutor {
  /** Real-market-data risk-mapping inputs for the signal's requested size, fetched BEFORE
   * the risk manager runs (see liveExecutor.ts's executeSignal) — replaces
   * simulator.ts's fixed ASSUMED_LIQUIDITY_USD/SIMULATED_SLIPPAGE_BPS placeholders with
   * numbers derived from an actual quote at actual current market depth, so sizing
   * decisions (shrink-to-fit-impact-ceiling, reject-on-slippage) reflect reality instead
   * of a guess. Read-only — fetching a quote doesn't move anything. */
  async previewRiskMapping(params: { action: "buy" | "sell"; tokenMint: string; sizeUsd: number; priceUsd: number }): Promise<RiskMappingPreview> {
    const limits = getRiskLimits();
    const { inputMint, outputMint, amountRaw } = resolveQuoteParams(params);
    const quote = await fetchJupiterQuote({ inputMint, outputMint, amountRaw, slippageBps: limits.maxSlippageBps });

    return {
      assumedLiquidityUsd: assumedLiquidityFromQuote(params.sizeUsd, Number(quote.priceImpactPct) * 100),
      simulatedSlippageBps: simulatedSlippageBpsFromQuote(BigInt(quote.outAmount), BigInt(quote.otherAmountThreshold)),
    };
  }

  async swap(params: { action: "buy" | "sell"; tokenMint: string; sizeUsd: number; priceUsd: number }): Promise<SwapFillResult> {
    const userPublicKey = getHotWalletPublicKey();
    if (!userPublicKey) throw new Error("No hot wallet has been created yet — cannot sign a live swap.");

    const limits = getRiskLimits();
    const isBuy = params.action === "buy";
    const { inputMint, outputMint, amountRaw } = resolveQuoteParams(params);

    const quote = await fetchJupiterQuote({ inputMint, outputMint, amountRaw, slippageBps: limits.maxSlippageBps });

    const realSlippageBps = simulatedSlippageBpsFromQuote(BigInt(quote.outAmount), BigInt(quote.otherAmountThreshold));
    if (realSlippageBps > limits.maxSlippageBps) {
      throw new Error(`Real quote slippage ${realSlippageBps}bps exceeds the configured ceiling of ${limits.maxSlippageBps}bps — aborted before signing.`);
    }
    const realPriceImpactPct = Number(quote.priceImpactPct) * 100;
    if (realPriceImpactPct > limits.maxPriceImpactPct) {
      throw new Error(`Real quote price impact ${realPriceImpactPct.toFixed(3)}% exceeds the configured ceiling of ${limits.maxPriceImpactPct}% — aborted before signing.`);
    }

    const { transaction, lastValidBlockHeight } = await buildAndSignJupiterSwap(quote, userPublicKey);

    const txHash = await connection.sendRawTransaction(transaction.serialize(), { maxRetries: 2 });
    const confirmationSlot = await confirmSwapWithTimeout(txHash, transaction.message.recentBlockhash, lastValidBlockHeight);

    const sizeToken = isBuy ? Number(quote.outAmount) / 10 ** SOL_DECIMALS : Number(quote.inAmount) / 10 ** SOL_DECIMALS;
    // Estimated, same as every other executor's fee — see config.ts's estimateTradeFeeUsd
    // doc for why this is a planning estimate rather than the exact real network fee.
    const feeUsd = estimateTradeFeeUsd(params.sizeUsd);

    return { sizeToken, feeUsd, txHash, confirmationSlot };
  }
}
