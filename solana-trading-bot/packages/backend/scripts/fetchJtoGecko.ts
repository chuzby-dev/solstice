// One-off: fetch JTO/USD 1m candles from GeckoTerminal (free, no API key — Birdeye's quota
// is exhausted for ~11 days) via the most liquid JTO pool (JTO/JitoSOL on Orca, $1.49M
// reserve), paginating backward with before_timestamp. Caches to a plain JSON tick array
// compatible with the existing PriceTick shape so it can be compared against the SOL cache.
// Safe to delete after use.
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const POOL = "G2FiE1yn9N9ZJx5e1E2LxxMnHvb1H3hCuHLPfKJ98smA"; // JTO/JitoSOL, Orca
const DAYS = 45;
const CACHE_DIR = join(process.cwd(), "data", "backtest-cache");
mkdirSync(CACHE_DIR, { recursive: true });

interface Candle { t: number; c: number } // unix seconds, close price USD

async function fetchPage(beforeTs: number | undefined, retried = false): Promise<number[][]> {
  const url = new URL(`https://api.geckoterminal.com/api/v2/networks/solana/pools/${POOL}/ohlcv/minute`);
  url.searchParams.set("aggregate", "1");
  url.searchParams.set("limit", "1000");
  url.searchParams.set("currency", "usd");
  if (beforeTs) url.searchParams.set("before_timestamp", String(beforeTs));
  const res = await fetch(url, { headers: { Accept: "application/json" } });
  if (res.status === 429) {
    if (retried) throw new Error("rate limited twice in a row, giving up");
    console.log("  429 — backing off 65s...");
    await sleep(65_000);
    return fetchPage(beforeTs, true);
  }
  if (!res.ok) throw new Error(`GeckoTerminal ${res.status}: ${await res.text()}`);
  const body = (await res.json()) as { data: { attributes: { ohlcv_list: number[][] } } };
  return body.data.attributes.ohlcv_list; // [timestamp, open, high, low, close, volume], newest first
}

function sleep(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

async function main() {
  const cutoff = Math.floor(Date.now() / 1000) - DAYS * 86400;
  const all: Candle[] = [];
  let before: number | undefined;
  let page = 0;
  for (;;) {
    page++;
    const rows = await fetchPage(before);
    if (rows.length === 0) break;
    for (const r of rows) all.push({ t: r[0]!, c: r[4]! });
    const oldest = rows[rows.length - 1]![0]!;
    console.log(`page ${page}: ${rows.length} candles, oldest=${new Date(oldest * 1000).toISOString()}`);
    if (oldest <= cutoff) break;
    before = oldest;
    await sleep(6000); // documented 30/min limit is apparently not the real ceiling; go slower
  }
  // ascending order, dedup, trim to cutoff
  const seen = new Set<number>();
  const ticks = all
    .filter((c) => c.t >= cutoff)
    .filter((c) => (seen.has(c.t) ? false : (seen.add(c.t), true)))
    .sort((a, b) => a.t - b.t)
    .map((c) => ({ tokenMint: "jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL", tokenSymbol: "JTO", priceUsd: c.c, timestamp: new Date(c.t * 1000).toISOString() }));

  const outFile = join(CACHE_DIR, "JTO-gecko-1m.json");
  writeFileSync(outFile, JSON.stringify(ticks));
  console.log(`wrote ${ticks.length} JTO ticks to ${outFile}`);
  console.log(`span: ${ticks[0]?.timestamp} -> ${ticks[ticks.length - 1]?.timestamp}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
