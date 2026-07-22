import type {
  ConvertRequest,
  ConvertResponse,
  DevnetBalanceResponse,
  LiveStatusResponse,
  PerformanceResponse,
  PositionsResponse,
  StatusResponse,
  TradesResponse,
  WalletResponse,
} from './types';

// Vite's dev proxy (vite.config.ts) forwards /api to the solstice-api
// server at 127.0.0.1:8080, so a relative path works in both dev and a
// same-origin production deployment.
const BASE = '/api/v1';

async function getJson<T>(path: string): Promise<T> {
  const response = await fetch(`${BASE}${path}`);
  if (!response.ok) {
    throw new Error(`${path} failed: ${response.status} ${response.statusText}`);
  }
  return response.json() as Promise<T>;
}

/** `null` means "not configured" (404) — not an error to surface. */
async function getOptional<T>(path: string): Promise<T | null> {
  const response = await fetch(`${BASE}${path}`);
  if (response.status === 404) return null;
  if (!response.ok) {
    throw new Error(`${path} failed: ${response.status} ${response.statusText}`);
  }
  return response.json() as Promise<T>;
}

async function postJson<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(`${BASE}${path}`, {
    method: 'POST',
    headers: body ? { 'Content-Type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!response.ok) {
    const text = await response.text().catch(() => '');
    throw new Error(`${path} failed: ${response.status} ${response.statusText} ${text}`);
  }
  return response.json() as Promise<T>;
}

export const api = {
  status: () => getJson<StatusResponse>('/status'),
  positions: () => getJson<PositionsResponse>('/positions'),
  trades: () => getJson<TradesResponse>('/trades'),
  performance: () => getJson<PerformanceResponse>('/performance'),
  wallet: () => getOptional<WalletResponse>('/wallet'),
  walletDevnet: () => getOptional<DevnetBalanceResponse>('/wallet/devnet'),
  walletConvert: (request: ConvertRequest) =>
    postJson<ConvertResponse>('/wallet/convert', request),
  liveStatus: () => getOptional<LiveStatusResponse>('/live/status'),
  liveEnable: () => postJson<LiveStatusResponse>('/live/enable'),
  liveDisable: () => postJson<LiveStatusResponse>('/live/disable'),
  liveSetMaxCapital: (max_capital_usd: number) =>
    postJson<LiveStatusResponse>('/live/config', { max_capital_usd }),
  liveSetMinConfidence: (min_confidence: number) =>
    postJson<LiveStatusResponse>('/live/config', { min_confidence }),
  liveSetStrategiesEnabled: (strategies_enabled: boolean) =>
    postJson<LiveStatusResponse>('/live/config', { strategies_enabled }),
  liveSetTakeProfitPercent: (take_profit_percent: number) =>
    postJson<LiveStatusResponse>('/live/config', { take_profit_percent }),
  liveSetCrossDexArbEnabled: (cross_dex_arb_enabled: boolean) =>
    postJson<LiveStatusResponse>('/live/config', { cross_dex_arb_enabled }),
  liveSetCrossDexMinSpread: (cross_dex_min_spread: number) =>
    postJson<LiveStatusResponse>('/live/config', { cross_dex_min_spread }),
  liveSetCrossDexMaxSlippageBps: (cross_dex_max_slippage_bps: number) =>
    postJson<LiveStatusResponse>('/live/config', { cross_dex_max_slippage_bps }),
  liveSetCrossDexMinNetEdgeBps: (cross_dex_min_net_edge_bps: number) =>
    postJson<LiveStatusResponse>('/live/config', { cross_dex_min_net_edge_bps }),
};

export function wsUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}${BASE}/ws`;
}

export function liveWsUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}${BASE}/live/ws`;
}
