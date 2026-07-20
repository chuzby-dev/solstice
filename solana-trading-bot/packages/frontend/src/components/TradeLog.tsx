import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api.js";
import { TOKEN_ALLOWLIST } from "../lib/tokens.js";

export function TradeLog(): JSX.Element {
  const [tokenMint, setTokenMint] = useState<string>("");
  const [actionFilter, setActionFilter] = useState<"" | "buy" | "sell">("");

  const trades = useQuery({
    queryKey: ["trades", tokenMint],
    queryFn: () => api.getTrades({ tokenMint: tokenMint || undefined, limit: 200 }),
    refetchInterval: 10_000,
  });

  const filtered = (trades.data ?? []).filter((t) => !actionFilter || t.action === actionFilter);

  return (
    <div className="space-y-4">
      <div className="flex gap-3">
        <select className="rounded bg-slate-800 p-2 text-sm" value={tokenMint} onChange={(e) => setTokenMint(e.target.value)}>
          <option value="">All tokens</option>
          {TOKEN_ALLOWLIST.map((t) => (
            <option key={t.mint} value={t.mint}>
              {t.symbol}
            </option>
          ))}
        </select>
        <select className="rounded bg-slate-800 p-2 text-sm" value={actionFilter} onChange={(e) => setActionFilter(e.target.value as "" | "buy" | "sell")}>
          <option value="">Buy + Sell</option>
          <option value="buy">Buy only</option>
          <option value="sell">Sell only</option>
        </select>
      </div>

      <div className="overflow-x-auto rounded-lg border border-slate-800">
        <table className="w-full text-sm">
          <thead className="bg-slate-900 text-left text-slate-500">
            <tr>
              <th className="p-2">Time</th>
              <th className="p-2">Action</th>
              <th className="p-2">Token</th>
              <th className="p-2">Price</th>
              <th className="p-2">Size (USD)</th>
              <th className="p-2">Fee</th>
              <th className="p-2">Strategy</th>
              <th className="p-2">Reason</th>
              <th className="p-2">Tx</th>
            </tr>
          </thead>
          <tbody>
            {filtered.map((t) => (
              <tr key={t.id} className="border-t border-slate-800">
                <td className="p-2 text-slate-400">{new Date(t.timestamp).toLocaleString()}</td>
                <td className={`p-2 font-medium ${t.action === "buy" ? "text-emerald-400" : "text-red-400"}`}>{t.action.toUpperCase()}</td>
                <td className="p-2">{t.tokenSymbol}</td>
                <td className="p-2">${t.priceUsd.toFixed(4)}</td>
                <td className="p-2">${t.sizeUsd.toFixed(2)}</td>
                <td className="p-2 text-amber-400/80">${t.feeUsd.toFixed(4)}</td>
                <td className="p-2 text-slate-400">{t.strategyId}</td>
                <td className="max-w-xs truncate p-2 text-slate-400" title={t.reason}>
                  {t.reason}
                </td>
                <td className="p-2">
                  {/* Phase 1 is paper-trading only: every trade is simulated, so there is
                      never a real tx hash / Solscan link. This becomes a live link once
                      real execution lands. */}
                  <span className="rounded bg-slate-800 px-2 py-0.5 text-xs text-slate-400">SIMULATED</span>
                </td>
              </tr>
            ))}
            {filtered.length === 0 && (
              <tr>
                <td colSpan={9} className="p-4 text-center text-slate-500">
                  No trades match the current filters.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
