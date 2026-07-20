import { desc, eq, and, type SQL } from "drizzle-orm";
import type { FastifyInstance } from "fastify";
import { db } from "../db/client.js";
import { trades } from "../db/schema.js";

interface TradeQuery {
  strategyConfigId?: string;
  tokenMint?: string;
  limit?: string;
}

export async function tradeRoutes(app: FastifyInstance): Promise<void> {
  app.get<{ Querystring: TradeQuery }>("/api/trades", async (req) => {
    const conditions: SQL[] = [];
    if (req.query.strategyConfigId) conditions.push(eq(trades.strategyConfigId, req.query.strategyConfigId));
    if (req.query.tokenMint) conditions.push(eq(trades.tokenMint, req.query.tokenMint));

    const limit = Math.min(Number(req.query.limit ?? 100), 500);

    const base = db.select().from(trades);
    const filtered = conditions.length > 0 ? base.where(and(...conditions)) : base;
    return filtered.orderBy(desc(trades.timestamp)).limit(limit).all();
  });
}
