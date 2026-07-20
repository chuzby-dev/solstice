# Architecture — Phase 1

## Goal of this phase

Prove out the full vertical slice (wallet view → strategy engine → risk-managed
execution → GUI) without touching real funds, so the architecture can be validated and
reviewed before any phase that can move money is built.

## Data flow

```
Pyth Network Hermes API (real mainnet prices, read-only)
        │  polled every PRICE_POLL_INTERVAL_MS (default 2s)
        ▼
market/priceFeed.ts ──► priceCache.ts (in-memory rolling history)
        │
        │ tick
        ▼
strategy-engine/engine.ts
   1. for each OPEN POSITION (per strategy config) in this token:
        checkAndApplyStopLoss() — mandatory protective exit, independent of strategy
   2. for each active StrategyConfig on this token:
        strategy.onInterval(ctx) → Signal | null   (ctx.currentPosition is THIS config's own sub-ledger)
        executeSignal(signal, riskLimits)
           → riskManager.evaluateSignal()   — shrink/reject per risk guards
           → simulator.applyFill()          — mutate virtual ledger (SQLite)
        ▼
ws/hub.ts broadcasts {trade, portfolio, price_tick, engine_status} to all connected GUIs
```

Nothing in this path constructs, signs, or sends a Solana transaction. The Solana RPC
calls in the backend are all read-only: `getBalance` / `getParsedTokenAccountsByOwner`
in `wallet/walletRoutes.ts`, and `getSignaturesForAddress` / `getParsedTransaction`
against **mainnet** in `market/whaleWatcher.ts` (used only by the whale-copy strategy
to observe a watched wallet's public history — see "Built-in strategies" below).

## Built-in strategies

All seven strategies from the spec are implemented, each as a stateless class in
`strategy-engine/strategies/` implementing `StrategyBase.onInterval(ctx) -> Signal | null`.
"Stateless" matters here: `strategy-engine/registry.ts` holds one singleton instance per
strategy *type*, shared across every configured instance of that type, so a strategy must
never store per-config state on `this` — anything it needs to remember (last trade time,
current position) is derived from `ctx` (backed by the DB) on every call.

- **DCA**, **Momentum** — see their own file comments; unchanged from the original phase.
- **Mean Reversion** (`meanReversion.ts`) — buys `deviationPct` below the `maPeriod`
  simple moving average, sells on reversion back up to it.
- **Grid Trading** (`grid.ts`) — buys/sells as price crosses fixed steps within
  `[lowerPrice, upperPrice]`. Simplified to one position at a time rather than many
  simultaneous open orders across levels (see "Known Phase 1 simplifications").
  `computeGridCrossing`'s `minRangePct` guard (default 0.3%, shared with
  Short-Window Grid) sits the strategy out entirely when the range is too thin to be
  worth trading fees — added after live observation: a quiet market produced a
  0.125%-wide auto-range, i.e. ~0.02%-per-step grid lines on a $75 asset, which this
  fee-less simulator showed as flat P&L but which real trading fees would turn into a
  guaranteed loss on every round trip.
- **RSI/MACD Crossover** (`rsiMacd.ts`) — enters on a bullish MACD crossover confirmed
  by RSI not overbought, exits on a bearish crossover or RSI reaching overbought. Uses
  an epsilon tolerance around histogram sign changes (`HISTOGRAM_EPSILON`) because
  floating-point EMA rounding can leave a conceptually-flat histogram at something like
  `8.8e-16` instead of exactly `0`, which would otherwise silently suppress a legitimate
  crossover — this was caught by a failing unit test, not inferred in advance.
- **Volatility Breakout** (`volatilityBreakout.ts`) — enters when a single tick-to-tick
  move exceeds `atrMultiplier` × recent average volatility (see below re: ATR).
- **Whale/Wallet Copy-Trading** (`whaleCopy.ts` + `market/whaleWatcher.ts`) — the only
  strategy needing genuinely async I/O (on-chain tx polling), which doesn't fit the
  synchronous `onInterval` contract every other strategy uses. Resolved by running
  `whaleWatcher` as an independent background poller (same pattern as `priceFeed.ts`)
  that queues detected transfers per strategy-config-id; `whaleCopy.ts` just drains that
  queue synchronously on each tick, applying `lagSeconds` and size limits. Requires
  `SOLANA_MAINNET_RPC_URL` and a `watchedWalletAddress` per config (validated as a real
  base58 public key on creation) — read-only, it can never sign or send on the watched
  wallet's behalf.
- **Short-Window Grid** (`shortWindowGrid.ts`) and **Adaptive Range Scalper**
  (`rangeScalper.ts`) — both use a *real elapsed-time* rolling window
  (`windowMinutes`) over `ctx.priceHistory`, filtering by actual tick timestamps
  rather than a fixed tick count like Momentum's `lookbackPeriods`. This makes them
  correct regardless of `PRICE_POLL_INTERVAL_MS`. Short-Window Grid reuses `grid.ts`'s
  level-crossing logic (`computeGridCrossing`, extracted as a shared pure function) but
  recomputes `[lowerPrice, upperPrice]` every tick from the window's actual high/low
  instead of a fixed manual range — no need to guess bounds up front. `priceCache`'s
  `HISTORY_LIMIT` (6000 ticks — see "Backtesting" for why it's this large) is sized to
  comfortably hold the maximum window even if the poll interval is turned down well
  below 10s.

  The Adaptive Range Scalper's window is user-configurable and clamped to **1-15
  minutes** in the strategy itself (not just the UI). It was deliberately designed
  around the classic ways naive range scalpers lose money — or simply never fire at
  all — each with an explicit defense (full detail in the file header of
  `rangeScalper.ts`):
  1. *Trend trap* — an efficiency-ratio regime filter (|net move| / path length) blocks
     entries when the market is trending rather than ranging, so it never buys
     "support" in a falling market.
  2. *Ranges too thin to profit* — windows whose total range is under `minRangePct` of
     price are skipped as unable to cover round-trip costs.
  3. *Stop/target scaled to the wrong thing* — an earlier version used a stop that was a
     fixed % of PRICE while the target was a % of the (often much smaller) RANGE. As the
     range shrinks toward `minRangePct`, reward shrinks with it but risk stayed fixed,
     so the reward:risk gate below became nearly impossible to clear except in ranges
     several times wider than the stated minimum — **the strategy could almost never
     fire at all**, caught via a direct user report ("how will the scalping strategy
     ever work, the defaults are really high") and confirmed by computing the actual
     numbers (a qualifying 0.3%-range setup needed ~3x that width just to clear the
     1.2x gate). Fixed by making the stop (`stopBufferPct`) a % of the *range* too, so
     the reward:risk ratio depends only on the shape parameters (`buyZonePct`,
     `targetRangePct`, `stopBufferPct`), not on how wide the range happens to be.
  4. *Self-referential range* — a first attempt at fix (3) computed the exit-side
     stop/target from a window that always includes the very tick being tested against
     it, which is self-defeating: a tick that sets a new low always looks safely above
     a stop derived from a range whose low IS that tick (the stop could **never** fire,
     verified with a standalone calculation before shipping), and symmetrically a
     target derived from a range whose high IS the current tick is trivially hit on any
     bare new high. Fixed by computing the exit-side reference range from ticks
     *before* the current one (`referenceTicks = windowTicks.slice(0, -1)`) — regression
     tests in `test/rangeScalper.test.ts` cover both directions directly.
  5. *No exit discipline* — the range-relative stop above, plus a time stop
     (`maxHoldMinutes`) that closes any scalp whose thesis has expired. A separate fixed
     `hardStopPct` (below entry, not range-relative) also runs unconditionally even
     before enough window data exists for the range-relative checks — a position must
     never go unprotected after a data-feed gap or restart, mirroring the same
     data-gap-resilience fix already applied to the stop/time-stop ordering.
  6. *Churn* — entries need a confirmation tick (in the buy zone AND ticking back up:
     buy the turn, not the fall), and a re-entry cooldown of half a window follows
     every trade.
  On top of those, every entry must offer at least 1.2x reward-to-risk; the floor is
  hard-coded, not a param, since a user-tunable value below 1 would guarantee negative
  expectancy. The range-target exit never fires below entry — losing exits happen only
  via the stop or time stop, keeping average losses small relative to average wins.
  None of this guarantees profitability; it removes the structural losers (and the
  structural non-firers) and only takes trades with favorable, scale-invariant
  asymmetry.

- **Confluence Scalper** (`confluenceScalper.ts`) — every other strategy in this
  codebase acts on one signal type (a moving average, a breakout, a crossover, a range
  position). This one requires multiple independent signals to agree before risking a
  trade, the way real short-term scalping systems typically operate: an EMA(9)/EMA(21)
  trend filter (only buy pullbacks WITH the trend), a Bollinger Band (20-period, 2
  std-dev, added to `indicators.ts`) or RSI(9) pullback signal (either is sufficient —
  requiring both simultaneously was the same over-strict-confluence mistake that made an
  early version of the Range Scalper nearly unfireable), and the same confirmation-tick
  defense proven out there (buy the turn, not the fall). Exit uses a fixed %
  take-profit/stop-loss — both measured in the same unit (% of entry price), so unlike
  Range Scalper's first attempt there's no mismatched-units bug to introduce — plus an
  immediate exit if the EMA trend itself flips against the position (the entry thesis
  was "pullback within an uptrend"; if the uptrend is gone, so is the thesis, regardless
  of where price sits relative to target/stop), plus a time stop. Deliberately excludes
  VWAP (needs trade volume, which Pyth doesn't provide), the Stochastic Oscillator
  (measures the same thing as RSI — redundant, not more robust), and MACD (already the
  dedicated signal in RSI/MACD Crossover; this strategy is meant to add a genuinely
  different angle, not re-skin an existing one).

- **Fee-Aware Micro Scalper** (`feeAwareScalper.ts`) — see "Realistic trading costs"
  below for the full design. In short: its profit target and stop aren't an arbitrary
  percentage, they're computed directly from the same round-trip fee estimate the
  simulator actually charges, plus a safety margin, specifically for small-account
  high-frequency trading where costs (not price risk) are the dominant threat.
- **Dip Reversion** (`dipReversion.ts`) — the one strategy in this codebase designed
  backwards from data instead of forwards from a TA pattern: built after finding a real
  statistical signal directly in this app's own historical SOL/USD price series (buying a
  confirmed dip and holding 60-180 minutes has a real edge; momentum/trend-following has
  none — see "Backtesting" below for the numbers). Its first version didn't survive a
  fee/execution-realistic backtest (the raw edge was smaller than the round-trip fee); a
  second version, widened to a rarer/larger-dip trigger after measuring where the fee-free
  edge actually clears the fee floor, is the first strategy in this codebase with a
  **positive** result on both the tuning and held-out validation windows — thinly (3
  validation trades), not a strong statistical confirmation yet. See "Backtesting" for the
  full before/after numbers. Its existence also forced a real architecture fix
  (`priceCache`'s history budget, see below). Every param is real elapsed time or a %
  magnitude, never a tick count — see "Backtesting"'s tick-count-vs-live-cadence section
  for why that distinction matters.

## Live price visibility

`GET /api/market/prices` (all allowlisted tokens' latest tick) and
`GET /api/market/:tokenMint/history?minutes=N` (recent history for one token, capped at
60 minutes) expose `priceCache` over REST. The frontend uses these plus the existing
`price_tick` WebSocket stream to show: an always-visible header ticker
(`LivePriceTicker.tsx`) so a current price is on screen everywhere, and a live price +
short rolling-window chart (`PriceSparkline.tsx`) inside the Strategy Selector's
configuration form — so the user can see actual recent movement before typing in any
price-based limit (grid bounds, etc.) instead of guessing blind.

## Why paper trading uses real prices, virtual money

Jupiter's swap/price infrastructure and Solana DEX liquidity in general are mainnet-only,
and devnet has no real token liquidity to price against. Using devnet prices would make
paper-trading results meaningless. Instead:

- Prices come from Pyth Network's public Hermes API (`market/priceFeed.ts`) —
  read-only, no API key, no funds at risk. This is the third data source tried: Jupiter's
  Price API needs a key to reliably price native SOL; CoinGecko's free tier worked with
  no key but its shared-IP rate limiting produced a burst-then-70-second-blackout
  pattern in practice (caught by a user report that live prices felt "slower than a
  wallet display," confirmed in the logs, not assumed). Pyth is the actual right fit —
  it's the on-chain price oracle most Solana wallets and DEXs read from, named directly
  in the original spec, free with no key — so `PRICE_POLL_INTERVAL_MS` could drop from
  10s to 2s and still track Pyth's own ~1-3s on-chain publish cadence.
  Hermes' public tier caps out at **10 requests / 10s**, banning the client for 60s if
  exceeded. Every poll already fetches all allowlisted tokens in one request (one
  `ids[]` query, not one request per token), so at the default 2s interval this app
  generates ~5 req/10s on its own — under the cap, though a shared sandbox egress IP
  could still push the aggregate over it (the same failure mode that broke CoinGecko).
  `priceFeed.ts` treats a 429 as a hard signal to stop entirely for the full 60s ban
  window (`blockedUntil`) rather than retrying every poll interval and re-triggering
  its own ban indefinitely.
- All trades execute against a virtual SQLite ledger (`db/schema.ts`:
  `positions`, `trades`, `portfolio_meta`) seeded with a simulated starting balance
  (`config.simulatedStartingCashUsd`, default $10,000).
- The wallet connection (devnet) is entirely separate from the paper-trading ledger —
  it exists only to prove the wallet-adapter wiring works ahead of a phase that
  actually funds and signs from a connected/hot wallet.

## Risk management

`execution/riskManager.ts` is a pure, side-effect-free function
(`evaluateSignal`) that enforces every non-negotiable guard from the spec:

- **Max position size** (% of portfolio) — shrinks oversized buys.
- **Per-token exposure cap** — shrinks or rejects buys that would over-concentrate.
- **Daily loss limit** — hard-rejects all buys once breached (auto-pause behavior).
- **Slippage ceiling** — hard-rejects if simulated slippage exceeds the configured cap.
- **Price-impact ceiling** — shrinks buys sized beyond the assumed-liquidity model.
- **Mandatory stop-loss** (`computeStopLossPrice`) — set on every newly opened position
  and enforced independently by `simulator.checkAndApplyStopLoss` on every price tick,
  regardless of which strategy opened the position.

This function has no DB or network dependency, which is what makes it exhaustively unit
tested (`test/riskManager.test.ts`) and reusable unchanged when a later phase wires it
into real transaction signing.

Limits are user-editable (Settings panel → `PUT /api/risk-settings`) but always clamped
to hard ceilings in `config.ts` (`riskHardCeilings`) — a user can tighten risk limits
freely but can never loosen them past the ceiling via the API.

## Realistic trading costs (fees)

Every simulated fill is charged an estimated real-world cost via `config.ts`
`estimateTradeFeeUsd`, deducted in `execution/simulator.ts` `applyFill()`. This was
added specifically because a fee-free paper simulator makes every strategy's results
look better than real trading would — actively misleading right before real funds go
into the account (this codebase supports paper trading only; there is still no signing
capability anywhere, see "Deferred to later phases" below, but the numbers shown here
should still be honest previews).

- **`solanaTxFeeUsd`** — the Solana base network fee (~5000 lamports), effectively
  negligible.
- **`priorityFeeUsd`** — a conservative mid-congestion estimate for landing a swap
  promptly. Real priority fees vary with network conditions.
- **`swapFeeBps` / `slippageBufferBps`** — per-leg estimate of DEX pool fee + realistic
  slippage for a small retail-sized SOL/USDC trade routed through Jupiter. This is a
  planning estimate, not a live quote — actual cost depends on the specific route
  Jupiter picks at execution time.

All four are configurable via env (see `.env.example`) for calibrating against real
observed costs later. The buy-side fee is folded into the position's **cost basis**, not
just deducted from cash — so it shows up immediately in that position's unrealized P&L
and correctly in realized P&L once it closes, rather than being an invisible drain that
only shows up in total portfolio value. `Trade.feeUsd` records exactly what was charged
per leg, visible in the Trade Log.

This only affects fills **going forward** from when the column was added — existing
trade history isn't rewritten (a small number of pre-existing rows show `feeUsd: 0`,
which is what they genuinely paid at the time, not a retroactive correction).

**Fee-Aware Micro Scalper** (`feeAwareScalper.ts`) is the strategy built specifically
around this cost model — see "Built-in strategies" above. It computes its own profit
target and stop directly from `estimateTradeFeeUsd` (the same function the simulator
uses to charge fees) rather than an arbitrary percentage, which matters most at low
trade sizes: the fixed portion of the cost (tx + priority fee) is a flat dollar amount,
so it consumes a much bigger percentage of a $5 trade than a $50 one. Designed around a
small account (~$200 total, ~$20/trade) doing frequent trades, where trading costs — not
price risk — are the dominant threat.

## Per-strategy position sub-ledgers

`positions` is keyed by `strategy_config_id`, not `token_mint` (see `db/schema.ts`).
Each configured strategy instance holds a fully independent position, stop-loss, and
average entry price in whatever token it trades — two active strategies both trading
SOL (e.g. Short-Window Grid and the Range Scalper) can't buy or sell out from under each
other. This wasn't the original design: Phase 1 initially shared one position per
token across all strategies, and running two SOL strategies concurrently surfaced the
exact failure mode you'd expect — one strategy's exit closing a position the other had
just opened. `getCurrentPosition(strategyConfigId)` and `getOpenPositionsForToken()`
in `execution/simulator.ts` are the two entry points: the former gives a strategy its
own position for `ctx.currentPosition`, the latter lets `engine.ts` run the mandatory
stop-loss check once per open position (per strategy) instead of once per token.

Cash remains one shared pool (`portfolio_meta`, unchanged) — only positions are
sub-ledgered. The per-token exposure risk guard (`perTokenExposurePct`) still has to be
a true portfolio-wide cap despite that, so `evaluateSignal`'s `currentTokenExposureUsd`
input is computed by `getTotalTokenExposureUsd(tokenMint)`, which sums every strategy's
exposure to that token — otherwise two strategies could each independently max out the
"per-token" cap and double the intended maximum exposure. Sell-size capping, in
contrast, correctly uses only the signaling strategy's own sub-ledger quantity — it can
only sell what it actually holds.

Migrating an existing database: the old one-row-per-token schema has no
`strategy_config_id` to attribute an open position to, so `db/client.ts` liquidates any
position found under the old schema back to cash (at its last recorded entry price,
logged clearly) rather than guessing an owner, then drops and recreates the table.

## Known Phase 1 simplifications

- **Price source (Pyth) covers only a small hardcoded set of tokens: SOL/USDC.**
  `market/priceFeed.ts` maps each allowlisted mint to a Pyth Hermes feed ID
  (`TOKEN_METADATA`) by hand. This is easy to extend (Pyth has feed IDs for most major
  Solana tokens — see `https://hermes.pyth.network/v2/price_feeds?query=<symbol>`) but
  isn't automatic; a later phase wanting arbitrary SPL token support should generalize
  this lookup or fall back to Jupiter/Birdeye/DexScreener for long-tail tokens Pyth
  doesn't cover.
- **Momentum has no volume confirmation.** The spec calls for "breakout above N-period
  high with volume confirmation," but Pyth's price feed doesn't carry trade volume
  either (it's a price oracle, not a DEX data source). The strategy currently triggers
  on price breakout alone. A later
  phase should wire in a volume-capable source (Birdeye/DexScreener) before this
  strategy runs live.
- **Volatility Breakout uses a close-to-close proxy, not true ATR.** True ATR needs
  OHLC candle data (high/low/close per period); the price feed only has one price per
  poll. `indicators.ts::closeToCloseVolatility` averages absolute tick-to-tick change as
  a stand-in — reasonable, but will read differently than a real ATR indicator would.
- **Grid Trading holds one position at a time, not many simultaneous orders.** A real
  grid bot keeps an open order at every level at once; this strategy (and
  Short-Window Grid) instead approximates the pattern with a single buy-low/sell-high
  cycle triggered by grid-line crossings.
- **Whale Copy-Trading depends on a rate-limited public RPC by default.**
  `SOLANA_MAINNET_RPC_URL` defaults to the public `api.mainnet-beta.solana.com`
  endpoint, which throttles `getParsedTransaction` heavily. The watcher polls
  conservatively (every 30s, max 5 signatures/check) and fails soft (logs and skips
  rather than crashing), but for reliable whale-copy behavior use Helius/QuickNode/Triton.
  It also only detects balance-delta transfers for the strategy's configured token, not
  arbitrary swaps the watched wallet makes in other tokens.
- **Price impact / slippage are modeled, not real.** `ASSUMED_LIQUIDITY_USD` and
  `SIMULATED_SLIPPAGE_BPS` in `execution/simulator.ts` are fixed placeholder figures.
  Once live Jupiter swaps are wired in, these should be replaced with the actual quote
  response's `priceImpactPct` and route slippage.
- **The app's SOL mint constant is missing a character.** `TOKEN_ALLOWLIST` /
  `market/priceFeed.ts`'s `TOKEN_METADATA` use
  `So11111111111111111111111111111111111111` (43 chars) everywhere in this codebase —
  the real canonical wrapped-SOL mint is `So11111111111111111111111111111111111111112`
  (with a trailing `2`). This has stayed invisible because Pyth's Hermes API prices by
  feed ID, not mint address, and the devnet wallet routes never validate address format
  strictly. It surfaced building the backtester: Birdeye's API *does* validate real
  Solana address format and 400s on the truncated string. Not fixed repo-wide here (touches
  `config.ts`, `.env*`, `priceFeed.ts`, the frontend token list, and every test file that
  hardcodes it — out of scope for a backtesting feature); `backtest/birdeyeClient.ts`
  hardcodes the corrected address for its own HTTP calls only, documented at the top of
  that file.
- **(Fixed) The live backend had never actually read the root `.env` file.** `config.ts`'s
  plain `import "dotenv/config"` loaded `.env` from `process.cwd()`, which is
  `packages/backend` when run as an npm workspace script — but `.env` lives at the repo
  root, one level up. Every config value had a fallback that happened to match `.env`'s
  shipped values, so every setting had silently been running on its hardcoded default the
  whole time with no visible symptom. First surfaced (and scoped-fixed only in the CLI
  script) while building the backtester; fixed at the app level once the on-demand
  `/api/backtest/*` routes made it a real live-server dependency (`BIRDEYE_API_KEY` has no
  fallback) — `config.ts` now resolves the root `.env` explicitly instead of relying on
  `cwd`. Confirmed harmless: every other setting's `.env` value already matched its
  hardcoded fallback, so this only changes behavior for the one value that mattered.

## Backtesting

`packages/backend/src/backtest/` replays historical price data through the real strategy
and risk logic to evaluate/tune strategies against actual market history instead of only
short live paper-trading windows. Run via `npm run backtest` (see script header for
flags) inside `packages/backend`; results in `packages/backend/reports/`.

**Reuse boundary.** `onInterval(ctx)` (every strategy class), `evaluateSignal()` +
`computeStopLossPrice()` (`execution/riskManager.ts` — pure), `estimateTradeFeeUsd()`
(`config.ts` — pure), and `indicators.ts` are reused completely unchanged — a backtest run
calls the exact same strategy code the live engine does. `execution/simulator.ts` itself
is *not* reused: it's hard-wired to the live SQLite `db` and `priceCache` singletons with
no injection seam, and `applyFill` isn't even exported, so reusing it as-is would mean
either mutating the live paper-trading ledger or spawning a subprocess per backtest run.
Instead `backtest/ledger.ts` is a separate in-memory ledger that replicates
`applyFill`'s fee/cost-basis/realized-P&L math and `checkAndApplyStopLoss`'s stop logic
exactly (including the daily-loss baseline rollover on calendar-day change) — see that
file's header comment for the line-by-line mapping. `backtest/backtestEngine.ts` mirrors
`strategy-engine/engine.ts`'s per-tick ordering (mandatory stop-loss check before the
strategy's own signal) and always uses the replayed tick's own timestamp as `ctx.now`,
never wall clock.

**Data.** `backtest/birdeyeClient.ts` fetches OHLCV history from Birdeye
(`BIRDEYE_API_KEY` in `.env`; free tier), paginating against its 1000-candles-per-request
cap and disk-caching results under `data/backtest-cache/` so repeated tuning runs don't
re-hit the API. SOL only — USDC is a stablecoin pegged ~$1, not a meaningful backtest
subject.

**Tuning.** `backtest/sweep.ts` runs a random search (a fixed trial budget, not a full
factorial grid — several strategies have 6-9 params) with the shipped defaults always
included as trial 0 for comparison. The chronological tick series is split ~70/30 into a
tuning window (searched) and a held-out validation window (never touched during the
search, only used afterward to check the winner still performs) — a parameter set that
only wins on the exact window it was searched against is exactly what this is meant to
catch, and it did (see `reports/backtest-2026-07-19.md` for several examples where a
tuning-window win reversed on validation).

**The tick-count-vs-live-cadence limit.** Birdeye's finest granularity is 1-minute
candles; the live engine polls Pyth every 2s (`PRICE_POLL_INTERVAL_MS`) and its
`ctx.priceHistory` caps out at `priceCache.HISTORY_LIMIT` ticks (see below for its current
value and history). A strategy param that
counts literal price *ticks* (`momentum.lookbackPeriods`, `meanReversion.maPeriod`,
`rsiMacd`'s RSI/MACD periods, `volatilityBreakout.atrPeriod`, `feeAwareScalper.smaPeriod`,
`confluenceScalper`'s EMA/Bollinger/RSI periods — enumerated in `sweep.ts`'s
`TICK_COUNT_PARAMS`) means a wildly different amount of real time depending on candle
spacing: a backtest-tuned period of, say, 43 on 1-hour candles is a 43-*hour* lookback,
which is ~77,400 ticks at the live poll rate — far beyond what `ctx.priceHistory` can ever
supply, so the indicator would simply never compute (`indicators.ts` returns `null` below
`period` values) and the strategy would never fire live, no matter how well those numbers
backtested. Params already measured in real elapsed time (`windowMinutes`,
`maxHoldMinutes`, `intervalMinutes`) don't have this problem. `report.ts` flags every
tick-count param with its live-tick-equivalent so this can't be missed when reading
results, and `npm run backtest -- --hold-periods` re-searches a strategy using only
live-safe params, holding periods fixed at their shipped value — see
`reports/backtest-2026-07-19.md` "The central finding" for the full writeup, including why
`strategy-engine/engine.ts`'s `ctx.priceHistory` was raised from 200 to 600 ticks as a
direct result (purely additive — more available history only lets an indicator compute
where it previously returned `null`).

**Result of the first pass:** one validated, live-safe default change shipped —
`rsi-macd`'s `overboughtRsi` (70→66) and `positionSizeUsd` (200→110), the only strategy
whose live-safe re-sweep improved on *both* the tuning and validation windows. Every
other strategy's shipped defaults were unchanged: either the sweep's best result didn't
hold up out-of-sample, depended on a period change that can't transfer live, or didn't
produce enough validation trades to trust. See `reports/backtest-2026-07-19.md` for the
full per-strategy verdict and reasoning — a strategy backtesting to flat-to-negative on
~180 real days is a genuine result, not a gap in the tuning effort.

### Dip Reversion: designing a strategy backwards from data, and what happened

The first pass only tuned existing strategies' params. A follow-up request asked for a
strategy that would actually have made money on this app's own historical data — so
before writing any strategy code, raw statistical tests were run directly against the
cached price series (`data/backtest-cache/`, price only — no fees, execution, or risk
manager modeled) to find what, if anything, has a real edge:

- **Momentum/trend-following has none, anywhere.** "Does the sign of the past N-minute
  return predict the next N-minute return", tested from 5 minutes to 168 hours on both the
  14-day 1-minute and 180-day hourly series: hit rate ≤50%, negative average forward
  return throughout. The 180-day window is also a real -42% secular decline (SOL
  $131→$76, 54% max drawdown), so a long-only strategy net-long through it is fighting the
  tape structurally.
- **Buying a confirmed dip and holding does, and it strengthens with lookback.** At
  10-20min scales the edge is real but thinner than a typical ~0.3% round-trip fee. At
  60-180min scales it looked strong: 90min lookback / 60min hold / 1.5% dip threshold
  measured a 72.9% hit rate and +0.33% average forward return across 288 samples in 14
  days of 1-minute data.

This motivated `dipReversion.ts` — the one strategy in this codebase designed backwards
from a data-driven signal rather than forwards from a TA pattern, with every param
deliberately time- or magnitude-based (never a tick count) so it wouldn't join the
tick-count-vs-live-cadence list above. Building it also justified raising
`priceCache.HISTORY_LIMIT` from 600 to 6000 ticks (~200min at the default poll rate) — the
600-tick/20min budget genuinely couldn't supply a 90-180min lookback live, so this wasn't
speculative future work anymore, it was blocking.

**That history-budget increase surfaced a real performance bug.** `rangeScalper.ts`,
`shortWindowGrid.ts`, and the new `dipReversion.ts` all located their real-time-elapsed
window with `ctx.priceHistory.filter(t => nowMs - new Date(t.timestamp).getTime() <=
windowMs)` — an O(full history) scan, cheap at 600 ticks but not at 6000, and paid on
*every* replayed tick of *every* backtest run. An Auto-tune sweep (150 trials × 2 windows
≈ 300 full backtests × ~20,000 ticks each) multiplied that into tens of billions of
comparisons and froze the live server's single-threaded event loop for several minutes —
confirmed via the process's own accumulated CPU time, not just "it seemed slow." Fixed by
adding `StrategyBase.recentWindow()`: since `priceHistory` is guaranteed oldest-first, it
scans backward from the end and stops at the first tick outside the window — O(window
size) instead of O(full history). All three strategies now use it; behavior is identical,
just no longer catastrophically slow (a 20-trial sweep that would previously not return
went from hanging indefinitely to completing in ~16s).

**With the real engine (fees, execution, risk manager, tuning/validation split) instead of
the raw price-only signal, round 1 of Dip Reversion did not validate.** A 45-day backtest:
the original defaults (90min lookback, 1.5% dip threshold) were net-negative on *both*
windows (tuning -0.09% over 87 round trips, validation -0.05% over 12). The sweep's best
config looked good on tuning (+0.21%, 6 trades, 83% win rate) but produced **zero trades
on validation** — no out-of-sample evidence, not a confirmed edge.

**Diagnosing *why* led to round 2, which does validate — thinly.** Simulating the exact
entry/exit rule fee-free (no fees, no execution, no risk manager — just "what does this
rule actually capture in the raw price") showed the average raw edge per trade was real
(+0.21%) but *smaller than the ~0.31% round-trip fee* at the shipped position size: the
signal was firing on dips too small and frequent to be worth trading, the same "fee drag
dominates at small edges" lesson `feeAwareScalper.ts` was built around. Sweeping the
fee-free simulation across dip-size thresholds found where the average edge clears the fee
floor with real margin (~0.5% avg return at a rarer, larger-dip trigger: 180min lookback —
the strategy's clamp ceiling — and a 2.5% threshold, vs. the original 90min/1.5%). Shipping
that as the new default and re-running the full fee/execution-realistic backtest: **the
first positive result on both windows of any strategy in this codebase** — tuning +0.06%
(32 round trips), validation +0.01% (3 round trips). Still thin (3 validation trades is a
small sample, not strong statistical confidence) but a genuinely different outcome than
every prior negative-or-overfit result. A further-tuned variant the sweep found looked much
better on tuning (+0.42%, 100% win rate) but again had zero validation trades — rejected on
the same standard as everything else, not adopted just because the number looked good.

See `dipReversion.ts`'s class doc for the full before/after numbers, and
`strategyMetadata["dip-reversion"].description` for the shipped, honestly-hedged summary.

`whale-copy` is not backtestable from price history alone (see "Built-in strategies"
above) and is out of scope for this backtest.

## Deferred to later phases (not built, do not assume they exist)

1. **Hot-wallet keystore + real signing** — `wallet/hotWallet.ts` is a stub that
   documents the intended design (OS keychain or passphrase-encrypted file, spending
   caps enforced at the signing layer) and throws `HotWalletNotImplementedError`.
2. **Live Jupiter swap execution** replacing `execution/simulator.ts`.
3. **Sandboxed custom-strategy scripting** (isolated worker, restricted API surface).
4. **Alerting** (Telegram/Discord/email).

Do not build any of the above without an explicit request — they involve real funds,
real signing, or new attack surface and deserve their own scoped review.
