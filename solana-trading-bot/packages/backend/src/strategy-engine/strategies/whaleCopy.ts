import type { Signal } from "@trading-bot/shared";
import { StrategyBase, type StrategyContext } from "../StrategyBase.js";
import { drainPending } from "../../market/whaleWatcher.js";

/** Whale/Wallet Copy-Trading: mirrors trades detected on a watched on-chain address
 * (config.watchedWalletAddress), scaled down by `copyRatioPct` and capped at
 * `maxSizeUsd`, only after they're at least `lagSeconds` old. Detection itself happens
 * out-of-band in market/whaleWatcher.ts (async on-chain polling); this just drains
 * whatever it has queued, keeping onInterval synchronous like every other strategy. */
export class WhaleCopyStrategy extends StrategyBase {
  readonly id = "whale-copy" as const;

  onInterval(ctx: StrategyContext): Signal | null {
    const { lagSeconds, maxSizeUsd, copyRatioPct } = ctx.config.params;

    const transfers = drainPending(ctx.config.id);
    if (transfers.length === 0) return null;

    const lagMs = (lagSeconds ?? 30) * 1000;
    const eligible = transfers.filter((t) => ctx.now.getTime() - t.blockTime.getTime() >= lagMs);
    if (eligible.length === 0) return null;

    // Mirror the most recent eligible whale trade; older ones in this batch are dropped
    // rather than queued indefinitely, since by the time we'd get to them they're stale.
    const transfer = eligible[eligible.length - 1]!;

    if (transfer.direction === "sell" && !ctx.currentPosition) return null; // nothing to mirror-sell

    const wantSizeUsd = transfer.tokenAmount * ctx.latestPrice.priceUsd * ((copyRatioPct ?? 10) / 100);
    let sizeUsd = Math.min(wantSizeUsd, maxSizeUsd ?? 200);
    if (transfer.direction === "sell" && ctx.currentPosition) {
      sizeUsd = Math.min(sizeUsd, ctx.currentPosition.quantity * ctx.latestPrice.priceUsd);
    }

    const shortSig = `${transfer.signature.slice(0, 8)}…`;
    return transfer.direction === "buy"
      ? this.buy(ctx, sizeUsd, `Mirroring whale buy of ${transfer.tokenAmount.toFixed(4)} ${ctx.config.tokenSymbol} (tx ${shortSig})`)
      : this.sell(ctx, sizeUsd, `Mirroring whale sell of ${transfer.tokenAmount.toFixed(4)} ${ctx.config.tokenSymbol} (tx ${shortSig})`);
  }
}
