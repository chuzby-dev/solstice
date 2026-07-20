import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Line, LineChart, ResponsiveContainer, Tooltip, YAxis } from "recharts";
import { api } from "../lib/api.js";
import { useLiveFeed } from "../hooks/useWebSocket.js";

interface PriceSparklineProps {
  tokenMint: string;
  tokenSymbol: string;
  windowMinutes?: number;
}

interface Point {
  time: number;
  price: number;
}

/** Live price + a short rolling-window chart, so the user can see recent movement
 * before typing in price-based strategy limits (e.g. a grid's price range) instead of
 * guessing blind. Seeds from the REST history endpoint, then stays live via the
 * existing WebSocket price_tick feed. */
export function PriceSparkline({ tokenMint, tokenSymbol, windowMinutes = 5 }: PriceSparklineProps): JSX.Element {
  const live = useLiveFeed();
  const history = useQuery({
    queryKey: ["price-history", tokenMint, windowMinutes],
    queryFn: () => api.getPriceHistory(tokenMint, windowMinutes),
    refetchInterval: 15_000,
  });

  const [points, setPoints] = useState<Point[]>([]);

  useEffect(() => {
    if (history.data) {
      setPoints(history.data.map((t) => ({ time: new Date(t.timestamp).getTime(), price: t.priceUsd })));
    }
  }, [history.data]);

  const latestTick = live.prices[tokenMint];
  useEffect(() => {
    if (!latestTick) return;
    setPoints((prev) => {
      const cutoff = Date.now() - windowMinutes * 60_000;
      const withNew = [...prev, { time: new Date(latestTick.timestamp).getTime(), price: latestTick.priceUsd }];
      return withNew.filter((p) => p.time >= cutoff);
    });
  }, [latestTick?.timestamp, windowMinutes]);

  const latestPrice = points[points.length - 1]?.price ?? latestTick?.priceUsd;
  const firstPrice = points[0]?.price;
  const changePct = latestPrice !== undefined && firstPrice !== undefined && firstPrice !== 0 ? ((latestPrice - firstPrice) / firstPrice) * 100 : null;

  return (
    <div className="rounded border border-slate-800 bg-slate-950/50 p-2">
      <div className="mb-1 flex items-center justify-between">
        <span className="text-xs text-slate-400">
          {tokenSymbol} · last {windowMinutes}min
        </span>
        {latestPrice !== undefined ? (
          <span className="text-sm font-medium text-slate-100">
            ${latestPrice.toFixed(4)}
            {changePct !== null && (
              <span className={`ml-1 text-xs ${changePct >= 0 ? "text-emerald-400" : "text-red-400"}`}>
                {changePct >= 0 ? "+" : ""}
                {changePct.toFixed(2)}%
              </span>
            )}
          </span>
        ) : (
          <span className="text-xs text-slate-600">no price yet</span>
        )}
      </div>
      <div className="h-12">
        {points.length >= 2 ? (
          <ResponsiveContainer width="100%" height="100%">
            <LineChart data={points}>
              <YAxis domain={["auto", "auto"]} hide />
              <Tooltip
                contentStyle={{ background: "#0f172a", border: "1px solid #1e293b", fontSize: 11 }}
                labelFormatter={(t) => new Date(t as number).toLocaleTimeString()}
                formatter={(v: number) => [`$${v.toFixed(4)}`, "price"]}
              />
              <Line type="monotone" dataKey="price" stroke={changePct !== null && changePct < 0 ? "#f87171" : "#22d3ee"} dot={false} strokeWidth={1.5} />
            </LineChart>
          </ResponsiveContainer>
        ) : (
          <p className="text-xs text-slate-600">Collecting price history…</p>
        )}
      </div>
    </div>
  );
}
