import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { BacktestMetrics, BacktestVerdict, RiskLevel, StrategyConfig } from "@trading-bot/shared";
import { api } from "../lib/api.js";
import { TOKEN_ALLOWLIST } from "../lib/tokens.js";
import { RiskDisclaimer } from "./RiskDisclaimer.js";
import { PriceSparkline } from "./PriceSparkline.js";

const RISK_COLORS: Record<RiskLevel, string> = {
  low: "text-risk-low border-risk-low",
  medium: "text-risk-medium border-risk-medium",
  high: "text-risk-high border-risk-high",
};

const VERDICT_LABELS: Record<BacktestVerdict, string> = {
  profitable: "Profitable",
  "not-profitable": "Not profitable",
  untested: "Untested",
};

const VERDICT_COLORS: Record<BacktestVerdict, string> = {
  profitable: "text-emerald-400 border-emerald-400",
  "not-profitable": "text-red-400 border-red-400",
  untested: "text-slate-400 border-slate-600",
};

const NOT_REPLAYABLE = new Set(["whale-copy"]);

function fmtPct(n: number): string {
  return `${n.toFixed(2)}%`;
}

function fmtRate(n: number | null): string {
  return n === null ? "n/a" : `${(n * 100).toFixed(0)}%`;
}

function MetricsRow({ label, metrics }: { label: string; metrics: BacktestMetrics }): JSX.Element {
  return (
    <div className="text-xs text-slate-400">
      <span className="text-slate-300">{label}:</span> return {fmtPct(metrics.totalReturnPct)} · {metrics.roundTripCount} trades · win{" "}
      {fmtRate(metrics.winRate)} · maxDD {fmtPct(metrics.maxDrawdownPct)} · fees {fmtPct(metrics.feeDragPct)}
    </div>
  );
}

export function StrategySelector(): JSX.Element {
  const queryClient = useQueryClient();
  const catalog = useQuery({ queryKey: ["strategy-catalog"], queryFn: api.getStrategyCatalog });
  const configs = useQuery({ queryKey: ["strategies"], queryFn: api.getStrategies });

  const [creatingFor, setCreatingFor] = useState<string | null>(null);
  const [token, setToken] = useState<(typeof TOKEN_ALLOWLIST)[number]>(TOKEN_ALLOWLIST[0]);
  const [pendingActivation, setPendingActivation] = useState<StrategyConfig | null>(null);
  const [formParams, setFormParams] = useState<Record<string, number> | null>(null);

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ["strategies"] });

  const createMutation = useMutation({
    mutationFn: api.createStrategy,
    onSuccess: invalidate,
  });
  const activateMutation = useMutation({
    mutationFn: ({ id, confirmed }: { id: string; confirmed: boolean }) => api.activateStrategy(id, confirmed),
    onSuccess: invalidate,
  });
  const deactivateMutation = useMutation({ mutationFn: api.deactivateStrategy, onSuccess: invalidate });
  const deleteMutation = useMutation({ mutationFn: api.deleteStrategy, onSuccess: invalidate });
  const backtestMutation = useMutation({ mutationFn: api.runBacktest });
  const tuneMutation = useMutation({ mutationFn: api.tuneStrategy });

  function openConfigFor(strategyId: string, defaultParams: Record<string, number>): void {
    setCreatingFor(strategyId);
    setFormParams({ ...defaultParams });
    backtestMutation.reset();
    tuneMutation.reset();
  }

  function closeConfig(): void {
    setCreatingFor(null);
    setFormParams(null);
    backtestMutation.reset();
    tuneMutation.reset();
  }

  if (catalog.isLoading) return <p className="text-slate-400">Loading strategy catalog…</p>;
  if (catalog.error) return <p className="text-red-400">Failed to load strategy catalog.</p>;

  return (
    <div className="space-y-8">
      <section>
        <h2 className="mb-3 text-lg font-semibold">Built-in Strategy Library</h2>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 items-start">
          {catalog.data?.map((meta) => (
            <div key={meta.id} className={`rounded-lg border bg-slate-900 ${RISK_COLORS[meta.riskLevel]}`}>
              <button
                type="button"
                onClick={() => (creatingFor === meta.id ? closeConfig() : openConfigFor(meta.id, meta.defaultParams))}
                className="flex w-full items-center justify-between gap-2 p-4 text-left"
              >
                <span className="flex flex-wrap items-center gap-2">
                  <span className="font-medium text-slate-100">{meta.name}</span>
                  <span className={`rounded-full border px-2 py-0.5 text-xs uppercase ${RISK_COLORS[meta.riskLevel]}`}>{meta.riskLevel} risk</span>
                  <span className={`rounded-full border px-2 py-0.5 text-xs ${VERDICT_COLORS[meta.backtestVerdict]}`}>
                    {VERDICT_LABELS[meta.backtestVerdict]}
                  </span>
                </span>
                <span className="shrink-0 text-slate-500">{creatingFor === meta.id ? "▲" : "▼"}</span>
              </button>
              {creatingFor === meta.id && formParams && (
                <div className="space-y-2 border-t border-slate-800 p-4 pt-3">
                  <p className="text-sm text-slate-400">{meta.description}</p>
                  <form
                    className="space-y-2"
                    onSubmit={(e) => {
                      e.preventDefault();
                    const formData = new FormData(e.currentTarget);
                    const watchedWalletAddress = meta.id === "whale-copy" ? String(formData.get("watchedWalletAddress") ?? "").trim() : undefined;
                    createMutation.mutate({ strategyId: meta.id, tokenMint: token.mint, tokenSymbol: token.symbol, params: formParams, watchedWalletAddress });
                    closeConfig();
                  }}
                >
                  <label className="block text-xs text-slate-400">
                    Token
                    <select
                      className="mt-1 w-full rounded bg-slate-800 p-1.5 text-sm"
                      value={token.mint}
                      onChange={(e) => setToken(TOKEN_ALLOWLIST.find((t) => t.mint === e.target.value) ?? TOKEN_ALLOWLIST[0])}
                    >
                      {TOKEN_ALLOWLIST.map((t) => (
                        <option key={t.mint} value={t.mint}>
                          {t.symbol}
                        </option>
                      ))}
                    </select>
                  </label>
                  <PriceSparkline
                    tokenMint={token.mint}
                    tokenSymbol={token.symbol}
                    windowMinutes={typeof meta.defaultParams.windowMinutes === "number" ? meta.defaultParams.windowMinutes : 5}
                  />
                  {meta.id === "whale-copy" && (
                    <label className="block text-xs text-slate-400">
                      Watched wallet address (mainnet, read-only)
                      <input
                        type="text"
                        name="watchedWalletAddress"
                        required
                        placeholder="Solana public key to mirror"
                        className="mt-1 w-full rounded bg-slate-800 p-1.5 font-mono text-xs"
                      />
                    </label>
                  )}
                  {Object.entries(meta.defaultParams).map(([key, defaultValue]) => (
                    <label key={key} className="block text-xs text-slate-400">
                      {meta.paramDescriptions[key] ?? key}
                      <input
                        type="number"
                        name={key}
                        value={formParams[key] ?? defaultValue}
                        onChange={(e) => setFormParams((p) => ({ ...(p ?? meta.defaultParams), [key]: Number(e.target.value) }))}
                        className="mt-1 w-full rounded bg-slate-800 p-1.5 text-sm"
                      />
                    </label>
                  ))}
                  <div className="flex flex-wrap gap-2 pt-1">
                    <button type="submit" className="rounded bg-emerald-700 px-3 py-1.5 text-sm hover:bg-emerald-600">
                      Add configuration
                    </button>
                    <button type="button" onClick={closeConfig} className="rounded px-3 py-1.5 text-sm text-slate-400 hover:bg-slate-800">
                      Cancel
                    </button>
                    {!NOT_REPLAYABLE.has(meta.id) && token.symbol !== "USDC" && (
                      <>
                        <button
                          type="button"
                          disabled={backtestMutation.isPending}
                          onClick={() =>
                            backtestMutation.mutate({ strategyId: meta.id, tokenMint: token.mint, tokenSymbol: token.symbol, params: formParams })
                          }
                          className="rounded bg-slate-800 px-3 py-1.5 text-sm hover:bg-slate-700 disabled:opacity-50"
                        >
                          {backtestMutation.isPending && backtestMutation.variables?.strategyId === meta.id ? "Backtesting…" : "Backtest"}
                        </button>
                        <button
                          type="button"
                          disabled={tuneMutation.isPending}
                          onClick={() => tuneMutation.mutate({ strategyId: meta.id, tokenMint: token.mint, tokenSymbol: token.symbol })}
                          className="rounded bg-slate-800 px-3 py-1.5 text-sm hover:bg-slate-700 disabled:opacity-50"
                        >
                          {tuneMutation.isPending && tuneMutation.variables?.strategyId === meta.id ? "Tuning…" : "Auto-tune"}
                        </button>
                      </>
                    )}
                  </div>

                  {(backtestMutation.isPending || tuneMutation.isPending) && (backtestMutation.variables?.strategyId === meta.id || tuneMutation.variables?.strategyId === meta.id) && (
                    <p className="text-xs text-slate-500">Fetching historical data — first run for a strategy/token can take up to a minute, cached after that.</p>
                  )}

                  {backtestMutation.isError && backtestMutation.variables?.strategyId === meta.id && (
                    <p className="text-xs text-red-400">{(backtestMutation.error as Error).message}</p>
                  )}
                  {backtestMutation.data && backtestMutation.variables?.strategyId === meta.id && (
                    <div className="rounded border border-slate-800 bg-slate-950 p-2">
                      <MetricsRow label="Backtest" metrics={backtestMutation.data.metrics} />
                      <p className="mt-1 text-[11px] text-slate-600">
                        {backtestMutation.data.candleCount} {backtestMutation.data.candleInterval} candles
                      </p>
                    </div>
                  )}

                  {tuneMutation.isError && tuneMutation.variables?.strategyId === meta.id && (
                    <p className="text-xs text-red-400">{(tuneMutation.error as Error).message}</p>
                  )}
                  {tuneMutation.data && tuneMutation.variables?.strategyId === meta.id && (
                    <div className="space-y-1 rounded border border-slate-800 bg-slate-950 p-2">
                      <MetricsRow label="Shipped defaults" metrics={tuneMutation.data.baseline.metrics} />
                      {tuneMutation.data.baseline.validationMetrics && (
                        <MetricsRow label="  ↳ validation" metrics={tuneMutation.data.baseline.validationMetrics} />
                      )}
                      {tuneMutation.data.best ? (
                        <>
                          <MetricsRow label="Best found" metrics={tuneMutation.data.best.metrics} />
                          {tuneMutation.data.best.validationMetrics && (
                            <MetricsRow label="  ↳ validation" metrics={tuneMutation.data.best.validationMetrics} />
                          )}
                          {(!tuneMutation.data.best.validationMetrics ||
                            tuneMutation.data.best.validationMetrics.roundTripCount === 0 ||
                            tuneMutation.data.best.validationMetrics.totalReturnPct < 0) && (
                            <p className="text-[11px] text-amber-400">
                              {!tuneMutation.data.best.validationMetrics || tuneMutation.data.best.validationMetrics.roundTripCount === 0
                                ? "⚠ No out-of-sample evidence yet (0 validation trades) — apply with caution."
                                : "⚠ Lost money on the held-out validation window — apply with caution."}
                            </p>
                          )}
                          {tuneMutation.data.tickCountParams.length > 0 && (
                            <p className="text-[11px] text-slate-600">
                              Held fixed (not tunable here — see docs/ARCHITECTURE.md): {tuneMutation.data.tickCountParams.join(", ")}
                            </p>
                          )}
                          <button
                            type="button"
                            onClick={() => setFormParams((p) => ({ ...(p ?? meta.defaultParams), ...tuneMutation.data!.best!.params }))}
                            className="mt-1 rounded bg-emerald-800 px-2 py-1 text-xs hover:bg-emerald-700"
                          >
                            Apply to form
                          </button>
                        </>
                      ) : (
                        <p className="text-[11px] text-slate-500">Not enough trade signal on this data to tune yet.</p>
                      )}
                    </div>
                  )}
                  </form>
                </div>
              )}
            </div>
          ))}
        </div>
      </section>

      <section>
        <h2 className="mb-3 text-lg font-semibold">Configured Strategies</h2>
        {configs.data?.length === 0 && <p className="text-sm text-slate-500">No strategies configured yet.</p>}
        <div className="space-y-2">
          {configs.data?.map((cfg) => (
            <div key={cfg.id} className="flex items-center justify-between rounded border border-slate-800 bg-slate-900 p-3">
              <div>
                <span className="font-medium">{cfg.strategyId}</span>
                <span className="ml-2 text-sm text-slate-400">{cfg.tokenSymbol}</span>
                <span className={`ml-2 rounded-full px-2 py-0.5 text-xs ${cfg.active ? "bg-emerald-900 text-emerald-300" : "bg-slate-800 text-slate-400"}`}>
                  {cfg.active ? "active" : "inactive"}
                </span>
                {cfg.watchedWalletAddress && (
                  <span className="ml-2 font-mono text-xs text-slate-500">
                    watching {cfg.watchedWalletAddress.slice(0, 4)}…{cfg.watchedWalletAddress.slice(-4)}
                  </span>
                )}
              </div>
              <div className="flex gap-2">
                {cfg.active ? (
                  <button onClick={() => deactivateMutation.mutate(cfg.id)} className="rounded bg-slate-800 px-3 py-1 text-sm hover:bg-slate-700">
                    Deactivate
                  </button>
                ) : (
                  <button onClick={() => setPendingActivation(cfg)} className="rounded bg-emerald-700 px-3 py-1 text-sm hover:bg-emerald-600">
                    Activate
                  </button>
                )}
                <button onClick={() => deleteMutation.mutate(cfg.id)} className="rounded px-3 py-1 text-sm text-red-400 hover:bg-red-950">
                  Delete
                </button>
              </div>
            </div>
          ))}
        </div>
      </section>

      {pendingActivation && (
        <RiskDisclaimer
          strategyName={`${pendingActivation.strategyId} · ${pendingActivation.tokenSymbol}`}
          onCancel={() => setPendingActivation(null)}
          onConfirm={() => {
            activateMutation.mutate({ id: pendingActivation.id, confirmed: true });
            setPendingActivation(null);
          }}
        />
      )}
    </div>
  );
}
