import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { useLiveFeed } from "../hooks/useWebSocket.js";
import { usePortfolioQuery } from "../hooks/usePortfolio.js";
import { api } from "../lib/api.js";

const MAX_HISTORY_POINTS = 120;

function formatUsd(value: number): string {
  return value.toLocaleString("en-US", { style: "currency", currency: "USD" });
}

export function Dashboard(): JSX.Element {
  const live = useLiveFeed();
  const initial = usePortfolioQuery();
  const configs = useQuery({ queryKey: ["strategies"], queryFn: api.getStrategies });

  const portfolio = live.portfolio ?? initial.data ?? null;

  const [history, setHistory] = useState<{ time: string; value: number }[]>([]);
  useEffect(() => {
    if (!portfolio) return;
    setHistory((h) =>
      [...h, { time: new Date(portfolio.timestamp).toLocaleTimeString(), value: portfolio.totalValueUsd }].slice(-MAX_HISTORY_POINTS),
    );
  }, [portfolio?.timestamp]);

  const activeStrategies = configs.data?.filter((c) => c.active) ?? [];
  const strategyNameFor = (strategyConfigId: string): string => configs.data?.find((c) => c.id === strategyConfigId)?.strategyId ?? "unknown";

  if (!portfolio) {
    return <p className="text-slate-400">Loading portfolio…</p>;
  }

  return (
    <div className="space-y-6">
      {live.paused && (
        <div className="rounded border border-amber-700 bg-amber-950/50 px-4 py-2 text-sm text-amber-300">
          ⏸ Trading is paused (kill switch active). No new simulated trades will execute until resumed in Settings.
        </div>
      )}

      <div className="grid grid-cols-2 gap-4 md:grid-cols-4">
        <StatCard label="Total Value (simulated)" value={formatUsd(portfolio.totalValueUsd)} />
        <StatCard label="Cash" value={formatUsd(portfolio.cashUsd)} />
        <StatCard
          label="Realized P&L"
          value={formatUsd(portfolio.realizedPnlUsd)}
          tone={portfolio.realizedPnlUsd >= 0 ? "positive" : "negative"}
        />
        <StatCard
          label="Unrealized P&L"
          value={formatUsd(portfolio.unrealizedPnlUsd)}
          tone={portfolio.unrealizedPnlUsd >= 0 ? "positive" : "negative"}
        />
      </div>

      <div className="rounded-lg border border-slate-800 bg-slate-900 p-4">
        <h2 className="mb-2 text-sm font-medium text-slate-400">Portfolio Value</h2>
        <div className="h-56">
          <ResponsiveContainer width="100%" height="100%">
            <LineChart data={history}>
              <XAxis dataKey="time" tick={{ fontSize: 10, fill: "#94a3b8" }} minTickGap={30} />
              <YAxis tick={{ fontSize: 10, fill: "#94a3b8" }} domain={["auto", "auto"]} width={70} />
              <Tooltip contentStyle={{ background: "#0f172a", border: "1px solid #1e293b" }} formatter={(v: number) => formatUsd(v)} />
              <Line type="monotone" dataKey="value" stroke="#22d3ee" dot={false} strokeWidth={2} />
            </LineChart>
          </ResponsiveContainer>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <div className="rounded-lg border border-slate-800 bg-slate-900 p-4">
          <h2 className="mb-2 text-sm font-medium text-slate-400">Open Positions</h2>
          {portfolio.positions.length === 0 ? (
            <p className="text-sm text-slate-500">No open positions.</p>
          ) : (
            <table className="w-full text-sm">
              <thead className="text-left text-slate-500">
                <tr>
                  <th className="pb-1">Strategy</th>
                  <th className="pb-1">Token</th>
                  <th className="pb-1">Qty</th>
                  <th className="pb-1">Avg Entry</th>
                  <th className="pb-1">Price</th>
                  <th className="pb-1">Unrealized</th>
                </tr>
              </thead>
              <tbody>
                {portfolio.positions.map((p) => (
                  <tr key={p.strategyConfigId} className="border-t border-slate-800">
                    <td className="py-1 text-slate-400">{strategyNameFor(p.strategyConfigId)}</td>
                    <td>{p.tokenSymbol}</td>
                    <td>{p.quantity.toFixed(4)}</td>
                    <td>{formatUsd(p.avgEntryPriceUsd)}</td>
                    <td>{formatUsd(p.currentPriceUsd)}</td>
                    <td className={p.unrealizedPnlUsd >= 0 ? "text-emerald-400" : "text-red-400"}>{formatUsd(p.unrealizedPnlUsd)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>

        <div className="rounded-lg border border-slate-800 bg-slate-900 p-4">
          <h2 className="mb-2 text-sm font-medium text-slate-400">Active Strategies ({activeStrategies.length})</h2>
          {activeStrategies.length === 0 ? (
            <p className="text-sm text-slate-500">No strategies active. Configure one in the Strategies tab.</p>
          ) : (
            <ul className="space-y-1 text-sm">
              {activeStrategies.map((s) => (
                <li key={s.id} className="flex justify-between border-t border-slate-800 py-1 first:border-t-0">
                  <span>{s.strategyId}</span>
                  <span className="text-slate-400">{s.tokenSymbol}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      <div className="rounded-lg border border-slate-800 bg-slate-900 p-4">
        <h2 className="mb-2 text-sm font-medium text-slate-400">Recent Trades</h2>
        {live.recentTrades.length === 0 ? (
          <p className="text-sm text-slate-500">No trades yet.</p>
        ) : (
          <ul className="space-y-1 text-sm">
            {live.recentTrades.slice(0, 8).map((t) => (
              <li key={t.id} className="flex justify-between border-t border-slate-800 py-1 first:border-t-0">
                <span className={t.action === "buy" ? "text-emerald-400" : "text-red-400"}>
                  {t.action.toUpperCase()} {t.sizeToken.toFixed(4)} {t.tokenSymbol}
                </span>
                <span className="text-slate-400">{formatUsd(t.priceUsd)}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function StatCard({ label, value, tone }: { label: string; value: string; tone?: "positive" | "negative" }): JSX.Element {
  const toneClass = tone === "positive" ? "text-emerald-400" : tone === "negative" ? "text-red-400" : "text-slate-100";
  return (
    <div className="rounded-lg border border-slate-800 bg-slate-900 p-4">
      <p className="text-xs text-slate-500">{label}</p>
      <p className={`mt-1 text-xl font-semibold ${toneClass}`}>{value}</p>
    </div>
  );
}
