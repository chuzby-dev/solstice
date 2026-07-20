import type { FastifyInstance } from "fastify";
import type { TradingMode } from "@trading-bot/shared";
import { getAppMode, setAppMode } from "../execution/appMode.js";

export async function modeRoutes(app: FastifyInstance): Promise<void> {
  app.get("/api/mode", async () => getAppMode());

  // `network` is never accepted here — it's derived entirely from `tradingMode` (see
  // execution/appMode.ts). Paper is always Devnet, Live is always Mainnet; there is no
  // way to request any other combination.
  app.put<{ Body: { tradingMode: TradingMode } }>("/api/mode", async (req, reply) => {
    const { tradingMode } = req.body;

    if (tradingMode !== "paper" && tradingMode !== "live") {
      return reply.status(400).send({ error: `Invalid tradingMode "${tradingMode}"` });
    }

    const result = setAppMode(tradingMode);
    return reply.send(result);
  });
}
