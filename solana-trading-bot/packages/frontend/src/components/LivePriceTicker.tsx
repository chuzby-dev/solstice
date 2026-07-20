import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api.js";
import { useLiveFeed } from "../hooks/useWebSocket.js";
import { TOKEN_ALLOWLIST } from "../lib/tokens.js";

/** Always-visible header ticker so a live price is on screen everywhere in the app,
 * not just inside the strategy configuration form. */
export function LivePriceTicker(): JSX.Element {
  const live = useLiveFeed();
  const initial = useQuery({ queryKey: ["live-prices"], queryFn: api.getLivePrices, staleTime: 60_000 });

  return (
    <div className="flex items-center gap-3 text-xs">
      {TOKEN_ALLOWLIST.map((t) => {
        const tick = live.prices[t.mint] ?? initial.data?.find((p) => p.tokenMint === t.mint);
        return (
          <span key={t.mint} className="rounded bg-slate-900 px-2 py-1 font-mono text-slate-300">
            {t.symbol} <span className="text-slate-100">{tick ? `$${tick.priceUsd.toFixed(4)}` : "…"}</span>
          </span>
        );
      })}
    </div>
  );
}
