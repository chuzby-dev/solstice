# Solana Trading Bot — Phase 1: Scaffold + Paper Trading Core

An autonomous Solana trading system, built out in phases. **This phase is paper-trading
only: no real funds, wallet keys, or on-chain transactions are ever involved.** See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full design and what's deferred to
later phases.

## ⚠️ Risk Disclaimer

Even in simulation, this system is designed to eventually place autonomous trades with
real funds in a future phase. Automated trading strategies can lose money quickly,
especially in volatile crypto markets. Before ever connecting a funded wallet in a later
phase:

- Start with the smallest amount you're willing to lose entirely.
- Test extensively in this paper-trading mode and on devnet/testnet first.
- Understand every strategy's logic and risk parameters before activating it.
- Never disable the mandatory stop-loss or daily-loss auto-pause.
- This is not financial advice, and nothing here guarantees profitability.

## What this phase actually does

- Connects a Solana wallet (Phantom/Solflare) in **view-only** mode on **devnet** — it
  reads your public key and balance, and nothing else. No signing capability exists in
  this codebase yet.
- Runs all eleven built-in strategies (Dollar-Cost Averaging, Momentum, Mean Reversion,
  Grid Trading, RSI/MACD Crossover, Volatility Breakout, Whale/Wallet Copy-Trading,
  Short-Window Grid, Adaptive Range Scalper, Confluence Scalper, and the Fee-Aware Micro
  Scalper) against **real mainnet prices** pulled from Pyth Network's public Hermes API
  — the same on-chain price oracle most Solana wallets read from, polled every 2s —
  executed against a **virtual SQLite ledger** that starts with $10,000 of simulated
  cash. Whale Copy-Trading additionally reads (read-only) a watched wallet's public
  mainnet transaction history to mirror its trades; Short-Window Grid and the Adaptive
  Range Scalper operate on a real elapsed-time rolling window (scalper: configurable
  1-15 minutes, with a trend-regime filter, reward:risk gate, tight per-trade stop, and
  time stop) rather than a fixed tick count; Confluence Scalper requires multiple
  independent technical signals (EMA trend, Bollinger Bands, RSI) to agree before
  risking a trade; the Fee-Aware Micro Scalper is built for small, high-frequency
  accounts and sizes its profit target/stop directly off real estimated Solana/Jupiter
  trading costs rather than an arbitrary % — see docs/ARCHITECTURE.md for details on all four.
- **Every simulated trade is now charged a realistic estimated fee** (Solana tx +
  priority fee + swap fee + slippage), deducted from cash and shown per-trade in the
  Trade Log. Paper P&L across every strategy now reflects real trading costs, not a
  fee-free simulation — see docs/ARCHITECTURE.md "Realistic trading costs" for why this
  matters and how to calibrate the fee estimate.
- Shows a live price everywhere it matters: an always-on header ticker, and a live
  price + short rolling-window chart right inside the strategy configuration form, so
  price-based limits (like a grid's range) can be set from actual recent movement
  instead of guessed blind.
- Gives every configured strategy its own independent position, even when two
  strategies trade the same token — Short-Window Grid and the Range Scalper can both
  run on SOL at once without one strategy's exit closing the other's position. Cash is
  still one shared pool; only positions are sub-ledgered per strategy (see
  docs/ARCHITECTURE.md).
- Enforces the full risk-management guard rail set (max position size, per-token
  exposure cap, daily-loss auto-pause, mandatory stop-loss, slippage/price-impact
  ceilings) on every simulated trade — the same logic a later phase will reuse for real
  execution.
- Ships a kill switch that immediately halts all simulated trading.
- Requires an explicit risk-disclaimer confirmation before any strategy can be activated.

**No code path in this repository can send a real transaction.** `sendTransaction` does
not appear anywhere in the source tree; `packages/backend/src/wallet/hotWallet.ts` is an
unimplemented stub that throws if called.

## Prerequisites

- Node.js 20+
- npm 10+

## Setup

```bash
npm install
cp .env.example .env   # defaults work out of the box, no API keys required for Phase 1
npm run dev             # starts backend (http://localhost:4000) and frontend (http://localhost:5173)
```

Open http://localhost:5173. Connect a devnet wallet (e.g. Phantom set to the Devnet
cluster) to see its balance, then head to the **Strategies** tab to configure and
activate any of the seven built-in strategies against SOL or USDC. Whale Copy-Trading
additionally needs a `watchedWalletAddress` and works best with a paid mainnet RPC
provider set in `SOLANA_MAINNET_RPC_URL` (the free default is rate-limited).

## Scripts

| Command | Description |
|---|---|
| `npm run dev` | Runs backend + frontend dev servers concurrently |
| `npm run build` | Builds all workspace packages |
| `npm test` | Runs the backend Vitest suite (strategy logic + risk-manager guards) |
| `npm run typecheck` | Type-checks all workspace packages |
| `npm run backtest` (from `packages/backend`) | Backtests strategies against real historical price data — see "Backtesting" above |

## Architecture overview

```
packages/
  shared/    Types shared between backend and frontend
  backend/   Fastify API + strategy engine + paper-trading execution simulator
  frontend/  React + Vite + Tailwind GUI
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full breakdown, key design
decisions (why paper trading uses real prices, how the risk manager works, per-strategy
simplifications like momentum's missing volume confirmation and volatility breakout's
ATR proxy), the backtesting engine (below), and the list of follow-up phases (hot-wallet
signing, live Jupiter swaps, custom-script sandbox, alerting).

## Backtesting

`npm run backtest` (inside `packages/backend`) replays real historical SOL/USD price data
(via [Birdeye](https://birdeye.so); set `BIRDEYE_API_KEY` in `.env`, free tier) through
the actual strategy and risk-manager code — not a re-implementation — to evaluate and
tune strategies against real market history instead of only short live paper-trading
windows:

```bash
cd packages/backend
npm run backtest -- --strategy all --trials 150       # full sweep, all replayable strategies
npm run backtest -- --strategy range-scalper           # one strategy
```

`whale-copy` isn't included — it depends on live on-chain transfer data with no
historical-replay source in this codebase. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) "Backtesting" for methodology (the
tuning/validation split, the tick-count-vs-live-cadence limit) and
`packages/backend/reports/` for tuning results — the first pass validated one default
change (`rsi-macd`'s `overboughtRsi` and `positionSizeUsd`) and left every other
strategy's shipped defaults unchanged, since the rest either didn't hold up on a held-out
validation window or depended on a lookback period the live 2-second-poll engine can't
actually support.

## Testing

```bash
npm test
```

Covers every risk-manager guard (position sizing, daily-loss auto-pause, per-token
exposure, slippage/price-impact ceilings, stop-loss computation), the indicator math
(SMA/EMA/RSI/MACD/volatility), and all seven built-in strategies' signal logic. Run
before activating any new strategy config.
