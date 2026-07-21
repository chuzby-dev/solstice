import type {
  PerformanceResponse,
  PositionsResponse,
  StatusResponse,
  TradesResponse,
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

export const api = {
  status: () => getJson<StatusResponse>('/status'),
  positions: () => getJson<PositionsResponse>('/positions'),
  trades: () => getJson<TradesResponse>('/trades'),
  performance: () => getJson<PerformanceResponse>('/performance'),
};

export function wsUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}${BASE}/ws`;
}
