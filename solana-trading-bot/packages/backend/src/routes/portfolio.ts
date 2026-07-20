import type { FastifyInstance } from "fastify";
import type { RiskLimits } from "@trading-bot/shared";
import { getPortfolioSnapshot } from "../execution/simulator.js";
import { getRiskLimits, setRiskLimits } from "../execution/riskSettings.js";

export async function portfolioRoutes(app: FastifyInstance): Promise<void> {
  app.get("/api/portfolio", async () => getPortfolioSnapshot());

  app.get("/api/risk-settings", async () => getRiskLimits());

  app.put<{ Body: RiskLimits }>("/api/risk-settings", async (req) => setRiskLimits(req.body));
}
