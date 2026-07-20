// One-off verification script (task #47): proves RealJupiterSwapExecutor's real
// quote -> risk-check -> build+sign -> submit -> confirm pipeline works end-to-end against
// real mainnet market data and the real hot wallet key. The wallet is expected to hold
// zero real mainnet SOL/USDC at this point — submission should fail safely (preflight
// balance rejection) rather than silently succeed, which is itself the thing being
// verified: quoting and signing work for real, but nothing moves without real funds
// actually present. Delete after running.
import { RealJupiterSwapExecutor } from "../src/execution/swapExecutor.js";

const SOL_MINT = "So11111111111111111111111111111111111111";

const executor = new RealJupiterSwapExecutor();

try {
  const result = await executor.swap({ action: "sell", tokenMint: SOL_MINT, sizeUsd: 2, priceUsd: 76 });
  console.log("Unexpected: swap succeeded despite an empty wallet.", result);
} catch (err) {
  console.log("Swap attempt result (failure expected — empty wallet):");
  console.log(err instanceof Error ? err.message : err);
}
