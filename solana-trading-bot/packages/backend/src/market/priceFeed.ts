import type { PriceTick } from "@trading-bot/shared";
import { config } from "../config.js";
import { priceCache } from "./priceCache.js";

// Read-only: this module only ever performs GET requests against Pyth Network's public
// Hermes price API. It never constructs, signs, or sends a transaction.
//
// History: originally targeted Jupiter's Price API (needs a key for reliable SOL
// pricing), then CoinGecko's free `simple/price` endpoint (no key, but this sandbox's
// shared egress IP got rate-limited into a burst-then-70s-blackout pattern — visibly
// slower than any real wallet's price display, confirmed via live testing). Pyth's
// Hermes API is the actual right fit: it's the on-chain price oracle used throughout
// Solana DeFi (named directly in the original spec) and free with no API key.
//
// Hermes' public tier enforces 10 requests per 10s, with a 60s block on the offending
// client if exceeded. Every poll here already fetches all allowlisted tokens in ONE
// request (see idsQuery below), so at the default 2s interval this module makes ~5
// requests per 10s on its own — under the limit even before accounting for the
// possibility that a shared sandbox egress IP pushes the aggregate higher (the same
// failure mode that broke CoinGecko). BLOCK_DURATION_MS below makes a 429 something
// this module respects and backs off from for the full ban window, rather than
// hammering Hermes every poll interval and extending its own block indefinitely.

const TOKEN_METADATA: Record<string, { symbol: string; pythFeedId: string }> = {
  // Feed IDs from Pyth's Hermes price-feed directory (Crypto.SOL/USD, Crypto.USDC/USD).
  So11111111111111111111111111111111111111: { symbol: "SOL", pythFeedId: "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d" },
  EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v: { symbol: "USDC", pythFeedId: "eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a" },
};

/** Exported (see bottom of file) for wallet/walletRoutes.ts (transaction history
 * persistence needs a mint->symbol lookup outside of a live price tick) — single source
 * of truth for the mint/symbol mapping rather than a second copy. */
function symbolFor(mint: string): string {
  return TOKEN_METADATA[mint]?.symbol ?? `${mint.slice(0, 4)}…${mint.slice(-4)}`;
}

interface PythLatestPriceResponse {
  parsed?: {
    id: string;
    price: { price: string; expo: number; publish_time: number };
  }[];
}

type TickListener = (tick: PriceTick) => void;

const RATE_LIMIT_BLOCK_MS = 60_000; // Hermes' stated ban duration after exceeding 10 req/10s

class PriceFeed {
  private listeners = new Set<TickListener>();
  private timer: ReturnType<typeof setInterval> | null = null;
  private blockedUntil = 0;

  subscribe(listener: TickListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  start(): void {
    if (this.timer) return;
    void this.pollOnce();
    this.timer = setInterval(() => void this.pollOnce(), config.pricePollIntervalMs);
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    this.blockedUntil = 0;
  }

  private async pollOnce(): Promise<void> {
    const trackedMints = config.tokenAllowlist.filter((mint) => TOKEN_METADATA[mint]);
    if (trackedMints.length === 0) return;

    // Respect an active rate-limit block: skip the request entirely rather than retry
    // every poll interval, which would just keep re-triggering the 60s ban.
    if (Date.now() < this.blockedUntil) return;

    const idsQuery = trackedMints.map((mint) => `ids[]=${TOKEN_METADATA[mint]!.pythFeedId}`).join("&");
    const url = `${config.priceApiUrl}?${idsQuery}`;

    try {
      const res = await fetch(url, { method: "GET" });
      if (res.status === 429) {
        this.blockedUntil = Date.now() + RATE_LIMIT_BLOCK_MS;
        console.warn(`[priceFeed] rate limited (429) — backing off for ${RATE_LIMIT_BLOCK_MS / 1000}s`);
        return;
      }
      if (!res.ok) {
        console.warn(`[priceFeed] price fetch failed: ${res.status} ${res.statusText}`);
        return;
      }
      const body = (await res.json()) as PythLatestPriceResponse;
      const byFeedId = new Map((body.parsed ?? []).map((entry) => [entry.id, entry]));

      for (const mint of trackedMints) {
        const meta = TOKEN_METADATA[mint]!;
        const entry = byFeedId.get(meta.pythFeedId);
        if (!entry) continue;
        const priceUsd = Number(entry.price.price) * 10 ** entry.price.expo;
        if (!Number.isFinite(priceUsd)) continue;
        const tick: PriceTick = { tokenMint: mint, tokenSymbol: meta.symbol, priceUsd, timestamp: new Date(entry.price.publish_time * 1000).toISOString() };
        priceCache.push(tick);
        for (const listener of this.listeners) listener(tick);
      }
    } catch (err) {
      console.warn("[priceFeed] poll error:", err instanceof Error ? err.message : err);
    }
  }
}

export const priceFeed = new PriceFeed();
export { symbolFor };
