// One-off: fetch JTO 1m candles to test Birdeye quota status and cache the data for the
// SOL/JTO pair-trading raw-signal analysis. Safe to delete after use.
import "../src/config.js";
import { fetchHistoricalTicks } from "../src/backtest/birdeyeClient.js";

const JTO_MINT = "jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL";
const now = new Date();
const from = new Date(now.getTime() - 45 * 86_400_000);

const ticks = await fetchHistoricalTicks(JTO_MINT, "JTO", "1m", from, now);
console.log(`fetched ${ticks.length} JTO candles, first=${ticks[0]?.timestamp} last=${ticks[ticks.length - 1]?.timestamp}`);
