import { eq } from "drizzle-orm";
import type { AppModeState, Network, TradingMode } from "@trading-bot/shared";
import { db } from "../db/client.js";
import { appMode } from "../db/schema.js";

/** The only two states the app can ever be in — like two separate accounts, not two
 * independently-variable toggles. `network` is a pure function of `tradingMode`, never a
 * second axis a caller can set independently: Paper always means Devnet (test funds,
 * simulated trades), Live always means Mainnet (real funds, real autonomous trades).
 * Before this, `tradingMode` and `network` were separately selectable, which allowed
 * (harmless but confusing) combinations like paper-mode-with-a-mainnet-wallet-in-view —
 * exactly the kind of "which mode am I actually in?" ambiguity this collapses away.
 * `live`+`devnet` was previously rejected at runtime; now it's simply impossible to
 * construct, since nothing ever passes `network` in from outside. */
function networkFor(tradingMode: TradingMode): Network {
  return tradingMode === "live" ? "mainnet" : "devnet";
}

export function getAppMode(): AppModeState {
  const row = db.select().from(appMode).where(eq(appMode.id, "singleton")).get();
  if (!row) throw new Error("app_mode singleton row missing; db not initialized correctly");
  return { tradingMode: row.tradingMode as TradingMode, network: row.network as Network };
}

export function setAppMode(tradingMode: TradingMode): AppModeState {
  const network = networkFor(tradingMode);
  const updatedAt = new Date().toISOString();
  db.update(appMode).set({ tradingMode, network, updatedAt }).where(eq(appMode.id, "singleton")).run();
  return { tradingMode, network };
}
