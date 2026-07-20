import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";

const MIN_REFERENCE_TICKS = 4;
const MIN_WINDOW_MINUTES = 1;
const MAX_WINDOW_MINUTES = 15;
/** Entries are only taken when (target - entry) >= this multiple of (entry - stop).
 * Hard-coded rather than a param: a user-tunable reward:risk floor below 1 would turn
 * the strategy into a systematic negative-expectancy machine. */
const MIN_REWARD_RISK = 1.2;

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

/** Adaptive Range Scalper (1-15 min window).
 *
 * The naive version of this strategy ("buy the bottom X% of the recent range, sell the
 * top") loses money — or simply never fires — in several well-known ways, each with an
 * explicit defense here:
 *
 * 1. TREND TRAP — "support" keeps breaking because the market is trending, so every buy
 *    catches a falling knife. Defense: a regime filter. The efficiency ratio
 *    |net move| / (sum of |tick moves|) is ~1 in a trend and ~0 in chop; entries are
 *    blocked when it exceeds `maxTrendEfficiency`. Range scalping is only attempted in
 *    actual ranges.
 * 2. RANGES TOO THIN TO PROFIT — if the whole window's range is smaller than round-trip
 *    fees/slippage, a perfect scalp still loses. Defense: skip windows whose range is
 *    under `minRangePct` of price.
 * 3. STOP/TARGET SCALED TO THE WRONG THING — an earlier version used a stop that was a
 *    fixed % of PRICE while the target was a % of the (often much smaller) RANGE. As the
 *    range shrinks toward `minRangePct`, reward shrinks with it but risk stayed fixed,
 *    so the reward:risk gate became nearly impossible to clear except in ranges several
 *    times wider than the stated minimum — the strategy could almost never fire at all.
 *    Fixed by making the stop `stopBufferPct` of the *range* too, so the reward:risk
 *    ratio depends only on the shape parameters (buyZonePct, targetRangePct,
 *    stopBufferPct), not on how wide the range happens to be.
 * 4. SELF-REFERENTIAL RANGE — a first attempt at (3) computed the exit-side stop/target
 *    from a window that always includes the very tick being tested against it. That's
 *    self-defeating: a tick that sets a new low always looks safely above a stop derived
 *    from a range whose low IS that tick, so the stop could never fire; symmetrically a
 *    target derived from a range whose high IS the current tick is trivially hit on any
 *    bare new high, whether or not it represents a meaningful move. Fixed by computing
 *    the exit-side reference range from ticks *before* the current one — the range as
 *    established prior to this evaluation, which the current price can legitimately
 *    break above or below.
 * 5. NO EXIT DISCIPLINE — the naive version holds losers until the (far too wide for
 *    scalping) global stop-loss. Defense: the range-relative stop above, plus a time
 *    stop (`maxHoldMinutes`) that exits a scalp whose thesis hasn't resolved in ~2
 *    windows. A separate `hardStopPct` (fixed % below entry) also runs unconditionally,
 *    even before enough window data exists to compute the range-relative stop — a
 *    position must never go unprotected just because of a data-feed gap or restart.
 * 6. CHURN — buying every tick that grazes the buy zone, including mid-collapse, and
 *    instantly re-entering after every exit. Defense: entries require a confirmation
 *    tick (price in the buy zone AND ticking back up — buy the turn, not the fall) and
 *    a re-entry cooldown of half a window after the last trade.
 *
 * None of this guarantees profit — it removes the classic structural losers (and the
 * structural non-firers) and only takes trades where the math is favorable. */
export class RangeScalperStrategy extends StrategyBase {
  readonly id = "range-scalper" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const p = ctx.config.params;
    const windowMinutes = clamp(p.windowMinutes ?? 5, MIN_WINDOW_MINUTES, MAX_WINDOW_MINUTES);
    const buyZonePct = p.buyZonePct ?? 20;
    const targetRangePct = p.targetRangePct ?? 70;
    const stopBufferPct = p.stopBufferPct ?? 10;
    const hardStopPct = p.hardStopPct ?? 2;
    const minRangePct = p.minRangePct ?? 0.3;
    const maxTrendEfficiency = p.maxTrendEfficiency ?? 0.35;
    const maxHoldMinutes = p.maxHoldMinutes ?? 10;
    const positionSizeUsd = p.positionSizeUsd ?? 100;

    const windowMs = windowMinutes * 60_000;
    const nowMs = ctx.now.getTime();
    const windowTicks = this.recentWindow(ctx, windowMs);
    const price = ctx.latestPrice.priceUsd;

    // ---------- Exit management (position open) ----------
    if (ctx.currentPosition) {
      const entry = ctx.currentPosition.avgEntryPriceUsd;
      const sizeUsd = ctx.currentPosition.quantity * price;

      // Emergency backstop: fixed % below entry, checked unconditionally regardless of
      // window data availability. A risk-reducing exit must never wait on window
      // statistics after a data-feed gap or restart. In normal operation this is wider
      // than the range-relative stop below and should rarely be the one that fires.
      const hardStopPrice = entry * (1 - hardStopPct / 100);
      if (price <= hardStopPrice) {
        return this.sell(ctx, sizeUsd, `Hard stop: price ${price} <= ${hardStopPct}% below entry ${entry.toFixed(4)}`);
      }

      // Reference range EXCLUDES the current tick (see class doc, point 4): a stop or
      // target derived from a range that includes the very price being tested against
      // it is self-defeating (unreachable stop, trivially-hit target).
      const referenceTicks = windowTicks.slice(0, -1);
      if (referenceTicks.length >= MIN_REFERENCE_TICKS) {
        const referencePrices = referenceTicks.map((t) => t.priceUsd);
        const windowLow = Math.min(...referencePrices);
        const windowHigh = Math.max(...referencePrices);
        const range = windowHigh - windowLow;

        if (range > 0) {
          const rangeStopPrice = windowLow - range * (stopBufferPct / 100);
          if (price <= rangeStopPrice) {
            return this.sell(ctx, sizeUsd, `Range stop: price ${price} broke below support $${windowLow.toFixed(4)} (buffer ${stopBufferPct}%)`);
          }

          // Never "takes profit" below entry — if the range has drifted down around
          // us, the stop or time stop handles the exit instead.
          const rangeTarget = windowLow + range * (targetRangePct / 100);
          if (price >= rangeTarget && price > entry) {
            return this.sell(ctx, sizeUsd, `Range target: price ${price} >= ${targetRangePct}% of established $${windowLow.toFixed(4)}-$${windowHigh.toFixed(4)} range`);
          }
        }
      }

      // Time stop: lastSignalAt is this config's last trade, i.e. our entry. If the
      // position was opened by another strategy (shared per-token ledger), lastSignalAt
      // may be null — no reliable entry time, so the time stop is skipped.
      if (ctx.lastSignalAt) {
        const heldMinutes = (nowMs - ctx.lastSignalAt.getTime()) / 60_000;
        if (heldMinutes >= maxHoldMinutes) {
          return this.sell(ctx, sizeUsd, `Time stop: held ${heldMinutes.toFixed(1)}min >= ${maxHoldMinutes}min, scalp thesis expired`);
        }
      }

      return null;
    }

    // ---------- Entry pipeline (flat) ----------
    // Unlike the exit side, entry's range legitimately includes the current (latest)
    // tick: the confirmation filter below already requires price to be ticking UP from
    // the previous tick, which structurally prevents entering exactly on a fresh window
    // low — so the buy-zone/R:R checks here don't suffer the same self-referential
    // triviality as the exit-side stop/target did.
    if (windowTicks.length < MIN_REFERENCE_TICKS) return null; // not enough data in the window yet

    const prices = windowTicks.map((t) => t.priceUsd);
    const windowLow = Math.min(...prices);
    const windowHigh = Math.max(...prices);
    const range = windowHigh - windowLow;

    // Filter 1: the range must be wide enough to be worth trading at all.
    if (range <= 0 || (range / price) * 100 < minRangePct) return null;

    // Filter 2: regime — only scalp actual ranges, never trends.
    const netMove = Math.abs(prices[prices.length - 1]! - prices[0]!);
    let pathLength = 0;
    for (let i = 1; i < prices.length; i++) {
      pathLength += Math.abs(prices[i]! - prices[i - 1]!);
    }
    const efficiency = pathLength > 0 ? netMove / pathLength : 1;
    if (efficiency > maxTrendEfficiency) return null; // trending: stand aside

    // Filter 3: price must be in the buy zone near the window low.
    const buyZoneThreshold = windowLow + range * (buyZonePct / 100);
    if (price > buyZoneThreshold) return null;

    // Filter 4: confirmation tick — buy the turn, not the fall.
    const previousTick = windowTicks[windowTicks.length - 2];
    if (!previousTick || price <= previousTick.priceUsd) return null;

    // Filter 5: re-entry cooldown of half a window since this config's last trade.
    if (ctx.lastSignalAt) {
      const sinceLastTradeMs = nowMs - ctx.lastSignalAt.getTime();
      if (sinceLastTradeMs < windowMs / 2) return null;
    }

    // Filter 6: the setup must offer real asymmetry. Both reward and risk are measured
    // as fractions of the SAME range, so this ratio is independent of how wide the
    // range happens to be (see class doc, point 3) — unlike a price-relative stop, a
    // thin-but-qualifying range and a wide range are held to the same standard.
    const target = windowLow + range * (targetRangePct / 100);
    const stopPrice = windowLow - range * (stopBufferPct / 100);
    const reward = target - price;
    const risk = price - stopPrice;
    if (risk <= 0 || reward < MIN_REWARD_RISK * risk) return null;

    return this.buy(
      ctx,
      positionSizeUsd,
      `Range entry: price ${price} in bottom ${buyZonePct}% of ${windowMinutes}min range ($${windowLow.toFixed(4)}-$${windowHigh.toFixed(4)}), turning up, R:R ${(reward / risk).toFixed(2)}`,
    );
  }
}
