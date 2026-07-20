import type { FastifyInstance } from "fastify";
import { priceCache } from "../market/priceCache.js";
import { config } from "../config.js";

const MAX_HISTORY_MINUTES = 60;

interface HistoryQuery {
  minutes?: string;
}

export async function marketRoutes(app: FastifyInstance): Promise<void> {
  // Live price snapshot for every allowlisted token — lets the GUI show a current
  // price next to any form that needs the user to set a price-based limit (e.g. a
  // strategy's absolute price bounds), without waiting for the next WebSocket tick.
  app.get("/api/market/prices", async () => {
    return config.tokenAllowlist.map((mint) => priceCache.latest(mint)).filter((tick) => !!tick);
  });

  // Recent price history for a single token, used for the short-window sparkline in
  // the Strategy Selector (and reusable for a future backtest chart).
  app.get<{ Params: { tokenMint: string }; Querystring: HistoryQuery }>("/api/market/:tokenMint/history", async (req, reply) => {
    const { tokenMint } = req.params;
    if (!config.tokenAllowlist.includes(tokenMint)) {
      return reply.status(400).send({ error: `Token ${tokenMint} is not on the allowlist` });
    }
    const minutes = Math.min(Math.max(Number(req.query.minutes ?? 5), 1), MAX_HISTORY_MINUTES);
    return priceCache.recentWithinMs(tokenMint, minutes * 60_000);
  });
}
