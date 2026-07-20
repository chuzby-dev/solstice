import { Connection, PublicKey, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import type { Network } from "@trading-bot/shared";
import { config } from "../config.js";

// The one place in the app that maps network -> RPC connection for plain balance reads —
// walletRoutes.ts (hot wallet balance display) and execution/autoSweep.ts (deciding
// whether there's anything to sweep) both need this, so it lives here once instead of
// twice. Not used by the live-trading path itself (execution/liveExecutor.ts's own
// RpcWalletBalanceProvider is deliberately separate — it's mainnet-only, SOL+USDC-shaped,
// and short-TTL cached for tick-rate polling, none of which this generic helper needs).
const devnetConnection = new Connection(config.solanaDevnetRpcUrl, "confirmed");
const mainnetConnection = new Connection(config.solanaMainnetRpcUrl, "confirmed");

export function connectionForNetwork(network: Network): Connection {
  return network === "mainnet" ? mainnetConnection : devnetConnection;
}

export interface WalletBalances {
  solBalance: number;
  tokenBalances: { mint: string; amount: number }[];
}

export async function fetchWalletBalances(network: Network, owner: PublicKey): Promise<WalletBalances> {
  const conn = connectionForNetwork(network);
  const [lamports, tokenAccounts] = await Promise.all([conn.getBalance(owner), conn.getParsedTokenAccountsByOwner(owner, { programId: TOKEN_PROGRAM_ID })]);

  const tokenBalances = tokenAccounts.value
    .map((acc) => {
      const info = acc.account.data.parsed.info;
      return { mint: info.mint as string, amount: Number(info.tokenAmount.uiAmountString ?? 0) };
    })
    .filter((t) => config.tokenAllowlist.includes(t.mint));

  return { solBalance: lamports / LAMPORTS_PER_SOL, tokenBalances };
}
