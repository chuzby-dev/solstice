import { parseArgs } from "node:util";
import type { BuiltInStrategyId, RiskLimits } from "@trading-bot/shared";
import { config, riskDefaults } from "../src/config.js";
import { fetchHistoricalTicks } from "../src/backtest/birdeyeClient.js";
import { runSweep, TICK_COUNT_PARAMS } from "../src/backtest/sweep.js";
import { formatSweepConsole, writeCombinedReport } from "../src/backtest/report.js";
import type { SweepResult } from "../src/backtest/sweep.js";
import { REPLAYABLE_STRATEGIES, getCandleConfig } from "../src/backtest/replayable.js";

// The app's own SOL mint constant (see birdeyeClient.ts's header comment for why it's
// missing a trailing "2" vs. the real address, and how that's handled).
const SOL_MINT = "So11111111111111111111111111111111111111";
const SOL_SYMBOL = "SOL";

function parseCliArgs() {
  const { values } = parseArgs({
    options: {
      strategy: { type: "string", default: "all" },
      trials: { type: "string", default: "150" },
      "fine-days": { type: "string", default: "14" },
      "coarse-days": { type: "string", default: "180" },
      "hold-periods": { type: "boolean", default: false },
    },
  });
  return {
    strategy: values.strategy as string,
    trials: Number(values.trials),
    fineDays: Number(values["fine-days"]),
    coarseDays: Number(values["coarse-days"]),
    holdPeriods: values["hold-periods"] as boolean,
  };
}

async function main(): Promise<void> {
  const args = parseCliArgs();
  const limits: RiskLimits = riskDefaults;
  const now = new Date();

  const requested: BuiltInStrategyId[] = args.strategy === "all" ? REPLAYABLE_STRATEGIES : [args.strategy as BuiltInStrategyId];
  const strategyIds = requested.filter((id) => {
    if (!REPLAYABLE_STRATEGIES.includes(id)) {
      console.warn(`[backtest] skipping "${id}": not replayable from price history alone (see docs/ARCHITECTURE.md "Backtesting")`);
      return false;
    }
    return true;
  });
  if (strategyIds.length === 0) {
    console.error("[backtest] no replayable strategies to run");
    process.exit(1);
  }

  const candleConfigs = new Map(strategyIds.map((id) => [id, getCandleConfig(id, args.fineDays, args.coarseDays)]));
  const needsFine = [...candleConfigs.values()].some((c) => c.isFine);
  const needsCoarse = [...candleConfigs.values()].some((c) => !c.isFine);

  const fineTicks = needsFine ? await fetchHistoricalTicks(SOL_MINT, SOL_SYMBOL, "1m", new Date(now.getTime() - args.fineDays * 86_400_000), now) : [];
  const coarseTicks = needsCoarse ? await fetchHistoricalTicks(SOL_MINT, SOL_SYMBOL, "1H", new Date(now.getTime() - args.coarseDays * 86_400_000), now) : [];

  const results: SweepResult[] = [];
  const intervalSeconds: Partial<Record<BuiltInStrategyId, number>> = {};

  for (const id of strategyIds) {
    const { isFine } = candleConfigs.get(id)!;
    const ticks = isFine ? fineTicks : coarseTicks;
    intervalSeconds[id] = isFine ? 60 : 3600;

    // --hold-periods searches only "live-safe" params, holding any tick-count period param
    // at its shipped default — see TICK_COUNT_PARAMS: a period tuned against 1H/1m candles
    // is a real-time lookback the live 2s-poll engine usually can't reproduce at all (its
    // ctx.priceHistory tops out at 600 ticks, ~20min), so those numbers aren't directly
    // ship-able as new defaults regardless of how well they backtest. See
    // docs/ARCHITECTURE.md "Backtesting" for the full finding.
    const excludeParams = args.holdPeriods ? (TICK_COUNT_PARAMS[id] ?? []) : [];
    console.log(`\n[backtest] running ${id} against ${ticks.length} ${isFine ? "1m" : "1H"} candles (${args.trials} trials${args.holdPeriods ? ", periods held fixed" : ""})...`);
    const result = runSweep(id, ticks, limits, config.simulatedStartingCashUsd, { trials: args.trials, excludeParams });
    results.push(result);
    console.log(formatSweepConsole(result, intervalSeconds[id]!));
  }

  if (results.length > 1) {
    const path = writeCombinedReport(results, intervalSeconds);
    console.log(`\n[backtest] combined report written to ${path}`);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
