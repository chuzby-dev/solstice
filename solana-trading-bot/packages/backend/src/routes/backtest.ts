import type { FastifyInstance } from "fastify";
import type { BacktestRunResult, BacktestTuneResult, BuiltInStrategyId } from "@trading-bot/shared";
import { config } from "../config.js";
import { getRiskLimits } from "../execution/riskSettings.js";
import { fetchHistoricalTicks } from "../backtest/birdeyeClient.js";
import { runBacktest } from "../backtest/backtestEngine.js";
import { runSweep, TICK_COUNT_PARAMS } from "../backtest/sweep.js";
import { computeMetrics } from "../backtest/metrics.js";
import { REPLAYABLE_STRATEGIES, getCandleConfig } from "../backtest/replayable.js";

interface RunBody {
  strategyId: BuiltInStrategyId;
  tokenMint: string;
  tokenSymbol: string;
  params: Record<string, number>;
}

interface TuneBody {
  strategyId: BuiltInStrategyId;
  tokenMint: string;
  tokenSymbol: string;
  trials?: number;
}

/** birdeyeClient.ts's disk cache key is exact-second (`{mint}-{interval}-{fromSec}-{toSec}`)
 * — fine for the CLI, where "now" is computed once per process run, but an on-demand route
 * computing `new Date()` fresh on every request would mint a distinct cache key (and refetch
 * from Birdeye, ~30s+ for a paginated fine-grained range) on literally every click. Rounding
 * down to the current UTC hour keeps repeated requests within the same hour on one cache
 * entry — an hour of staleness is immaterial against 14-180 days of history. */
function roundDownToHour(d: Date): Date {
  const rounded = new Date(d);
  rounded.setUTCMinutes(0, 0, 0);
  return rounded;
}

function validateReplayable(strategyId: BuiltInStrategyId, tokenMint: string): string | null {
  if (!REPLAYABLE_STRATEGIES.includes(strategyId)) {
    return `"${strategyId}" can't be backtested from price history alone (see docs/ARCHITECTURE.md "Backtesting")`;
  }
  if (!config.tokenAllowlist.includes(tokenMint)) {
    return `Token ${tokenMint} is not on the allowlist`;
  }
  return null;
}

/** On-demand backtesting for the Strategy Selector UI — runs the real strategy/risk code
 * (see backtest/backtestEngine.ts, backtest/sweep.ts) against real historical price data,
 * fully isolated from the live paper-trading db/priceCache. See docs/ARCHITECTURE.md
 * "Backtesting" for methodology (tuning/validation split, tick-count-param caveat). */
export async function backtestRoutes(app: FastifyInstance): Promise<void> {
  app.post<{ Body: RunBody }>("/api/backtest/run", async (req, reply) => {
    const { strategyId, tokenMint, tokenSymbol, params } = req.body;

    const validationError = validateReplayable(strategyId, tokenMint);
    if (validationError) return reply.status(400).send({ error: validationError });

    try {
      const { interval, days } = getCandleConfig(strategyId);
      const now = roundDownToHour(new Date());
      const ticks = await fetchHistoricalTicks(tokenMint, tokenSymbol, interval, new Date(now.getTime() - days * 86_400_000), now);

      const result = runBacktest(strategyId, params, ticks, getRiskLimits(), config.simulatedStartingCashUsd);
      const response: BacktestRunResult = {
        strategyId,
        tokenSymbol,
        candleInterval: interval,
        candleCount: ticks.length,
        metrics: computeMetrics(result),
      };
      return reply.send(response);
    } catch (err) {
      return reply.status(500).send({ error: err instanceof Error ? err.message : "Backtest failed" });
    }
  });

  app.post<{ Body: TuneBody }>("/api/backtest/tune", async (req, reply) => {
    const { strategyId, tokenMint, tokenSymbol, trials } = req.body;

    const validationError = validateReplayable(strategyId, tokenMint);
    if (validationError) return reply.status(400).send({ error: validationError });

    try {
      const { interval, days } = getCandleConfig(strategyId);
      const now = roundDownToHour(new Date());
      const ticks = await fetchHistoricalTicks(tokenMint, tokenSymbol, interval, new Date(now.getTime() - days * 86_400_000), now);

      // Always hold tick-count period params fixed — see TICK_COUNT_PARAMS: a period
      // tuned against 1H/1m candles is a real-time lookback the live 2s-poll engine
      // usually can't reproduce (ctx.priceHistory tops out at 600 ticks, ~20min), so a
      // tuned period isn't safely appliable to a live config regardless of how well it
      // backtests. This keeps `best.params` always safe to apply directly — no
      // client-side filtering needed. See docs/ARCHITECTURE.md "Backtesting".
      const tickCountParams = TICK_COUNT_PARAMS[strategyId] ?? [];
      const sweep = runSweep(strategyId, ticks, getRiskLimits(), config.simulatedStartingCashUsd, {
        trials: trials ?? 150,
        excludeParams: tickCountParams,
      });

      const toTrial = (t: typeof sweep.baseline) => ({ params: t.params, metrics: t.tuning, validationMetrics: t.validation });
      const response: BacktestTuneResult = {
        strategyId,
        tokenSymbol,
        candleInterval: interval,
        candleCount: ticks.length,
        baseline: toTrial(sweep.baseline),
        best: sweep.best ? toTrial(sweep.best) : null,
        tickCountParams,
      };
      return reply.send(response);
    } catch (err) {
      return reply.status(500).send({ error: err instanceof Error ? err.message : "Auto-tune failed" });
    }
  });
}
