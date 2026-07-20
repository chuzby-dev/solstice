import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Dashboard } from "./components/Dashboard.js";
import { StrategySelector } from "./components/StrategySelector.js";
import { TradeLog } from "./components/TradeLog.js";
import { Settings } from "./components/Settings.js";
import { Wallet } from "./components/Wallet.js";
import { LivePriceTicker } from "./components/LivePriceTicker.js";
import { useLiveFeed } from "./hooks/useWebSocket.js";
import { api } from "./lib/api.js";

const TABS = ["Dashboard", "Strategies", "Wallet", "Trade Log", "Settings"] as const;
type Tab = (typeof TABS)[number];

export function App(): JSX.Element {
  const [tab, setTab] = useState<Tab>("Dashboard");
  const live = useLiveFeed();
  // Polled (not just fetched once) so this banner can't go stale while the tab sits open —
  // mode can change from another browser tab, or reset to paper on a server restart.
  const mode = useQuery({ queryKey: ["app-mode"], queryFn: api.getMode, refetchInterval: 5_000 });
  const isLive = mode.data?.tradingMode === "live";

  // The browser tab title is the one piece of "what mode am I in" text that's visible
  // even when this tab isn't focused — index.html's static <title> was permanently
  // "Paper Trading (Devnet)" regardless of actual state, which is exactly the kind of
  // stale label that can make a real mainnet session look safely simulated at a glance.
  //
  // Paper trading itself never touches any network at all — simulator.ts prices from
  // real market data but the trade and the money are entirely virtual, full stop. The
  // network selection only matters for the WALLET (its real balance, manual sends,
  // auto-sweep) and for live trading. So this deliberately never says "Paper Trading
  // (Mainnet)" — that reads as if paper trades execute on mainnet, which they don't; the
  // wallet's target network is a separate fact, worded as its own clause.
  useEffect(() => {
    if (!mode.data) return;
    document.title = isLive
      ? "Solana Trading Bot — LIVE (Mainnet)"
      : `Solana Trading Bot — Paper Trading · Wallet: ${mode.data.network === "mainnet" ? "Mainnet" : "Devnet"}`;
  }, [mode.data, isLive]);

  return (
    <div className="mx-auto min-h-screen max-w-6xl px-4 py-6">
      {isLive && (
        <div className="mb-4 rounded-lg border border-red-800 bg-red-950/40 px-4 py-2 text-center text-sm font-medium text-red-300">
          LIVE — active strategies are trading real funds autonomously on Mainnet
        </div>
      )}
      <header className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold">Solana Trading Bot</h1>
          <p className="text-xs text-slate-500">
            {mode.data
              ? isLive
                ? "LIVE · Trading real funds on Mainnet"
                : `Paper trading (simulated, virtual money) · Wallet: ${mode.data.network === "mainnet" ? "Mainnet" : "Devnet"}`
              : "Loading mode…"}
          </p>
        </div>
        <div className="flex items-center gap-3">
          <LivePriceTicker />
          <div className="flex items-center gap-2 text-xs">
            <span className={`h-2 w-2 rounded-full ${live.connected ? "bg-emerald-500" : "bg-red-500"}`} />
            {live.connected ? "Connected" : "Reconnecting…"}
            {live.paused && <span className="ml-2 rounded-full bg-amber-900 px-2 py-0.5 text-amber-300">PAUSED</span>}
          </div>
        </div>
      </header>

      <nav className="mb-6 flex gap-1 border-b border-slate-800">
        {TABS.map((t) => (
          <button
            key={t}
            onClick={() => setTab(t)}
            className={`px-4 py-2 text-sm ${tab === t ? "border-b-2 border-emerald-500 text-slate-100" : "text-slate-500 hover:text-slate-300"}`}
          >
            {t}
          </button>
        ))}
      </nav>

      <main>
        {tab === "Dashboard" && <Dashboard />}
        {tab === "Strategies" && <StrategySelector />}
        {tab === "Wallet" && <Wallet />}
        {tab === "Trade Log" && <TradeLog />}
        {tab === "Settings" && <Settings />}
      </main>
    </div>
  );
}
