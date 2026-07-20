import type { RiskCheckResult, RiskLimits } from "@trading-bot/shared";

/** Smallest trade size (USD) worth executing after risk caps shrink a signal. */
export const MIN_TRADE_USD = 1;

export interface RiskEvaluationInput {
  action: "buy" | "sell";
  requestedSizeUsd: number;
  priceUsd: number;
  totalPortfolioValueUsd: number;
  cashUsd: number;
  /** Current USD value already held in this token (before this trade). */
  currentTokenExposureUsd: number;
  /** Current quantity of this token held (needed to cap sell size). */
  currentPositionQuantity: number;
  /** USD lost so far today (positive number = a loss). */
  dailyLossUsd: number;
  startOfDayValueUsd: number;
  limits: RiskLimits;
  /** Phase-1 simplification: Jupiter's Price API doesn't expose order-book depth, so
   * price impact is modeled as a fixed assumed-liquidity figure rather than real
   * route data. Swap this for Jupiter quote `priceImpactPct` once live swaps land. */
  assumedLiquidityUsd: number;
  /** Phase-1 simplification: a fixed assumed slippage, since no real swap is quoted. */
  simulatedSlippageBps: number;
}

/** Pure, side-effect-free risk evaluation. Every non-negotiable guard from spec section 7
 * is enforced here: max position size, daily loss auto-pause, per-token exposure cap,
 * and slippage/price-impact ceilings. This runs before every simulated (and, in a later
 * phase, every real) trade. */
export function evaluateSignal(input: RiskEvaluationInput): RiskCheckResult {
  const { limits } = input;

  if (input.action === "buy") {
    const dailyLossLimitUsd = (input.startOfDayValueUsd * limits.maxDailyLossPct) / 100;
    if (input.dailyLossUsd >= dailyLossLimitUsd) {
      return {
        allowed: false,
        reason: `Daily loss limit reached ($${input.dailyLossUsd.toFixed(2)} >= $${dailyLossLimitUsd.toFixed(2)}); trading paused for the day`,
        triggeredGuard: "maxDailyLossPct",
      };
    }

    let size = input.requestedSizeUsd;

    const maxPositionUsd = (input.totalPortfolioValueUsd * limits.maxPositionPct) / 100;
    size = Math.min(size, maxPositionUsd);

    const maxTokenExposureUsd = (input.totalPortfolioValueUsd * limits.perTokenExposurePct) / 100;
    const allowedAdditionalExposure = maxTokenExposureUsd - input.currentTokenExposureUsd;
    if (allowedAdditionalExposure <= 0) {
      return {
        allowed: false,
        reason: `Per-token exposure cap reached (${limits.perTokenExposurePct}% of portfolio)`,
        triggeredGuard: "perTokenExposurePct",
      };
    }
    size = Math.min(size, allowedAdditionalExposure);

    const maxImpactSizeUsd = (limits.maxPriceImpactPct / 100) * input.assumedLiquidityUsd;
    size = Math.min(size, maxImpactSizeUsd);

    if (input.simulatedSlippageBps > limits.maxSlippageBps) {
      return {
        allowed: false,
        reason: `Estimated slippage ${input.simulatedSlippageBps}bps exceeds ceiling of ${limits.maxSlippageBps}bps`,
        triggeredGuard: "maxSlippageBps",
      };
    }

    size = Math.min(size, input.cashUsd);

    if (size < MIN_TRADE_USD) {
      return {
        allowed: false,
        reason: "Trade size fell below the $1 minimum after applying risk caps",
        triggeredGuard: "insufficient_balance",
      };
    }

    return {
      allowed: true,
      adjustedSizeUsd: size,
      reason: size < input.requestedSizeUsd ? `Size reduced from $${input.requestedSizeUsd.toFixed(2)} to $${size.toFixed(2)} by risk caps` : undefined,
    };
  }

  // sell
  const maxSellUsd = input.currentPositionQuantity * input.priceUsd;
  const size = Math.min(input.requestedSizeUsd, maxSellUsd);

  if (size < MIN_TRADE_USD) {
    return {
      allowed: false,
      reason: "No sufficient position to sell",
      triggeredGuard: "insufficient_balance",
    };
  }

  if (input.simulatedSlippageBps > limits.maxSlippageBps) {
    return {
      allowed: false,
      reason: `Estimated slippage ${input.simulatedSlippageBps}bps exceeds ceiling of ${limits.maxSlippageBps}bps`,
      triggeredGuard: "maxSlippageBps",
    };
  }

  return {
    allowed: true,
    adjustedSizeUsd: size,
    reason: size < input.requestedSizeUsd ? `Size reduced from $${input.requestedSizeUsd.toFixed(2)} to $${size.toFixed(2)} (capped to position size)` : undefined,
  };
}

/** The mandatory protective stop-loss price for a newly opened position (spec section 7:
 * "Mandatory stop-loss on every position"). Applied uniformly regardless of which strategy
 * opened the position. */
export function computeStopLossPrice(entryPriceUsd: number, limits: RiskLimits): number {
  return entryPriceUsd * (1 - limits.defaultStopLossPct / 100);
}
