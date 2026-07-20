import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useWallet } from "@solana/wallet-adapter-react";
import { WalletMultiButton } from "@solana/wallet-adapter-react-ui";
import type { RiskLimits } from "@trading-bot/shared";
import { api } from "../lib/api.js";
import { TOKEN_ALLOWLIST } from "../lib/tokens.js";
import { useLiveFeed } from "../hooks/useWebSocket.js";

export function Settings(): JSX.Element {
  const { publicKey } = useWallet();
  const live = useLiveFeed();
  const queryClient = useQueryClient();

  const balance = useQuery({
    queryKey: ["wallet-balance", publicKey?.toBase58()],
    queryFn: () => api.getWalletBalance(publicKey!.toBase58()),
    enabled: !!publicKey,
  });

  const riskSettings = useQuery({ queryKey: ["risk-settings"], queryFn: api.getRiskSettings });
  const updateRisk = useMutation({
    mutationFn: api.updateRiskSettings,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["risk-settings"] }),
  });

  const pause = useMutation({ mutationFn: api.pause });
  const resume = useMutation({ mutationFn: api.resume });

  const [draft, setDraft] = useState<RiskLimits | null>(null);
  const current = draft ?? riskSettings.data ?? null;

  return (
    <div className="space-y-8">
      <section className="rounded-lg border border-slate-800 bg-slate-900 p-4">
        <h2 className="mb-3 text-sm font-medium text-slate-400">External Wallet Connect (devnet, view-only)</h2>
        <p className="mb-3 text-xs text-slate-500">
          Connects an EXTERNAL wallet (Phantom/Solflare) purely to read its public devnet balance — never requests a
          signature, never moves funds. This is separate from the bot's own trading wallet on the Wallet tab, which is a
          server-generated wallet the app custodies and signs from itself.
        </p>
        <div className="mb-3">
          <WalletMultiButton />
        </div>
        {publicKey && (
          <div className="text-sm text-slate-300">
            <p className="font-mono text-xs text-slate-500">{publicKey.toBase58()}</p>
            {balance.data && (
              <p className="mt-1">
                {balance.data.solBalance.toFixed(4)} SOL (devnet)
                {balance.data.tokenBalances.map((t) => (
                  <span key={t.mint} className="ml-3">
                    {t.amount.toFixed(2)} {TOKEN_ALLOWLIST.find((tok) => tok.mint === t.mint)?.symbol ?? t.mint.slice(0, 4)}
                  </span>
                ))}
              </p>
            )}
          </div>
        )}
      </section>

      <section className="rounded-lg border border-red-900 bg-red-950/30 p-4">
        <h2 className="mb-2 text-sm font-medium text-red-300">Kill Switch</h2>
        <p className="mb-3 text-xs text-slate-400">Immediately halts all strategy execution. No new simulated trades will occur until resumed.</p>
        <div className="flex items-center gap-3">
          <span className={`rounded-full px-3 py-1 text-sm ${live.paused ? "bg-amber-900 text-amber-300" : "bg-emerald-900 text-emerald-300"}`}>
            {live.paused ? "PAUSED" : "TRADING ACTIVE"}
          </span>
          {live.paused ? (
            <button onClick={() => resume.mutate()} className="rounded bg-emerald-700 px-4 py-2 text-sm font-medium hover:bg-emerald-600">
              Resume Trading
            </button>
          ) : (
            <button onClick={() => pause.mutate()} className="rounded bg-red-700 px-4 py-2 text-sm font-medium hover:bg-red-600">
              Pause All Trading
            </button>
          )}
        </div>
      </section>

      <section className="rounded-lg border border-slate-800 bg-slate-900 p-4">
        <h2 className="mb-3 text-sm font-medium text-slate-400">Risk Limits</h2>
        {current && (
          <form
            className="grid grid-cols-1 gap-3 md:grid-cols-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (current) updateRisk.mutate(current);
            }}
          >
            <RiskField label="Max position size (% of portfolio)" value={current.maxPositionPct} onChange={(v) => setDraft({ ...current, maxPositionPct: v })} />
            <RiskField label="Max daily loss (%) — auto-pause" value={current.maxDailyLossPct} onChange={(v) => setDraft({ ...current, maxDailyLossPct: v })} />
            <RiskField label="Per-token exposure cap (%)" value={current.perTokenExposurePct} onChange={(v) => setDraft({ ...current, perTokenExposurePct: v })} />
            <RiskField label="Mandatory stop-loss (%)" value={current.defaultStopLossPct} onChange={(v) => setDraft({ ...current, defaultStopLossPct: v })} />
            <RiskField label="Max slippage (bps)" value={current.maxSlippageBps} onChange={(v) => setDraft({ ...current, maxSlippageBps: v })} />
            <RiskField label="Max price impact (%)" value={current.maxPriceImpactPct} onChange={(v) => setDraft({ ...current, maxPriceImpactPct: v })} />
            <div className="md:col-span-2">
              <button type="submit" className="rounded bg-emerald-700 px-4 py-2 text-sm font-medium hover:bg-emerald-600">
                Save Risk Limits
              </button>
              {updateRisk.isSuccess && <span className="ml-3 text-sm text-emerald-400">Saved.</span>}
            </div>
          </form>
        )}
      </section>

      <section className="rounded-lg border border-slate-800 bg-slate-900 p-4">
        <h2 className="mb-2 text-sm font-medium text-slate-400">Token Allowlist</h2>
        <ul className="text-sm text-slate-300">
          {TOKEN_ALLOWLIST.map((t) => (
            <li key={t.mint} className="font-mono text-xs text-slate-500">
              {t.symbol} — {t.mint}
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}

function RiskField({ label, value, onChange }: { label: string; value: number; onChange: (v: number) => void }): JSX.Element {
  return (
    <label className="block text-xs text-slate-400">
      {label}
      <input
        type="number"
        step="0.1"
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="mt-1 w-full rounded bg-slate-800 p-2 text-sm text-slate-100"
      />
    </label>
  );
}
