import type { AppModeState, AutoSweepConfig, BacktestRunResult, BacktestTuneResult, HotWalletKeyExport, HotWalletStatus, PortfolioSnapshot, PriceTick, RiskLimits, StrategyConfig, StrategyMetadata, Trade, TradingMode, TransferPreview, WalletSendResult, WalletTransaction } from "@trading-bot/shared";

/** Thrown on any non-2xx response. Carries the full parsed error body (not just the
 * message) so callers that need extra fields — e.g. wallet sends' `requiresAcknowledgement`/
 * `usdValue` on the large-send gate — can read them without a bespoke request path. */
export interface ApiError extends Error {
  requiresAcknowledgement?: boolean;
  usdValue?: number;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`/api${path}`, {
    ...init,
    // Only send a JSON Content-Type when there's actually a body — Fastify's JSON
    // parser rejects an empty body sent with 'application/json' (FST_ERR_CTP_EMPTY_JSON_BODY).
    headers: init?.body ? { "Content-Type": "application/json", ...init?.headers } : init?.headers,
  });
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    const error = new Error(body.error ?? `Request to ${path} failed with ${res.status}`) as ApiError;
    Object.assign(error, body);
    throw error;
  }
  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}

export interface CreateStrategyInput {
  strategyId: string;
  tokenMint: string;
  tokenSymbol: string;
  params?: Record<string, number>;
  watchedWalletAddress?: string;
}

export interface BacktestRunInput {
  strategyId: string;
  tokenMint: string;
  tokenSymbol: string;
  params: Record<string, number>;
}

export interface BacktestTuneInput {
  strategyId: string;
  tokenMint: string;
  tokenSymbol: string;
  trials?: number;
}

export const api = {
  getStrategyCatalog: () => request<StrategyMetadata[]>("/strategies/catalog"),
  getStrategies: () => request<StrategyConfig[]>("/strategies"),
  createStrategy: (input: CreateStrategyInput) => request<StrategyConfig>("/strategies", { method: "POST", body: JSON.stringify(input) }),
  runBacktest: (input: BacktestRunInput) => request<BacktestRunResult>("/backtest/run", { method: "POST", body: JSON.stringify(input) }),
  tuneStrategy: (input: BacktestTuneInput) => request<BacktestTuneResult>("/backtest/tune", { method: "POST", body: JSON.stringify(input) }),
  activateStrategy: (id: string, confirmed: boolean) =>
    request<{ ok: true }>(`/strategies/${id}/activate`, { method: "POST", body: JSON.stringify({ confirmed }) }),
  deactivateStrategy: (id: string) => request<{ ok: true }>(`/strategies/${id}/deactivate`, { method: "POST" }),
  deleteStrategy: (id: string) => request<void>(`/strategies/${id}`, { method: "DELETE" }),

  getTrades: (params?: { strategyConfigId?: string; tokenMint?: string; limit?: number }) => {
    const qs = new URLSearchParams();
    if (params?.strategyConfigId) qs.set("strategyConfigId", params.strategyConfigId);
    if (params?.tokenMint) qs.set("tokenMint", params.tokenMint);
    if (params?.limit) qs.set("limit", String(params.limit));
    const suffix = qs.toString() ? `?${qs.toString()}` : "";
    return request<Trade[]>(`/trades${suffix}`);
  },

  getPortfolio: () => request<PortfolioSnapshot>("/portfolio"),
  getRiskSettings: () => request<RiskLimits>("/risk-settings"),
  updateRiskSettings: (limits: RiskLimits) => request<RiskLimits>("/risk-settings", { method: "PUT", body: JSON.stringify(limits) }),

  getKillswitch: () => request<{ paused: boolean }>("/killswitch"),
  pause: () => request<{ paused: boolean }>("/killswitch/pause", { method: "POST" }),
  resume: () => request<{ paused: boolean }>("/killswitch/resume", { method: "POST" }),

  getWalletBalance: (pubkey: string) =>
    request<{ pubkey: string; network: string; solBalance: number; tokenBalances: { mint: string; amount: number }[] }>(`/wallet/${pubkey}/balance`),

  getHotWalletStatus: () => request<HotWalletStatus>("/wallet/hot/status"),
  createHotWallet: () => request<HotWalletStatus>("/wallet/hot/create", { method: "POST" }),
  sendFromWallet: (input: { tokenMint: string; amount: number; destination: string; acknowledgedLargeSend?: boolean }) =>
    request<WalletSendResult>("/wallet/send", { method: "POST", body: JSON.stringify(input) }),
  previewSend: (input: { tokenMint: string; amount: number; destination: string }) =>
    request<TransferPreview>("/wallet/send/preview", { method: "POST", body: JSON.stringify(input) }),
  // Network-aware — reflects whichever network is currently selected (see /api/mode),
  // unlike getWalletBalance above which is always devnet (the unrelated external-wallet
  // view-only connect).
  getHotWalletBalance: () =>
    request<{ pubkey: string; network: string; solBalance: number; tokenBalances: { mint: string; amount: number }[] }>("/wallet/hot/balance"),
  getWalletHistory: () => request<WalletTransaction[]>("/wallet/history"),
  exportHotWalletKey: () => request<HotWalletKeyExport>("/wallet/hot/export-key", { method: "POST", body: JSON.stringify({ confirmed: true }) }),
  getAutoSweep: () => request<AutoSweepConfig>("/wallet/auto-sweep"),
  setAutoSweep: (input: AutoSweepConfig) => request<AutoSweepConfig>("/wallet/auto-sweep", { method: "PUT", body: JSON.stringify(input) }),

  getLivePrices: () => request<PriceTick[]>("/market/prices"),
  getPriceHistory: (tokenMint: string, minutes = 5) => request<PriceTick[]>(`/market/${tokenMint}/history?minutes=${minutes}`),

  getMode: () => request<AppModeState>("/mode"),
  // `network` is never sent — the backend derives it entirely from tradingMode (Paper
  // always means Devnet, Live always means Mainnet; see execution/appMode.ts). There is
  // no way to request any other combination.
  setMode: (tradingMode: TradingMode) => request<AppModeState>("/mode", { method: "PUT", body: JSON.stringify({ tradingMode }) }),
};
