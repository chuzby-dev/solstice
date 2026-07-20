import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";
import { sma } from "../indicators.js";
import { estimateTradeFeeUsd } from "../../config.js";

/** Fee-Aware Micro Scalper: high-frequency, low-value, low-volatility trading built
 * for a small account (designed around ~$200 total, ~$20 per trade) where trading
 * costs are the dominant risk, not price risk.
 *
 * Every other scalping strategy in this codebase sizes its profit target off market
 * structure (a range, a Bollinger band, an ATR multiple) and only incidentally clears
 * trading costs if the move happens to be big enough. This one inverts that: the
 * profit target and stop are DERIVED DIRECTLY from `estimateTradeFeeUsd` — the same
 * cost model `execution/simulator.ts` now actually charges on every fill (see
 * config.ts `tradingCosts`) — with a safety margin on top, rather than from an
 * arbitrary percentage. If the realistic round-trip cost isn't clearly covered, no
 * trade happens, full stop.
 *
 * Why this matters more here than elsewhere: at LOW trade values, the fixed portion of
 * the cost (Solana tx fee + priority fee — a flat dollar amount, not a percentage)
 * becomes a bigger share of the trade. A $0.011 fixed cost is 0.055% of a $20 trade but
 * 0.55% of a $2 trade — ten times worse. "Low value + high frequency" without fee
 * awareness is close to a mathematical guarantee of losing to costs alone, however good
 * the entry signal is. This strategy makes that arithmetic explicit and refuses to
 * trade when it doesn't work out, rather than hoping volatility bails it out.
 *
 * Mechanism: a short-period SMA acts as a "fair value" anchor (tick-count period, ~24s
 * of history at the default 2s poll interval — deliberately short, matching "high
 * frequency"). Buys a small dip below it (`dipPct`, far smaller than
 * MeanReversionStrategy's 5% default — appropriate for genuinely low-volatility
 * conditions, the opposite regime RangeScalperStrategy filters OUT via minRangePct),
 * confirmed by a turning-up tick. Before entering, checks that the SMA itself — the
 * realistic reversion target — is high enough to clear the fee-derived profit target;
 * if the "reasonable" bounce wouldn't even cover costs with margin, it skips the setup
 * entirely rather than hoping for a bigger move than the recent range suggests.
 * Take-profit/stop are then fixed at entry-relative percentages computed from that same
 * fee math, plus a short time stop (this is meant to resolve fast, not to be held).
 *
 * Be aware `dipPct` is a cheap pre-filter, not the real gate: with realistic defaults
 * (~0.30% round-trip cost from swap fee + slippage alone, before the fixed per-tx
 * component), the fee-derived requirement is typically the binding constraint, not the
 * stated `dipPct`. At the defaults below, a dip needs to be roughly 0.53% below the SMA
 * for the SMA-reversion check to pass — several times deeper than `dipPct`'s 0.08%
 * floor. This is intentional: it means the strategy will legitimately sit idle during
 * genuinely flat stretches rather than trade noise it can't profit from after costs —
 * the same lesson learned the hard way with an earlier version of RangeScalperStrategy
 * that didn't enforce this and could barely ever fire profitably. "High frequency"
 * here means "resolves fast and re-enters readily once a real move happens," not
 * "trades on every tick regardless of whether the move actually justifies the cost." */
export class FeeAwareScalperStrategy extends StrategyBase {
  readonly id = "fee-aware-scalper" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const p = ctx.config.params;
    const positionSizeUsd = p.positionSizeUsd ?? 20;
    const smaPeriod = p.smaPeriod ?? 12;
    const dipPct = p.dipPct ?? 0.08;
    const minProfitMultiple = p.minProfitMultiple ?? 1.5;
    const stopLossMultiple = p.stopLossMultiple ?? 1.0;
    const maxHoldMinutes = p.maxHoldMinutes ?? 3;

    // Round-trip cost estimate for a position of this size, using the SAME cost model
    // the simulator actually charges — not a separate guess. One leg each way.
    const oneLegFeeUsd = estimateTradeFeeUsd(positionSizeUsd);
    const roundTripFeeUsd = oneLegFeeUsd * 2;
    const requiredProfitPct = ((roundTripFeeUsd * minProfitMultiple) / positionSizeUsd) * 100;
    const stopLossPct = ((roundTripFeeUsd * stopLossMultiple) / positionSizeUsd) * 100;

    const price = ctx.latestPrice.priceUsd;

    if (ctx.currentPosition) {
      const entry = ctx.currentPosition.avgEntryPriceUsd;
      const sizeUsd = ctx.currentPosition.quantity * price;

      const stopPrice = entry * (1 - stopLossPct / 100);
      if (price <= stopPrice) {
        return this.sell(ctx, sizeUsd, `Stop-loss: price ${price} <= ${stopLossPct.toFixed(3)}% below entry ${entry.toFixed(4)} (${stopLossMultiple}x round-trip fee $${roundTripFeeUsd.toFixed(4)})`);
      }

      const targetPrice = entry * (1 + requiredProfitPct / 100);
      if (price >= targetPrice) {
        return this.sell(ctx, sizeUsd, `Take-profit: price ${price} >= ${requiredProfitPct.toFixed(3)}% above entry ${entry.toFixed(4)} (clears ${minProfitMultiple}x round-trip fee $${roundTripFeeUsd.toFixed(4)})`);
      }

      if (ctx.lastSignalAt) {
        const heldMinutes = (ctx.now.getTime() - ctx.lastSignalAt.getTime()) / 60_000;
        if (heldMinutes >= maxHoldMinutes) {
          return this.sell(ctx, sizeUsd, `Time stop: held ${heldMinutes.toFixed(1)}min >= ${maxHoldMinutes}min`);
        }
      }

      return null;
    }

    // ---------- Entry pipeline (flat) ----------
    const prices = ctx.priceHistory.map((t) => t.priceUsd);
    const smaValue = sma(prices, smaPeriod);
    if (smaValue === null) return null; // not enough history yet

    const dipThreshold = smaValue * (1 - dipPct / 100);
    if (price > dipThreshold) return null; // not dipped enough to be worth watching

    const previousTick = ctx.priceHistory[ctx.priceHistory.length - 2];
    if (!previousTick || price <= previousTick.priceUsd) return null; // confirmation tick: buy the turn, not the fall

    // The realistic reversion target (the SMA itself) must clear the fee-derived
    // profit target with margin. If it doesn't, this dip isn't a trade — it's just
    // noise that would lose to costs even in the best realistic case.
    const requiredTargetPrice = price * (1 + requiredProfitPct / 100);
    if (smaValue < requiredTargetPrice) return null;

    return this.buy(
      ctx,
      positionSizeUsd,
      `Fee-aware entry: price ${price} dipped ${dipPct}%+ below SMA${smaPeriod} $${smaValue.toFixed(4)}, turning up; ` +
        `needs ${requiredProfitPct.toFixed(3)}% to clear round-trip cost $${roundTripFeeUsd.toFixed(4)} (${minProfitMultiple}x margin) — SMA reversion covers it`,
    );
  }
}
