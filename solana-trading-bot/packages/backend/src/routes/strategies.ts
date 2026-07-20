import { randomUUID } from "node:crypto";
import { eq } from "drizzle-orm";
import type { FastifyInstance } from "fastify";
import { PublicKey } from "@solana/web3.js";
import type { BuiltInStrategyId } from "@trading-bot/shared";
import { db } from "../db/client.js";
import { strategyConfigs } from "../db/schema.js";
import { strategyMetadata, strategyRegistry } from "../strategy-engine/registry.js";
import { config } from "../config.js";

interface CreateBody {
  strategyId: BuiltInStrategyId;
  tokenMint: string;
  tokenSymbol: string;
  params?: Record<string, number>;
  watchedWalletAddress?: string;
}

interface ActivateBody {
  /** Explicit confirmation of the on-screen risk disclaimer (spec section 2). Activation
   * is rejected without it. */
  confirmed: boolean;
}

function toApiShape(row: typeof strategyConfigs.$inferSelect) {
  return {
    id: row.id,
    strategyId: row.strategyId,
    tokenMint: row.tokenMint,
    tokenSymbol: row.tokenSymbol,
    params: JSON.parse(row.params) as Record<string, number>,
    watchedWalletAddress: row.watchedWalletAddress ?? undefined,
    active: row.active,
    createdAt: row.createdAt,
  };
}

export async function strategyRoutes(app: FastifyInstance): Promise<void> {
  app.get("/api/strategies/catalog", async () => Object.values(strategyMetadata));

  app.get("/api/strategies", async () => db.select().from(strategyConfigs).all().map(toApiShape));

  app.post<{ Body: CreateBody }>("/api/strategies", async (req, reply) => {
    const { strategyId, tokenMint, tokenSymbol, params, watchedWalletAddress } = req.body;

    if (!strategyRegistry[strategyId]) {
      return reply.status(400).send({ error: `Unknown strategyId '${strategyId}'` });
    }
    if (!config.tokenAllowlist.includes(tokenMint)) {
      return reply.status(400).send({ error: `Token ${tokenMint} is not on the allowlist` });
    }

    let validatedWalletAddress: string | null = null;
    if (strategyId === "whale-copy") {
      if (!watchedWalletAddress) {
        return reply.status(400).send({ error: "whale-copy requires a watchedWalletAddress" });
      }
      try {
        validatedWalletAddress = new PublicKey(watchedWalletAddress).toBase58();
      } catch {
        return reply.status(400).send({ error: "watchedWalletAddress is not a valid Solana public key" });
      }
    }

    const metadata = strategyMetadata[strategyId];
    const row = {
      id: randomUUID(),
      strategyId,
      tokenMint,
      tokenSymbol,
      params: JSON.stringify({ ...metadata.defaultParams, ...params }),
      watchedWalletAddress: validatedWalletAddress,
      active: false,
      createdAt: new Date().toISOString(),
    };
    db.insert(strategyConfigs).values(row).run();
    return reply.status(201).send(toApiShape(row));
  });

  app.post<{ Params: { id: string }; Body: ActivateBody }>("/api/strategies/:id/activate", async (req, reply) => {
    if (!req.body?.confirmed) {
      return reply.status(400).send({ error: "Strategy activation requires explicit risk-disclaimer confirmation (confirmed: true)" });
    }
    const existing = db.select().from(strategyConfigs).where(eq(strategyConfigs.id, req.params.id)).get();
    if (!existing) return reply.status(404).send({ error: "Strategy config not found" });

    db.update(strategyConfigs).set({ active: true }).where(eq(strategyConfigs.id, req.params.id)).run();
    return reply.send({ ok: true });
  });

  app.post<{ Params: { id: string } }>("/api/strategies/:id/deactivate", async (req, reply) => {
    const existing = db.select().from(strategyConfigs).where(eq(strategyConfigs.id, req.params.id)).get();
    if (!existing) return reply.status(404).send({ error: "Strategy config not found" });

    db.update(strategyConfigs).set({ active: false }).where(eq(strategyConfigs.id, req.params.id)).run();
    return reply.send({ ok: true });
  });

  app.delete<{ Params: { id: string } }>("/api/strategies/:id", async (req, reply) => {
    db.delete(strategyConfigs).where(eq(strategyConfigs.id, req.params.id)).run();
    return reply.status(204).send();
  });
}
