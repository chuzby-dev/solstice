import Fastify from "fastify";
import cors from "@fastify/cors";
import websocketPlugin from "@fastify/websocket";
import { config } from "./config.js";
import "./db/client.js"; // ensures tables exist / seed rows inserted before routes run
import { walletRoutes } from "./wallet/walletRoutes.js";
import { strategyRoutes } from "./routes/strategies.js";
import { tradeRoutes } from "./routes/trades.js";
import { portfolioRoutes } from "./routes/portfolio.js";
import { killswitchRoutes } from "./routes/killswitch.js";
import { marketRoutes } from "./routes/market.js";
import { backtestRoutes } from "./routes/backtest.js";
import { modeRoutes } from "./routes/mode.js";
import { getAppMode } from "./execution/appMode.js";
import { registerWsHub, broadcast } from "./ws/hub.js";
import { startEngine } from "./strategy-engine/engine.js";

async function main(): Promise<void> {
  const app = Fastify({ logger: true });

  await app.register(cors, { origin: true });
  await app.register(websocketPlugin);

  await app.register(registerWsHub);
  await app.register(walletRoutes);
  await app.register(strategyRoutes);
  await app.register(tradeRoutes);
  await app.register(portfolioRoutes);
  await app.register(killswitchRoutes);
  await app.register(marketRoutes);
  await app.register(backtestRoutes);
  await app.register(modeRoutes);

  // Was hardcoded to { mode: "paper-trading", network: "devnet" } — misleading once
  // tradingMode/network became a real, user-switchable toggle (see routes/mode.ts):
  // whoever checks this endpoint deserves the real current mode, not a permanently-stale
  // "everything's paper" answer.
  app.get("/api/health", async () => ({ ok: true, ...getAppMode() }));

  await app.listen({ port: config.port, host: "0.0.0.0" });

  startEngine(broadcast);

  const shutdown = async (): Promise<void> => {
    await app.close();
    process.exit(0);
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

main().catch((err) => {
  console.error("Fatal error starting server:", err);
  process.exit(1);
});
