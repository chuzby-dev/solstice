import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { PriceTick } from "@trading-bot/shared";

// Historical OHLCV for the backtest engine only — never used by the live paper-trading
// path (see market/priceFeed.ts for that, which uses Pyth Hermes, not Birdeye).
//
// IMPORTANT: the app's internal SOL mint constant (TOKEN_ALLOWLIST / market/priceFeed.ts's
// TOKEN_METADATA, "So11111111111111111111111111111111111111") is missing its trailing "2"
// versus the real canonical wrapped-SOL mint ("...1111112"). This has stayed silent because
// Pyth (feed-ID based, not mint-based) and the devnet wallet routes never validate Solana
// address format strictly. Birdeye's API does — confirmed live: the truncated address
// returns HTTP 400 "address is invalid format", the corrected one returns real data. Rather
// than change the mint constant everywhere (touches config.ts, .env*, priceFeed.ts, the
// frontend token list, and every test file that hardcodes it — out of scope for a backtest
// feature), CORRECT_MINTS below holds the real address used ONLY for Birdeye HTTP calls;
// ticks returned still carry the app's own (truncated) mint so they line up with
// StrategyConfig.tokenMint the rest of this codebase expects.
const CORRECT_MINTS: Record<string, string> = {
  So11111111111111111111111111111111111111: "So11111111111111111111111111111111111111112",
};

const BIRDEYE_BASE_URL = "https://public-api.birdeye.so/defi/ohlcv";
const CACHE_DIR = join(process.cwd(), "data", "backtest-cache");
const MAX_RECORDS_PER_REQUEST = 1000;
const RATE_LIMIT_RETRY_MS = 5000;
// Empirically the free tier 429s well before 250ms spacing would suggest (observed during
// development); 1.5s between paginated chunks cut retries to near zero.
const REQUEST_SPACING_MS = 1500;

export type BirdeyeInterval = "1m" | "3m" | "5m" | "15m" | "30m" | "1H" | "2H" | "4H" | "6H" | "8H" | "12H" | "1D";

const INTERVAL_SECONDS: Record<BirdeyeInterval, number> = {
  "1m": 60,
  "3m": 180,
  "5m": 300,
  "15m": 900,
  "30m": 1800,
  "1H": 3600,
  "2H": 7200,
  "4H": 14400,
  "6H": 21600,
  "8H": 28800,
  "12H": 43200,
  "1D": 86400,
};

interface BirdeyeCandle {
  o: number;
  h: number;
  l: number;
  c: number;
  v: number;
  unixTime: number;
}

interface BirdeyeOhlcvResponse {
  success: boolean;
  message?: string;
  data?: { items: BirdeyeCandle[] };
}

function requireApiKey(): string {
  const key = process.env.BIRDEYE_API_KEY;
  if (!key) throw new Error("BIRDEYE_API_KEY is not set — add it to .env (see .env.example)");
  return key;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function cachePath(tokenMint: string, interval: BirdeyeInterval, fromSec: number, toSec: number): string {
  return join(CACHE_DIR, `${tokenMint}-${interval}-${fromSec}-${toSec}.json`);
}

async function fetchChunk(queryMint: string, interval: BirdeyeInterval, fromSec: number, toSec: number, retried = false): Promise<BirdeyeCandle[]> {
  const url = `${BIRDEYE_BASE_URL}?address=${queryMint}&address_type=token&type=${interval}&time_from=${fromSec}&time_to=${toSec}`;
  const res = await fetch(url, {
    method: "GET",
    headers: { "X-API-KEY": requireApiKey(), "x-chain": "solana", accept: "application/json" },
  });

  if (res.status === 429) {
    if (retried) throw new Error("[birdeyeClient] rate limited (429) twice in a row, giving up on this chunk");
    console.warn(`[birdeyeClient] rate limited (429) — waiting ${RATE_LIMIT_RETRY_MS / 1000}s and retrying once`);
    await sleep(RATE_LIMIT_RETRY_MS);
    return fetchChunk(queryMint, interval, fromSec, toSec, true);
  }
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`[birdeyeClient] request failed: ${res.status} ${res.statusText} — ${body}`);
  }

  const body = (await res.json()) as BirdeyeOhlcvResponse;
  if (!body.success || !body.data) {
    throw new Error(`[birdeyeClient] Birdeye returned success=false: ${body.message ?? "no message"}`);
  }
  return body.data.items;
}

/** Fetches historical candles for `appTokenMint` (the app's own, possibly-truncated mint
 * constant — see CORRECT_MINTS above) between fromDate/toDate, converts each candle's close
 * price into a PriceTick, and disk-caches the result so repeated backtest/sweep runs don't
 * re-hit Birdeye or its rate limit. Paginates in MAX_RECORDS_PER_REQUEST-sized chunks per
 * Birdeye's documented per-request cap. */
export async function fetchHistoricalTicks(appTokenMint: string, tokenSymbol: string, interval: BirdeyeInterval, fromDate: Date, toDate: Date): Promise<PriceTick[]> {
  mkdirSync(CACHE_DIR, { recursive: true });

  const fromSec = Math.floor(fromDate.getTime() / 1000);
  const toSec = Math.floor(toDate.getTime() / 1000);
  const cacheFile = cachePath(appTokenMint, interval, fromSec, toSec);
  if (existsSync(cacheFile)) {
    return JSON.parse(readFileSync(cacheFile, "utf-8")) as PriceTick[];
  }

  const queryMint = CORRECT_MINTS[appTokenMint] ?? appTokenMint;
  const stepSeconds = INTERVAL_SECONDS[interval] * MAX_RECORDS_PER_REQUEST;
  const candles: BirdeyeCandle[] = [];

  for (let chunkFrom = fromSec; chunkFrom < toSec; chunkFrom += stepSeconds) {
    const chunkTo = Math.min(chunkFrom + stepSeconds, toSec);
    const chunk = await fetchChunk(queryMint, interval, chunkFrom, chunkTo);
    candles.push(...chunk);
    if (chunkTo < toSec) await sleep(REQUEST_SPACING_MS);
  }

  candles.sort((a, b) => a.unixTime - b.unixTime);
  const ticks: PriceTick[] = candles.map((c) => ({
    tokenMint: appTokenMint,
    tokenSymbol,
    priceUsd: c.c,
    timestamp: new Date(c.unixTime * 1000).toISOString(),
  }));

  writeFileSync(cacheFile, JSON.stringify(ticks));
  console.log(`[birdeyeClient] fetched ${ticks.length} ${interval} candles for ${tokenSymbol} (${fromDate.toISOString()} → ${toDate.toISOString()}), cached to ${cacheFile}`);
  return ticks;
}
