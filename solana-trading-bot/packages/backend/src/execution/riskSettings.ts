import { eq } from "drizzle-orm";
import type { RiskLimits } from "@trading-bot/shared";
import { db } from "../db/client.js";
import { riskSettings } from "../db/schema.js";
import { riskHardCeilings } from "../config.js";

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

/** Clamps user-supplied risk settings to the non-negotiable hard ceilings (config.ts).
 * A user can tighten these (lower risk) freely but can never loosen past the ceiling. */
export function clampRiskLimits(input: RiskLimits): RiskLimits {
  return {
    maxPositionPct: clamp(input.maxPositionPct, 0.1, riskHardCeilings.maxPositionPct),
    maxDailyLossPct: clamp(input.maxDailyLossPct, 0.1, riskHardCeilings.maxDailyLossPct),
    perTokenExposurePct: clamp(input.perTokenExposurePct, 0.1, riskHardCeilings.perTokenExposurePct),
    defaultStopLossPct: clamp(input.defaultStopLossPct, 0.1, 90),
    maxSlippageBps: clamp(input.maxSlippageBps, 1, riskHardCeilings.maxSlippageBps),
    maxPriceImpactPct: clamp(input.maxPriceImpactPct, 0.1, riskHardCeilings.maxPriceImpactPct),
  };
}

export function getRiskLimits(): RiskLimits {
  const row = db.select().from(riskSettings).where(eq(riskSettings.id, "singleton")).get();
  if (!row) throw new Error("risk_settings singleton row missing; db not initialized correctly");
  const { id: _id, ...limits } = row;
  return limits;
}

export function setRiskLimits(input: RiskLimits): RiskLimits {
  const clamped = clampRiskLimits(input);
  db.update(riskSettings).set(clamped).where(eq(riskSettings.id, "singleton")).run();
  return clamped;
}
