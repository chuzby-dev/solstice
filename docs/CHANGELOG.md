# Solstice Changelog

**Purpose**: Track specification document changes, releases, and version history.

**Format**: This changelog follows [Keep a Changelog](https://keepachangelog.com/).

---

## [0.1.0-alpha] - 2026-07-21 (0.5% default spread + toggle-switch arm controls)

### Changes

Two user-requested changes, made together since the first only became
safe because of the second (per the prior slippage-decoupling entry):

1. `cross_dex_min_spread` default lowered from `0.015` (1.5%) to `0.005`
   (0.5%), matching the user's read on realistic cross-DEX spreads. Only
   reasonable now that `cross_dex_max_slippage_bps` (0.3%/leg) is
   decoupled from the general trading slippage -- doing this without that
   prior change would have let per-leg slippage tolerance alone exceed
   the entire edge.
2. Replaced the typed-confirmation gates (`ENABLE LIVE TRADING`,
   `ENABLE CROSS-DEX ARB`) on the dashboard with plain toggle switches for
   both the main kill switch and the cross-DEX arb arm control. New
   reusable `ToggleSwitch` component (`dashboard/src/components/ToggleSwitch.tsx`).
   The typed-phrase requirement was a UI affordance only, not a backend
   safety check -- the backend gates (risk limits, position caps,
   `cross_dex_arb_enabled` defaulting `false`, the kill switch itself)
   are unchanged and still the actual protection.

### Verified

`cargo fmt --all`, `cargo clippy --workspace --lib --bins --tests
--all-features -D warnings`, `cargo test --workspace` all pass clean.
`npx tsc --noEmit` in `dashboard/` passes clean. Verified live in the
browser against the (still pre-restart) running API: toggled the
cross-DEX arb switch off, confirmed the API call landed and a
`CrossDexArbEnabledChanged`/"Cross-DEX arbitrage disarmed" event streamed
into the live activity feed in real time. Did not toggle the live-trading
kill switch itself during verification -- that's a real-funds action left
to the user.

---

## [0.1.0-alpha] - 2026-07-21 (Decouple cross-DEX arb slippage from general trading slippage)

### Problem

The user pushed back on the 1.5% default `cross_dex_min_spread`, suspecting
real cross-DEX spreads run closer to 0.5%. That's a reasonable guess, but
lowering the threshold alone would have been dangerous: `execute_cross_dex_arb`
was reusing the engine's general `slippage_bps` (150bps = 1.5%, tuned for
ordinary directional trades where a missed fill just means a skipped
opportunity) as the per-leg tolerance for *both* arb legs. At a 0.5%
spread, a trade that tolerates 1.5% slippage on each leg can lose money
even when the detected spread was real -- the tolerance alone is 3x wider
than the edge being captured.

### Fix

Added `cross_dex_max_slippage_bps` to `LiveTradingConfig`, decoupled from
`slippage_bps`, default `30` (0.3% per leg, ~0.6% round-trip budget).
`find_arb_opportunity` and `execute_cross_dex_arb` now read this instead
of the general slippage setting. Wired end to end the same way as the
other cross-DEX arb controls: `set_cross_dex_max_slippage_bps`,
`LiveEvent::CrossDexMaxSlippageChanged`, `LiveStatusSnapshot` field,
`POST /api/v1/live/config` (validates 1-10000bps), and a dedicated
dashboard stat tile + input card with an explicit warning to keep it
below half of `cross_dex_min_spread`.

This doesn't itself change `cross_dex_min_spread` -- the user can now
safely lower it (e.g. toward 0.5-0.7%) once satisfied with the tighter
per-leg tolerance, without the two settings fighting each other the way
they would have before this change.

### Verified

`cargo fmt --all`, `cargo clippy --workspace --lib --bins --tests
--all-features -D warnings`, `cargo test --workspace` (`--exclude
solstice-api` plus `-p solstice-api --lib` separately, live server still
running) all pass clean. `npx tsc --noEmit` in `dashboard/` passes clean.
New test: `test_set_cross_dex_max_slippage_bps_updates_status`.

---

## [0.1.0-alpha] - 2026-07-21 (Cross-DEX arbitrage executor)

### Problem

After take-profit, the user asked why the live engine wasn't "just buying
on Raydium and selling on Orca right away" -- i.e. real cross-DEX
arbitrage. It wasn't, because nothing implemented that: `SpreadArbitrageStrategy`
only *detects* a spread and places a single directional `Buy`, betting on
convergence rather than capturing the gap directly, and execution always
routes to whichever single DEX quotes best at that instant
(`get_best_route_with_source`) -- never a deliberate buy-here/sell-there
pair.

### Design

True atomic arbitrage (buy + sell in one transaction, so a price move
between legs is impossible) would need an on-chain program that reads the
first leg's actual output and feeds it into the second leg's input --
nothing in this workspace has that, and guessing at one for real capital
was ruled out on the same grounds as the earlier Raydium encoding
caution. Built the achievable version instead: two separate, sequential,
real transactions, reusing already-proven primitives rather than
inventing anything at the instruction level -- `execute_swap` (the same
function every other live trade goes through), and
`SolanaRpcClient::get_token_balance` to read the *actual* base-token
balance delta after the buy leg lands, rather than trusting the buy
quote's `out_amount` (the two legs aren't atomic, so the real fill can
differ from what was quoted).

### What was built

`LiveTradingConfig` gained `cross_dex_arb_enabled` (default `false`) and
`cross_dex_min_spread` (default `0.015`, i.e. 1.5% -- deliberately wider
than `SpreadArbitrageStrategy`'s much smaller threshold, since two
separate swaps each pay their own fees/slippage instead of one).
`LiveTradingEngine` gained:

- `find_arb_opportunity` -- quotes every registered DEX individually for
  a pair (unlike `sample_market`, keeping the DEX name attached to each
  price instead of collapsing to an untagged list), returns the
  cheapest-vs-priciest gap.
- `evaluate_cross_dex_arbitrage` -- runs each tick, skips any pair with
  an already-open position (one attempt at a time per pair).
- `execute_cross_dex_arb` -- buys on the cheap DEX, reads the real
  balance delta, **registers the bought inventory as a tracked position
  before attempting the sell leg**, then sells on the pricier DEX. If the
  sell leg fails after the buy lands, the position stays tracked and
  protected by stop-loss/take-profit rather than becoming an orphaned
  wallet balance -- the failure mode is "still holding, still protected,"
  not "lost track of."

Gated behind its own `set_cross_dex_arb_enabled`/`set_cross_dex_min_spread`
methods, separate from the main `enable`/`disable` kill switch, since this
is a materially different risk profile (two non-atomic live transactions
per opportunity) from every other trade this engine makes. Off by
default. Wired end to end: new `LiveEvent` variants
(`CrossDexArbEnabledChanged`, `CrossDexMinSpreadChanged`,
`CrossDexOpportunityDetected`, `CrossDexArbFilled`, `CrossDexArbFailed`),
`LiveStatusSnapshot` fields, the `POST /api/v1/live/config` handler, and
a dedicated dashboard card on `LiveTradingPage.tsx` with its own
`ENABLE CROSS-DEX ARB` typed-confirmation arm switch (mirroring the main
kill switch's confirm-phrase pattern), separate from the regular
max-capital/min-confidence/take-profit controls.

### Verified

`cargo fmt --all`, `cargo clippy --workspace --lib --bins --tests
--all-features -D warnings`, and `cargo test --workspace` (via
`--exclude solstice-api` plus `-p solstice-api --lib` separately, the
established workaround for the running `serve.exe` binary lock) all pass
clean. `npx tsc --noEmit` in `dashboard/` passes clean. New tests:
`test_set_cross_dex_arb_enabled_updates_status`,
`test_set_cross_dex_min_spread_updates_status`,
`test_evaluate_cross_dex_arbitrage_noop_when_disabled` (confirms no
network I/O is attempted while disarmed), and
`test_evaluate_cross_dex_arbitrage_skips_pair_with_open_position`.
`find_arb_opportunity`/`execute_cross_dex_arb`'s network-dependent paths
aren't unit-tested directly -- consistent with how `execute_planned_trade`
itself isn't either in this codebase; only the pure gating/config logic
is. Verified visually in the browser: the dashboard's new stat tiles,
min-spread input, and arm/disarm card render correctly against the live
API, with the existing event feed, kill switch, and other controls
unaffected.

At the time this landed, the live server was still running the
pre-take-profit build with a real, tracked $15 SOL/USDC position and live
trading re-armed by the user. `cross_dex_arb_enabled` defaults to `false`,
so this feature does nothing until explicitly armed via the dashboard --
rebuilding/restarting the server to pick up this change (and take-profit,
still not yet live either) was held pending explicit user confirmation,
per the established restart safety practice.

---

## [0.1.0-alpha] - 2026-07-20 (Phase 1.2 update)

### Phase 1.2 - Market Data Ingestion ✅ COMPLETE

**First validated build**: the workspace had never been compiled before this
pass. Fixed the fallout from that (missing dependency, borrow-checker error,
an averaging bug and an off-by-one in cache TTL expiration, floating-point
exact-equality assertions, and assorted clippy lints) so `cargo fmt`,
`cargo clippy --all-targets --all-features -D warnings`, and
`cargo test --workspace` all pass clean. See `crates/*` history for detail.

**Solana SDK upgrade**: bumped `solana-sdk`/`solana-client` from 1.18 to 2.2
workspace-wide. The Yellowstone gRPC crates are version-locked to a Solana
SDK generation, and the current Yellowstone protocol needs `solana-sdk` 2.x;
staying on 1.18 would have meant building the streaming adapter against a
year-old, soon-unsupported combination. `tonic`/`prost` moved to 0.14 to
match. No API-visible changes in `solstice-core` or `solstice-blockchain`
beyond the dependency bump.

**Yellowstone gRPC adapter** (`solstice-market-data::yellowstone`):
- `YellowstoneConfig` - endpoint pool (primary + fallback), timeouts,
  backoff parameters, bounded delivery channel size, optional `x-token` auth
- `AccountFilter` - include/exclude/owner-program/min-lamports criteria,
  translated to the wire-format `SubscribeRequestFilterAccounts` and
  re-checked client-side against every inbound update (the exclude and
  min-lamports criteria have no server-side equivalent)
- `YellowstoneClient` - subscribes over the real `yellowstone-grpc-proto`
  generated tonic client, with:
  - automatic reconnection across the endpoint pool with exponential
    backoff on connection/stream errors
  - ping/pong keepalive handling
  - health tracking based on time since the last received message
  - backpressure via a bounded `tokio::mpsc` channel: a full channel makes
    ingestion await rather than drop updates or buffer unboundedly
- `YellowstoneParser` - parses `SubscribeUpdateAccount` into a new
  `MarketEvent::AccountUpdate` core event (raw account state; protocol-
  specific decoding for individual DEXes/oracles is added with those
  integrations in Phase 2)
- Note: the community `yellowstone-grpc-client` convenience crate assumes a
  Unix target (`tokio::net::UnixStream`) and does not build on Windows, so
  the adapter is built directly on `yellowstone-grpc-proto` + `tonic`
  instead of that wrapper.
- Windows build note: `solana-secp256r1-program` (pulled in by `solana-sdk`
  2.x) links against OpenSSL; building on Windows requires OpenSSL
  installed with `OPENSSL_DIR`/`OPENSSL_LIB_DIR`/`OPENSSL_INCLUDE_DIR` set
  (this environment uses the `ShiningLight.OpenSSL.Dev` winget package).

**Ready for**: Phase 1.4 (Storage Infrastructure)

---

## [0.1.0-alpha] - 2026-07-20 (Phase 1.4 update)

### Phase 1.4 - Storage Infrastructure ✅ COMPLETE — Phase 1 gate reached

New `solstice-storage` crate. `docs/DATABASE.md` and
`docs/REDIS_ARCHITECTURE.md` referenced by `WORKSPACE.md` don't exist yet,
so the schema and cache API below were designed from `WORKSPACE.md`'s
`solstice-storage` summary (public API shape, responsibilities, key
components) rather than a detailed spec.

**Schema** (`migrations/0001_init.sql`, applied via `sqlx::migrate!`):
- `market_snapshots` — time-series price observations, hypertable on `time`
- `trades` — completed trade records
- `position_updates` — position state history (one row per recorded update)
- `account_snapshots` — raw Yellowstone account state, hypertable on `time`
- `TimescaleDB` extension is enabled and hypertables created when available;
  falls back to ordinary tables if the extension isn't installed, so the
  migration doesn't hard-fail against a plain Postgres.

**`StoragePool`** (Postgres/TimescaleDB, via `sqlx`):
- `save_market_snapshot` / `get_market_data(base, quote, TimeRange)`
- `save_trade` / (trade lookups go through `get_market_data` today; a
  dedicated trade query surface lands with the execution engine in Phase 4)
- `save_position_update` / `get_position_history(PositionId)`
- `save_account_snapshot` / `get_latest_account_snapshot`
- Runtime (non-macro) `sqlx::query`/`query_as`, not `query!`, so the crate
  builds without a live `DATABASE_URL` at compile time.

**`CacheManager`** (Redis, via `redis` + `ConnectionManager`):
- `get`/`set`/`set_default_ttl`/`delete`/`exists`, `get_json`/`set_json`
  convenience wrappers, `publish` for pub/sub, key-prefix namespacing.

**Row/domain conversions** (`models.rs`): `u64` core fields (token
quantities, lamports) convert to Postgres `BIGINT` (`i64`) via `TryFrom`,
returning `StorageError::ValueOutOfRange` instead of truncating silently.

**Test strategy**: this environment has no running Postgres or Redis (Docker
is installed but the daemon isn't running). Pure logic — config builders,
row/domain conversions, TTL math, error mapping — has real unit test
coverage. Connection-requiring behavior lives in
`tests/integration_tests.rs`, `#[ignore]`'d with a doc comment on how to
spin up local containers and run them (`cargo test -p solstice-storage --
--ignored`).

**Phase 1 gate reached**: core infrastructure (workspace, core types,
blockchain RPC/transactions, market data ingestion incl. Yellowstone,
storage) all compile, lint, and test clean.

**Ready for**: Phase 2 (DEX Integration)

---

## [0.1.0-alpha] - 2026-07-20 (Phase 2.1 update)

### Phase 2.1 - Jupiter Integration ✅ COMPLETE

New `solstice-dex` crate, following `docs/DEX_INTEGRATIONS.md`'s unified
`DexClient` trait (via `async-trait` for object safety — `Arc<dyn
DexClient>` is stored in the aggregator).

**`JupiterClient`**: real Jupiter Quote/Swap-Instructions API v6 integration.
- `get_quote` calls `GET /quote`, parses the actual response schema
  (`inAmount`/`outAmount`/`priceImpactPct`/`routePlan[].swapInfo`), and
  derives `fee_bps` from the summed per-leg `feeAmount`.
- `build_swap_instructions` calls `POST /swap-instructions` (not the spec
  doc's fictional `swap.tx_instructions` on `/swap` — the real `/swap`
  endpoint returns a fully-assembled serialized transaction, not an
  instruction list; `/swap-instructions` is the endpoint that actually
  returns one) and decodes compute-budget/setup/swap/cleanup instructions
  from base64. Address lookup tables in the response are detected and
  logged but not resolved — building a versioned transaction from them is
  an execution-layer concern for Phase 4.
- `subscribe_prices` polls the quote endpoint on an interval (Jupiter has
  no push feed) rather than leaving the trait method unimplemented.

**`DexAggregator`**: queries all registered `DexClient`s concurrently via
`tokio::spawn`, picks the highest-output quote, logs and skips DEXes that
error rather than failing the whole request. `RouteCache` is a real
TTL+LRU cache (via the `lru` crate) keyed on (input mint, output mint,
amount), not the spec doc's cache-everything-forever-until-a-manual-clear
sketch.

**Test strategy**: this environment can reach the crates.io registry but
not arbitrary hosts (`api.jup.ag` connections fail outright — confirmed
directly), so live-network tests are `#[ignore]`'d in
`tests/integration_tests.rs`. Response parsing, fee/slippage math, and
aggregator selection logic are unit-tested against realistic fixture JSON
and mock `DexClient` implementations instead.

**Deferred to 2.2/2.3**: Raydium, Orca, OpenBook, Meteora, Phoenix — each
requires parsing that protocol's own on-chain account layouts (and,
likely, its own SDK crate with its own dependency-resolution risk, per
the Yellowstone/solana-sdk experience in Phase 1.2).

**Ready for**: Phase 2.2 (Primary DEXes: Raydium, Orca, OpenBook)

---

## [0.1.0-alpha] - 2026-07-20 (Phase 2.2 partial update)

### Phase 2.2 - Primary DEXes: Raydium ✅ (Orca, OpenBook not started)

**solstice-blockchain gap fix**: `SolanaRpcClient` (Phase 1.3) only ever did
endpoint selection and health tracking — nothing actually called a live
Solana node. Added `get_account`/`get_multiple_accounts`, wrapping
`solana_client::nonblocking::rpc_client::RpcClient` and routed through the
existing endpoint failover/health tracking (success/error recorded per
attempt, retried across the endpoint pool up to `max_retries`). Every
future on-chain DEX integration needs this, not just Raydium.

**`RaydiumClient`** (`solstice-dex::raydium`): real constant-product AMM v4
integration against the `raydium_amm` crate (IDL-generated, solana-sdk
2.x-native — its `Pubkey` unifies with ours in the dependency graph, no
conversion needed).
- `get_quote` fetches the pool account and both vault token accounts over
  RPC, reads reserves via SPL Token's stable account layout (amount at
  byte offset 64), and applies Raydium's actual constant-product formula
  with the pool's actual on-chain `swap_fee_numerator/denominator`.
- Pool addresses aren't derivable from a mint pair, so `RaydiumClient`
  holds a small pool registry (`register_pool`) rather than guessing or
  deriving one — population from config/discovery is a later task.
- `build_swap_instructions` deliberately returns a descriptive error
  instead of a guess: Raydium's `SwapBaseIn` instruction also needs the
  pool's underlying OpenBook/Serum market accounts (bids/asks/event
  queue/vault signer), and the only crate for that layout (`serum_dex`)
  is pinned to a 2022-era Solana SDK incompatible with this workspace.
  Hand-rolling that layout from memory for a real funds-moving
  instruction was judged too risky to guess at (confirmed with the user
  before proceeding this way).

**Not started**: Orca (`orca_whirlpools_client`/`_core` exist and are
actively maintained, but pin `solana-*` crates on the `^3` line — one
major version ahead of this workspace's `2.2`, so `Pubkey` values need
explicit byte-level conversion at the boundary, unlike Raydium) and
OpenBook (blocked on the same stale `serum_dex`/`openbook-v2` dependency
problem noted above).

**Ready for**: Orca integration, or moving on to Phase 2.3/3 depending on
priority — flagged to the user rather than assumed.

---

## [0.1.0-alpha] - 2026-07-20 (Phase 2.2 continued: Orca)

### Phase 2.2 - Primary DEXes: Orca ✅ (OpenBook not started)

**`OrcaClient`** (`solstice-dex::orca`): real concentrated-liquidity
(Whirlpools) integration against `orca_whirlpools_client` +
`orca_whirlpools_core` (both actively maintained, IDL-generated).
- `get_quote` fetches the pool account and up to three surrounding
  tick-array accounts (the one containing the current tick, plus its
  immediate neighbors — arrays that were never initialized on-chain are
  simply omitted, not treated as an error), then calls
  `orca_whirlpools_core::swap_quote_by_input_token` to do the actual
  tick-crossing/fee/sqrt-price math. That math is Orca's own vetted
  implementation, not a reimplementation of CLMM math here — this
  integration's job is fetching the right accounts and calling it
  correctly, not re-deriving the math itself.
- `get_liquidity` reports both vault balances directly.
- **Cross-major-version `Pubkey` conversion**: unlike `raydium_amm`,
  `orca_whirlpools_client` pins `solana-pubkey` on the `3.x` line (one
  major version past this workspace's `solana-sdk` 2.x, which resolves
  `solana-pubkey` 2.x) — Cargo treats them as distinct types even though
  `solana-pubkey` 3.0 is just `pub use solana_address::Address as
  Pubkey`. Added `solana-pubkey` (v3, renamed to `solana-pubkey-v3` in
  Cargo.toml to avoid colliding with the workspace's implicit 2.x) as a
  direct dependency purely so the boundary conversion helpers
  (`to_sdk_pubkey`/`to_orca_pubkey`, byte-level via `to_bytes()`/`from()`)
  have a name to reference.
- `build_swap_instructions` is not implemented, for the same class of
  reason as Raydium: the on-chain `SwapV2` instruction needs three
  tick-array accounts in an order that depends on swap direction, and
  this integration cannot confirm that ordering convention against a
  reference here. Unlike Raydium's gap (blocked on a stale external
  crate), this one *could* be closed by finding/testing the right
  convention — flagged as a follow-up rather than guessed at.

**Not started**: OpenBook (still blocked on the stale `serum_dex`/
`openbook-v2` dependency problem from the Raydium entry above).

**Ready for**: resolving Orca's swap-instruction ordering, OpenBook, or
Phase 2.3/3 — flagged to the user rather than assumed.

---

## [0.1.0-alpha] - 2026-07-20 (Phase 2.3 assessment + Phase 3.1)

### Phase 2.3 - Secondary DEXes: assessed, not implemented

Checked Meteora and Phoenix before writing code. Phoenix's only available
crates (`phoenix-sdk`, `phoenix-v1`) are pinned to Solana SDK 1.14.x — same
blocked class as OpenBook. Meteora's `meteora-dlmm-sdk` is actively
maintained and solana-sdk-2.x-era (would need the same `solana-pubkey`
byte-conversion pattern used for Orca), but unlike Orca it's *only*
account/instruction layout generated from the IDL — there's no
accompanying math crate for DLMM's bin-walking swap algorithm the way
`orca_whirlpools_core` exists for Orca's concentrated-liquidity math.
Implementing it correctly would mean hand-rolling that algorithm from
memory with no reference to verify against, the same risk avoided for
OpenBook/Phoenix/Raydium's and Orca's swap instructions. Not attempted.
Jupiter + Raydium + Orca quoting is where Phase 2 stands.

### Phase 3.1 - Strategy Framework ✅ COMPLETE

New `solstice-strategy` crate, reusing `solstice-core`'s existing domain
types (`Signal`, `SignalType`, `Position`, `OrderBook`, `Price`,
`TokenPair`) rather than defining a parallel, conflicting set the way
`docs/STRATEGY_FRAMEWORK.md`'s sketch does.

**One deliberate deviation from the spec**: `StrategyManager` does not
dynamically load `.so`/`.dll` plugins via `libloading` +
`extern "C" fn create_strategy()`. Rust has no stable ABI across compiler
versions, so that pattern typically produces undefined behavior (not a
clean error) when a plugin is built with a different rustc than the host
— and this workspace has no compiled plugin binary to validate such
loading against regardless. `register_strategy` instead takes an
already-constructed `Arc<dyn Strategy>`; strategies are Rust crates
compiled into the host (or, for real hot-reload, run out-of-process
behind an RPC boundary) — the pattern most production Rust plugin
systems converge on for the same ABI-stability reason. Documented in
`manager.rs`; dynamic loading can be added later if a real need appears.

**Also adapted, not copied verbatim, from the spec**:
- `MarketSnapshot.prices` is `HashMap<TokenPair, Vec<Price>>` (one entry
  per source/DEX), not a single collapsed price per pair — the spec's own
  `SpreadArbitrageStrategy` example needs multiple price *observations of
  the same pair* to detect a spread, but its `MarketSnapshot` sketch (one
  price per token) can't represent that. Its actual example code compares
  prices of two *different*, unrelated tokens against each other, which
  isn't arbitrage detection at all.
- `SimpleMovingAverageStrategy` maintains its own rolling price window
  internally (`Mutex<VecDeque<f64>>`), fed one point per `evaluate` call
  — a `MarketSnapshot` is a point-in-time view, so nothing else in the
  spec's sketch explains where SMA's historical series would come from.

**Delivered**: `Strategy` trait (via `async-trait` for object safety),
`StrategyManager` (register/unregister with lifecycle hooks, concurrent
`evaluate_all` via `tokio::spawn` — one strategy panicking or erroring
doesn't affect the others), `SignalValidator`, `SignalDeduplicator`
(TTL-based, keyed on signal id), `SignalRanker` (confidence descending),
and two real reference strategies (`SimpleMovingAverageStrategy`,
`SpreadArbitrageStrategy`) with actual signal-generating logic, not stubs.

**Ready for**: Phase 3.2 (Fair Value Engine), 3.3 (Statistical
Arbitrage), or 3.4 (Portfolio Management).

---

## [0.1.0-alpha] - 2026-07-20 (Phase 3.2-3.4)

### Phase 3.2 - Fair Value Engine ✅ COMPLETE

`FairValueEngine::compute_fair_value` blends multiple price observations
of the same pair into one fair-value estimate, weighted by both
confidence and recency (exponential half-life decay — configurable, so a
short half-life trusts only very recent observations and a long one
treats everything recent-ish equally). Combining several low-confidence
observations does not itself produce a high-confidence result: output
confidence is the weight-averaged input confidence, not inflated by
source count.

### Phase 3.3 - Statistical Arbitrage ✅ (cointegration deferred)

`StatArbEngine` accumulates its own rolling price history per pair (fed
via `observe`, since — like the SMA strategy — a `MarketSnapshot` is a
single point in time with nowhere else for a series to live) and detects:
- **Mean reversion**: current price's z-score against the pair's rolling
  mean/stddev; opportunities above a configurable threshold.
- **Correlation**: Pearson correlation between every pair of observed
  price series; pairs above a configurable threshold are flagged as
  pairs-trading candidates.

**Cointegration detection** (also named in `WORKSPACE.md`'s summary) is
not implemented: a correct implementation needs an ADF (Augmented
Dickey-Fuller) unit-root test, which is easy to get subtly wrong without
a statistics crate to check the implementation against — the same
"don't hand-roll unverifiable math" reasoning applied to Meteora's DLMM
swap algorithm and OpenBook's account layout. Flagged as a follow-up.

### Phase 3.4 - Portfolio Management ✅ COMPLETE

`PortfolioManager` computes per-pair concentration (position value ÷
total portfolio value) and emits `SignalType::Rebalance` signals for any
pair exceeding a configurable maximum concentration. Cross-asset
correlation-based limits (as opposed to plain concentration limits) await
3.3's deferred cointegration/correlation-stability work — flagged, not
silently dropped.

**Test note**: `stat_arb`'s correlation test initially used sample data
that was accidentally perfectly anti-correlated (r = -1.0) rather than
uncorrelated, which the "uncorrelated pairs aren't flagged" test caught
immediately — worth calling out since it's exactly the kind of thing a
statistics implementation needs real test coverage to catch, not just
code review.

**Ready for**: Phase 4 (Execution & Risk), or returning to the deferred
cointegration/DEX gaps.

---

## [0.1.0-alpha] - 2026-07-20 (Phase 4)

### Phase 4.1 - Position Sizing ✅ COMPLETE

New `solstice-execution` crate. `PositionSizer::calculate_size` uses
fractional Kelly criterion (`f* = p - (1-p)/b`, clamped to `[0, 1]`,
scaled by a configurable `kelly_fraction` for safety — full Kelly is
aggressive and rarely appropriate) with the signal's `confidence` as win
probability, then clamps the result against every hard limit: an
explicit `suggested_size` hint on the signal, max position size/percent,
and available capital. Never suggests a negative or over-bankroll size.

### Phase 4.2 - Risk Management ✅ COMPLETE

Direct implementation of `docs/RISK_MANAGEMENT.md`: `PositionLimits`,
`DailyLossLimits`, `ExposureLimits`, `ConcentrationLimits`, `OrderLimits`
as pure checks with no I/O or shared state, composed by
`PreTradeRiskChecker::check_before_trade`. `RiskMonitor` tracks portfolio
risk snapshots over time and trips a circuit breaker on daily-loss
breach — per the spec's fail-safe philosophy, nothing in this crate
resets it automatically; `reset_circuit_breaker` is the only way back,
and it's on the caller to invoke it deliberately. `StopLossManager`
flags long positions that have fallen past a configurable loss
threshold (short-position stop logic is inverted and isn't implemented,
since nothing in this workspace opens shorts yet).

One deviation from the spec: `PreTradeRiskChecker` doesn't fetch a quote
from a `DexAggregator` itself the way the spec's sketch does — the
simulated slippage is passed in by the caller instead. Risk checks stay
pure/synchronous; fetching a quote is an I/O concern that belongs to the
execution planner, not the risk checker.

### Phase 4.3 - Execution Planning ✅ (partial)

`ExecutionPlanner::plan` extracts a signal's token pair (`Buy`/`Sell`
only — `Close`/`Rebalance` signals don't concern a single pair the same
way and have no plan through this path), fetches the best route via
`solstice-dex`'s `DexAggregator`, estimates slippage, and runs
`PreTradeRiskChecker` against it — returning an `ExecutionPlan` whose
`approval` field records the outcome (a plan that fails risk checks is
still `Ok`, not an `Err`, so callers can inspect/log why). Does not yet
build a submittable transaction: that's blocked on the DEX
swap-instruction gaps already noted in the Phase 2.2/2.3 entries above
(Raydium/Orca instruction building deferred, OpenBook/Phoenix/Meteora
not integrated), and multi-leg/split order routing isn't implemented.

### Phase 4.4 - Order Management ✅ COMPLETE (in-memory)

`OrderManager` tracks orders through `Submitted → PartiallyFilled →
Filled` (or `→ Failed`/`Cancelled`), rejecting fills against terminal
orders and rejecting submission of any plan whose `approval` wasn't
`Approved` — an order should never exist for a trade the risk checker
didn't clear. State lives in memory only; persistence to
`solstice-storage`'s `trades`/`position_updates` tables is a follow-up.

**Ready for**: closing the Phase 4.3 transaction-building gap (once a
DEX swap-instruction path is available), wiring `OrderManager` to
storage persistence, or moving to Phase 5+ (Jito/MEV, Simulation, APIs).

---

## [0.1.0-alpha] - 2026-07-20 (Phase 6.3, out of roadmap order)

### Phase 6.3 - Paper Trading Mode ✅ (live-quote path only)

User explicitly asked to prioritize getting to a runnable live-data demo
over roadmap sequencing. Skips Phase 5 (Jito/MEV) and 6.1/6.2 (event-loop
replay engine, simulated slippage/partial fills) entirely — this is a
live-quote paper-trading loop, not the replay-based simulation engine
those milestones describe.

New `solstice-simulation` crate with a runnable binary:

```sh
cargo run -p solstice-simulation --bin paper-trade
```

`PaperTradingEngine` polls `RaydiumClient`/`OrcaClient` (from Phase 2.2)
for real on-chain SOL/USDC quotes every 15s, via a user-supplied Helius
RPC endpoint (`HELIUS_RPC_URL` in a local, gitignored `.env`). Feeds the
result through `FairValueEngine` and `StatArbEngine` (Phase 3.2/3.3),
`StrategyManager` (SMA + spread-arb strategies), `PositionSizer` +
`PreTradeRiskChecker` (Phase 4.1/4.2), and `OrderManager` (Phase 4.4) —
every piece of the platform built so far, wired end to end against real
market data, with no real transaction ever built or submitted (fills are
simulated at the quote's own execution price).

**Verified before wiring in, not trusted from memory**: the Raydium
(`58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2`) and Orca
(`Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE`) SOL/USDC pool addresses
were fetched live via `getAccountInfo` and checked (owner program,
account size, and — for Orca — the SOL/USDC mint bytes at their expected
struct offsets) before use, after an initial guessed Raydium address
turned out to be wrong (returned no account). The correct address came
from Raydium's own `api-v3.raydium.io` pool-lookup endpoint, then
independently confirmed on-chain.

**Bugs caught by actually running it against live data** (not something
review alone would have caught):
- **Stack overflow** on Windows debug builds: Orca's tick-array value
  types are several KB each and get moved through a few layers of async
  calls, overflowing the default ~1MB thread stack. Fixed by running the
  tokio runtime on a dedicated 16MB-stack thread rather than the default
  `#[tokio::main]` setup.
- **Unbounded position accumulation**: the pre-trade position-size check
  only validated the *new* trade's size against the cap, not size-so-far
  in that pair (a limitation inherited from `docs/RISK_MANAGEMENT.md`'s
  own sketch, which has the same gap) — so a strategy re-signaling every
  tick would re-buy up to the cap every single cycle instead of stopping
  once the cap was reached. Fixed by tracking existing per-pair exposure
  and sizing against remaining headroom.

**Known simplifications, not silently hidden**: one position per pair
(no averaging across multiple entries), instant fills at quoted price (no
slippage/partial-fill modeling — that's Phase 6.2's job), console-only
output (no metrics/API endpoint yet — that's Phase 7).

**Ready for**: Phase 7 (REST/WebSocket API) to expose this engine's state,
then Phase 8 (React dashboard) for the GUI the user is aiming for.

---

## [0.1.0-alpha] - 2026-07-21 (Phase 7)

### Phase 7.1/7.2 - REST + WebSocket API ✅ (core paths; configuration
endpoints, OpenAPI docs, and metrics not yet done)

New `solstice-api` crate. Second runnable binary:

```sh
cargo run -p solstice-api --bin serve
```

Runs the same SOL/USDC paper-trading engine as `paper-trade` (factored
out into `solstice_simulation::build_sol_usdc_demo_engine` so the two
binaries share one source of truth for pool addresses/risk config)
alongside an Axum server on `127.0.0.1:8080` (override via
`SOLSTICE_API_ADDR`):

- `GET /api/v1/status` — running state, monitored pairs, open position
  count, total value, circuit-breaker status
- `GET /api/v1/positions` — current simulated positions
- `GET /api/v1/trades` — full order history (all statuses, newest first)
- `GET /api/v1/performance` — cash/realized/unrealized P&L, total value
- `WS /api/v1/ws` — every `EngineEvent` (price update, signal generated,
  order filled) as newline-delimited JSON, broadcast to all connected
  clients

**Response DTOs, not raw internal types**: `solstice-api::dto` defines
its own response shapes rather than serializing `Order`/`Quote`/
`TradeApproval` directly — an API response is a contract with clients
and shouldn't shift just because an internal refactor changes a domain
type's fields. `PaperTradingEngine` gained `EngineEvent` (broadcast
channel, best-effort — a slow/absent subscriber never affects trading)
and `PortfolioSnapshot`/`PositionSnapshot` (JSON-friendly views) to
support this without leaking its internals either.

**No authentication**: matches `WORKSPACE.md`'s `solstice-api` summary in
listing auth as a responsibility, but none is implemented — this is a
local paper-trading demo, not something to expose beyond a trusted
network. Flagged, not silently omitted.

**Verified end to end, not just built**: ran `serve`, confirmed the
engine traded (`SpreadArb` filled a real signal off the live Raydium/Orca
spread), and hit all four REST endpoints with `curl` while it was running
— `/positions` and `/trades` reflected the actual simulated fill from the
live session, not fixture data.

**Ready for**: Phase 8 (React/TypeScript dashboard) — the GUI the user is
aiming for — consuming this API's REST endpoints and WebSocket stream.

---

## [0.1.0-alpha] - 2026-07-21 (Phase 8)

### Phase 8.1/8.2/8.3 - React Dashboard ✅ (8.4 control interface deferred)

New `dashboard/` app: React 19 + TypeScript + Vite, Tailwind v4, React Router,
Recharts. This is the professional simulation GUI the user's mid-session pivot
was aiming for — a live view onto the paper-trading engine running behind
`solstice-api`, not a mock or a storyboard.

```sh
cargo run -p solstice-api --bin serve   # terminal 1 — engine + API on :8080
npm run dev --prefix dashboard          # terminal 2 — dashboard on :5173
```

The Vite dev server proxies `/api/*` (including the WebSocket upgrade) to
`127.0.0.1:8080`, so the dashboard talks to the real API with no CORS
workaround needed in development.

**Pages** (`HashRouter`, four routes under a shared `Layout` sidebar/topbar):
- **Overview** — status/pairs/positions/portfolio-value stat tiles, a live
  Raydium-vs-Orca price chart built by folding the WebSocket event stream,
  and a scrolling activity feed of every `EngineEvent`
- **Positions** — polls `GET /positions` every 5s
- **Trades** — polls `GET /trades` every 5s, color-coded order status
- **Performance** — polls `GET /performance` every 5s; stat tiles plus a
  portfolio-value-over-time chart accumulated client-side from repeated polls
  (the API itself has no historical-series endpoint, so this is a session-local
  view, not a query against stored history)

**Data flow**: a small typed API client (`src/api/client.ts`, DTOs hand-mirrored
from `solstice-api::dto` in `src/api/types.ts`) backs a `usePolling` hook for
the REST pages, and a `useEngineEvents` WebSocket hook (auto-reconnect, capped
200-event rolling buffer) feeds the live Overview chart and activity feed.

**Color/chart methodology**: built per the `dataviz` skill's validated default
palette — categorical hues in fixed order (Raydium = series-1/blue, Orca =
series-6/orange), status colors reserved for order/connection state, dark-mode
CSS custom properties, thin 2px lines, legend + tooltip on both charts.

**8.4 (control interface) deferred, not built**: the roadmap's Phase 8.4 calls
for configuration management, strategy selection, start/stop controls, and
manual order submission. `solstice-api` currently exposes only read-only
endpoints (status/positions/trades/performance/ws) — there is no mutating
surface for the dashboard to call. Building a control UI against endpoints
that don't exist would mean either a fake/no-op UI or scope-creeping into new
backend work the user hasn't asked for. Left as explicit future work.

**Verified end to end, not just built**: ran `cargo build`/`tsc -b`/`vite build`
clean, then ran both the real `serve` binary (live Helius mainnet data) and
`vite dev` together and drove all four pages in a browser — confirmed live
portfolio value, an actual `SpreadArb` fill, live Raydium/Orca price ticks on
the chart, and the WebSocket reconnect badge going Connecting → Live, all
against genuine engine state rather than fixtures.

---

## [0.1.0-alpha] - 2026-07-21 (Phase 6.1/6.2/6.4)

### Phase 6.1 - Simulation Engine ✅, 6.2 - Order Simulation ✅, 6.4 - Backtesting Engine ✅

The historical-replay backtesting these milestones call for, deliberately
skipped back in Phase 6.3 in favor of a runnable live-paper-trading demo. New
`solstice_simulation::backtest` module and a second runnable binary:

```sh
cargo run -p solstice-simulation --bin backtest -- data.csv --short 5 --long 20 --capital 10000 --out report.json
```

`data.csv` is a two-column `timestamp,price` CSV (RFC3339 timestamps) — the
common export shape for on-chain price history, since nothing in this
workspace ingests a specific vendor's historical-data API and
`solstice-storage`'s `market_snapshots` table only has data for pairs this
platform has itself observed live.

**A second engine, not a generalized one**: `BacktestEngine` is a new type
alongside `PaperTradingEngine`, not a refactor of it into one engine with two
data sources. `PaperTradingEngine` fills instantly at a live DEX quote's exact
price because that quote already reflects real venue liquidity and fees; a
bare historical price point carries none of that, so a backtest needs its own
execution-cost model (`backtest::fill_model`) or it silently overstates every
strategy's performance with free, instant, unlimited-size fills. Forcing both
onto one abstraction would have meant threading that cost model through the
live path too, for no benefit — trying to average `∞ liquidity, 0 cost` and
`configurable slippage/fees/partial fills` into a shared code path was worse
than two engines that each say plainly what they model. `BacktestEngine` does
reuse the same strategy → `PositionSizer` → `PreTradeRiskChecker` →
`OrderManager` pipeline `PaperTradingEngine` uses, just single-threaded
(`&mut self`, no `Arc<Mutex<_>>`/broadcast channel) since a replay is one
sequential pass with one caller, not something a concurrently-polling API
server needs to share.

**Order simulation** (`backtest::fill_model`): `SlippageModel` (none / fixed
bps / size-scaled bps against a reference notional), `FeeModel` (flat
proportional fee), `PartialFillConfig` (caps how much of an order fills per
tick, so a large order spreads across several ticks — `PartiallyFilled` —
instead of filling instantly against one bare price point with no real depth
information to justify that). All three are configurable, not fit to any
specific real venue's actual microstructure — a caller who wants that must
supply their own numbers.

**Performance calculation & report generation** (`backtest::report`):
`PerformanceMetrics` — total return, max drawdown, a per-tick Sharpe ratio
(explicitly documented as *not* annualized, since replay tick spacing is
whatever the input data uses, not a fixed period), fill/fee counts, and win
rate over closed positions. `BacktestReport::to_json_pretty()` for machine
consumption and `to_markdown()` for a human-readable summary — the CLI prints
the latter and can write the former to a file via `--out`.

**Closed positions come only from stop-loss exits, matching a known live-engine
limitation**: no strategy shipped in this workspace (`SMA`, `SpreadArb`) emits
a `Sell`/`Close` signal — both only ever emit `Buy`. `win_rate` and
`num_closed_positions` will read `0`/`None` for a backtest where nothing
triggered a stop loss, which is a limitation of the strategies, not the
backtest engine; flagged here rather than silently under-reported.

**Parameter optimization framework** (`backtest::optimize::optimize_grid`):
sweeps caller-constructed strategy *instances* (e.g. several
`SimpleMovingAverageStrategy::new(pair, short, long)` with different window
sizes), not `StrategyConfig::strategy_config`'s `serde_json::Value` blob —
every strategy implementation in this workspace ignores that argument
entirely, so sweeping it would change nothing. Runs each candidate against
the same historical ticks with a fresh `StrategyManager` (no state leaks
between runs) and ranks results by a caller-supplied metric function.

**Storage-backed historical loading intentionally not built**: `solstice-storage`'s
`get_market_data` could in principle back a second data source alongside CSV,
but there's no live Postgres in this sandbox to build and verify that against,
and per this session's established pattern, unverified DB integration code
for a data path a backtest's correctness depends on isn't worth the risk of
guessing at. CSV loading is fully implemented and tested; storage-backed
loading is documented here as follow-up work, not silently attempted.

**Verified end to end**: `cargo fmt --check`, `cargo clippy --workspace
--all-targets --all-features -D warnings`, and `cargo test --workspace` all
pass clean (22 new tests: fill-model math, CSV parsing/validation,
equity-curve/Sharpe/drawdown/win-rate computation, and full engine replays —
including one against the real `SimpleMovingAverageStrategy` that asserts it
actually buys into a synthetic uptrend and a stop-loss scenario that asserts
a crash actually closes the position at a loss). Also ran the `backtest`
binary against a generated 500-point synthetic random-walk CSV and confirmed
real fills, a real equity curve, and a real Markdown/JSON report — not just a
clean compile.

---

## [0.1.0-alpha] - 2026-07-21 (Phase 5)

### Phase 5.1 - Jito Integration ✅ (transport layer), 5.2 - MEV Protection ✅ (partial), 5.3 - Settlement & Monitoring ✅ (partial)

A Jito Block Engine client for MEV-protected bundle submission — new
`solstice_execution::jito` module. This is deliberately scoped as a
**transport layer**: it bundles, tips, submits, and confirms already-signed
transactions, regardless of what those transactions do. It cannot by itself
turn a signal into an on-chain trade, and here's exactly why:

**No swap-instruction building exists anywhere in this workspace.**
`solstice-dex`'s `Quote`/`RouteSegment` (used by every strategy/execution
path so far) carry pricing and routing *metadata only* — no program ID, no
account list, no instruction data. Building a real Raydium/Orca/Jupiter swap
instruction is new capability this phase doesn't add, consistent with this
session's standing rule: don't guess at account layouts/orderings for
money-moving instructions. A Jito bundle here is built from whatever
already-signed `Transaction`s the caller supplies — this module doesn't care
what's in them.

**What's built and how it was verified**:
- `jito::Bundle` — an ordered, capped (5-transaction) set of transactions to
  submit atomically. Cap enforcement is unit-tested.
- `jito::TipStrategy` — `Fixed(lamports)` or `BpsOfNotional{..}` (clamped
  min/max), and `build_tip_instruction` — a plain `system_instruction::transfer`
  to a tip account. Tip accounts are never hardcoded: `JitoClient::get_tip_accounts`
  queries the Block Engine's `getTipAccounts` live. **This one call was
  verified against the real endpoint** (`https://mainnet.block-engine.jito.wtf/api/v1/bundles`)
  while building it — an earlier version pointed at the wrong path
  (`/api/v1` instead of `/api/v1/bundles`) and failed with a JSON decode
  error until corrected against the live response. There's now a
  `#[ignore]`d live test (`jito::client::tests::test_get_tip_accounts_live`,
  same convention as `solstice-blockchain`'s existing live RPC test) that
  passed when run explicitly.
- `jito::JitoClient::send_bundle`/`get_bundle_status`/`confirm_bundle` —
  `sendBundle` and `getBundleStatuses` request/response handling, built to
  Jito's documented JSON-RPC shape and unit-tested against synthetic
  fixture JSON (request shape, success, RPC error, landed, failed, and
  not-yet-found-treated-as-pending cases). **Not exercised against a real
  submission** — that needs a real signed transaction and real SOL for the
  tip, which this agent does not hold and will not acquire on the user's
  behalf. Flagged rather than silently assumed correct.
- `jito::submit_with_fallback` (5.2/5.3) — tries the Jito bundle path first;
  on rejection, a `Failed` status, or a `confirm_bundle` timeout, falls back
  to submitting the primary transactions directly via a new
  `SolanaRpcClient::send_transaction`/`get_latest_blockhash` in
  `solstice-blockchain` (that crate previously had no send capability at
  all — only read-only `get_account`/`get_multiple_accounts`). The fallback
  path deliberately drops the tip transaction: a direct RPC send gets no
  MEV protection, so paying the Jito tip for it would burn SOL for nothing.
- "Bundle redundancy" (5.2) is submitting the same bundle to every
  configured `JitoConfig::endpoints` entry in turn — real multi-region
  redundancy, just sequential rather than concurrent (no new dependency
  needed for that; a reasonable scope cut given a handful of endpoints).

**Deliberately not built**: dynamic fee-market-aware tip optimization
(`TipStrategy` is caller-configured, not self-tuning), and settlement
recording to `solstice-storage` (not wired automatically — `SubmissionOutcome`
returns the bundle id/signatures a caller needs to record a fill via the
existing `StoragePool::save_trade` themselves). Both are explicit follow-up
work, not silently skipped.

**Verified end to end**: `cargo fmt --check`, `cargo clippy --workspace
--all-targets --all-features -D warnings`, and `cargo test --workspace` all
pass clean (26 new tests in `solstice-execution` covering bundle capping,
tip math, request/response parsing, and the fallback path's guard
conditions). Plus the one live call described above. Bundle
submission/confirmation against real mainnet — the only remaining
unverified piece — needs a funded wallet and real swap instructions the
user would have to supply; not something to fabricate or attempt here.

---

## [0.1.0-alpha] - 2026-07-21 (Phase 9.1/9.2)

### Phase 9.1 - Unit Tests ✅ (targeted, not exhaustive), 9.2 - Integration Tests ✅ (targeted, not exhaustive)

No coverage tool run — `cargo tarpaulin` doesn't support Windows well and
wasn't installed in this sandbox — so instead of chasing an 80% number
without a way to measure it, this pass did a manual audit (grep every
non-trivial source file for `#[test]`/`#[tokio::test]`) to find the
highest-risk *untested* code, and closed the worst gaps found rather than
padding coverage on code that already had it.

**Two real gaps, both closed**:

1. **`PaperTradingEngine` had zero tests.** `crates/solstice-simulation/src/engine.rs`
   (564 lines) is the actual live paper-trading logic — the same code this
   session watched fill a real $1,000 SOL order earlier today — and had
   never been unit-tested at all. `act_on_signal`, `evaluate_stop_losses`,
   and `portfolio_snapshot` are callable directly without touching the
   network (only `sample_market`/`tick`'s DEX-quoting does I/O), so 6 new
   tests exercise them directly: opening a position and debiting cash,
   the position-size-cap rejecting a second fill on top of an already-near-cap
   position, a no-price signal being a safe no-op, a losing position
   actually getting closed by the stop-loss check (with negative realized
   P&L), and the snapshot's total-value math.

2. **`solstice-api` had zero integration tests.** Every REST handler and
   the WebSocket endpoint were entirely unverified beyond manual `curl`
   sessions during Phase 7/8 development. New `crates/solstice-api/tests/integration_tests.rs`
   (6 tests) drives the *real* `ApiServer` router — added a small
   `ApiServer::router()` accessor for this — against a real, in-memory
   `PaperTradingEngine` (no live network: the test engine registers no
   Raydium/Orca pools, so `tick()` never reaches out to a DEX). REST
   endpoints are tested via `tower::ServiceExt::oneshot`; the WebSocket
   endpoint needed a real bound `TcpListener` and a real `tokio-tungstenite`
   client instead, since `oneshot` can't exercise a protocol upgrade — that
   test calls `engine.tick()` and asserts a real `TickCompleted` JSON frame
   arrives over the actual socket.

**Also added**: failure-path tests for the new
`SolanaRpcClient::send_transaction`/`get_latest_blockhash` (Phase 5) against
an unreachable endpoint (connection-refused on `127.0.0.1:1`, so they fail
in milliseconds rather than waiting out a timeout) confirming they return a
typed error instead of hanging or panicking, plus a live `#[ignore]`d
`get_latest_blockhash` test against real mainnet (same convention as the
existing `get_account_live` test) — run explicitly and confirmed passing.
`ApiError`/`error.rs` (previously completely untested, and in fact never
even constructed by any handler) got two tests of its own.

**Not attempted**: 9.1's 80%+ coverage *claim* (no tool to measure it
against, see above); 9.2's "recovery procedures" (needs a live RPC/DB to
actually fail and recover against, which isn't running in this sandbox);
9.3 chaos testing and 9.4 performance/load testing (both need live
infrastructure — Postgres, Redis, RPC nodes under load — this sandbox
doesn't have). Left unchecked in `ROADMAP.md` rather than claimed done.

**Verified end to end**: `cargo fmt --check`, `cargo clippy --workspace
--all-targets --all-features -D warnings`, and `cargo test --workspace` all
pass clean — 288 tests total across the workspace (16 new this pass: 6 in
`solstice-simulation`, 8 in `solstice-api` (2 unit + 6 integration), 2 in
`solstice-blockchain`), zero failures.

---

## [0.1.0-alpha] - 2026-07-21 (Phase 10.3 groundwork: sign/submit/confirm pipeline)

### Correction to the Phase 5 entry, and closing the gap it described

The Phase 5 changelog entry claimed "no swap-instruction building exists
anywhere in this workspace." **That was wrong.** `solstice-dex`'s
`JupiterClient::build_swap_instructions` (Phase 2.1) already called
Jupiter's real `/swap-instructions` API and returned genuine, executable
`Instruction`s — Phase 5 only looked at `Quote`/`RouteSegment` (pricing
metadata) and missed that the DEX client itself goes further. Recorded here
rather than silently edited into the old entry, since this changelog is a
history, not just a status board.

What Phase 5 got right stands: nothing assembled those instructions into an
actual `Transaction`, signed it, or submitted it anywhere. That gap is what
this pass closes, plus two real bugs found only by actually running the
existing Jupiter integration live for the first time.

### New: transaction assembly (`solstice_execution::swap`)

`build_swap_transaction(dex, swap, quote, recent_blockhash, signers) -> Transaction`
fetches instructions via `DexClient::build_swap_instructions`, assembles
them with `solstice_blockchain::TransactionBuilder`, signs, and checks the
*actual serialized size* against Solana's 1232-byte limit before returning
— rather than building a `VersionedTransaction` with address lookup
tables, which `DexClient::build_swap_instructions`'s return type
(`Vec<Instruction>`, no ALT metadata) doesn't have enough information to do
safely. A caller that hits the size error needs ALT support, which isn't
built. This isn't a hypothetical edge case: the live SOL/USDC test below
returned a route that itself needs one ALT.

### New: `SolanaRpcClient::confirm_transaction` (`solstice-blockchain`)

Polls `getSignatureStatuses` until a transaction confirms, fails on-chain,
or times out, populating the `TransactionConfirmation`/`TransactionStatus`
types that already existed in `solstice-blockchain::types` but that nothing
had ever produced from a real RPC call. Paired with the existing
`send_transaction`, this is the first time this codebase has had a
complete submit → confirm path.

### Two real bugs found by actually running Jupiter's integration live

Everything in `solstice-dex::jupiter` had unit tests against hand-written
fixture JSON, but had never been run against the real API until this pass.
Both bugs were invisible to the fixtures and only surfaced against a real
response:

1. **Wrong/dead API endpoint.** `api.jup.ag/v6` (the hardcoded default) is
   unreachable from this sandbox and is, independently, now Jupiter's paid
   tier — free access moved to `lite-api.jup.ag/swap/v1` (same
   request/response shape, confirmed via direct `curl` and then via the
   live test below). `JupiterClient::DEFAULT_API_BASE` now points there.
2. **The "raw quote" being forwarded to `/swap-instructions` was
   incomplete.** `JupiterQuoteResponse` used `#[serde(flatten)]` on a
   `raw: serde_json::Value` field, intending it to hold the complete
   original response so it could be sent back verbatim. Flatten doesn't
   work that way — it only captures whatever's *left over* after the named
   fields (`inAmount`, `outAmount`, `routePlan`, ...) consume their keys,
   so `raw` was silently missing exactly the fields `/swap-instructions`
   needs back. Every existing unit test passed anyway, because the
   hand-written fixtures never round-tripped through a real second
   request. The real API returned a clear `422` — `missing field
   'inAmount'` — the first time this was actually tried. Fixed by
   deserializing `fields` and `raw` **separately** from the same JSON
   (`JupiterQuoteFields` for the typed parts, a second untouched
   `serde_json::Value` for `raw`), so `raw` is genuinely the complete
   document. Also found and fixed in passing: the live API doesn't always
   include `routePlan[].swapInfo.feeAmount`, which the parser previously
   required unconditionally (`#[serde(default)]` added).

### Devnet dry run: written and unit-tested, blocked on a rate limit

Added `test_sign_submit_confirm_pipeline_on_devnet` (`#[ignore]`d,
`solstice-blockchain`): generates a throwaway `Keypair` (devnet-only, never
persisted — devnet SOL is free faucet-issued test currency with no
monetary value, not a real financial asset), requests an airdrop, signs and
submits a trivial 1-lamport self-transfer, and confirms it landed. This
would be the first time this codebase ever actually submits a transaction
to any network — everything before this was either paper-simulated or a
read-only RPC call.

**It hasn't completed successfully in this sandbox**: `https://api.devnet.solana.com`'s
airdrop faucet returns `429 "reached your airdrop limit today"` for this
environment's outbound IP (confirmed directly via `curl`, and via the
public web faucet at faucet.solana.com, which shares the same underlying
limit). This is an external rate limit, not a code defect — the test's
logic is sound and its failure-path sibling
(`test_confirm_transaction_times_out_when_unreachable`) does pass. Whoever
runs this from a non-rate-limited IP (i.e., not this shared sandbox) should
see it pass; that's the remaining step to actually close Phase 10.3's
"Testnet trading" checkbox.

### Verified end to end

`cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features
-D warnings`, and `cargo test --workspace` all pass clean — 306 tests
total across the workspace (passed + explicitly-`#[ignore]`d live tests),
zero failures. Two live calls confirmed working end to end: `JupiterClient`
fetching a real SOL/USDC quote and real swap instructions from
`lite-api.jup.ag` (not `#[ignore]`d test data — an actual passing live
test), and the devnet RPC endpoint itself being reachable (only the faucet
is rate-limited).

---

## [0.1.0-alpha] - 2026-07-21 (Phase 10.3: devnet dry run actually completed)

### The sign/submit/confirm pipeline, proven live

The previous entry left one thing open: the devnet faucet was
IP-rate-limited for this sandbox, so `test_sign_submit_confirm_pipeline_on_devnet`
had never actually run to completion. Worked around, and it now has:

1. Added `cargo run -p solstice-blockchain --example gen_devnet_keypair` —
   generates a throwaway devnet-only keypair (zero real value) and prints
   the address plus a ready-to-use `requestAirdrop` curl command.
2. Generated one: `CAxwjUEH7XgataKcfihGwzNWswqXsLtVgqpHjVLR9K3f`. The
   sandbox's own airdrop attempts still hit the same `429`, so it was
   funded manually via the faucet.solana.com web UI instead (10 devnet
   SOL, confirmed via `getBalance`).
3. Added `cargo run -p solstice-blockchain --example devnet_dry_run` —
   loads a keypair file and runs the real pipeline (fetch blockhash → sign
   a 1-lamport self-transfer → submit → poll for confirmation) against it.
   **This passed for real**: signature
   `1cj1mdfjJiy6iS4EhncEQX5qNUggikm6sGs3u2nUch98w6XcbzXR2gJZ3fvkBQAbCWwUQghxC7zhhdqZCWpqhTo`,
   confirmed at slot 477804109
   (https://explorer.solana.com/tx/1cj1mdfjJiy6iS4EhncEQX5qNUggikm6sGs3u2nUch98w6XcbzXR2gJZ3fvkBQAbCWwUQghxC7zhhdqZCWpqhTo?cluster=devnet).
   First real on-chain transaction this codebase has ever submitted.
4. Extended the actual `#[ignore]`d test itself
   (`solstice_blockchain::client::tests::test_sign_submit_confirm_pipeline_on_devnet`)
   to accept `DEVNET_TEST_KEYPAIR` pointing at a pre-funded keypair file,
   skipping the airdrop step when set. Ran it against the same funded
   wallet: **passed**. Without the env var it still falls back to
   requesting a fresh airdrop, for environments where the faucet isn't
   rate-limited.

**What this does and doesn't prove**: the generic transaction
sign/submit/confirm pipeline (`TransactionBuilder` +
`SolanaRpcClient::send_transaction`/`confirm_transaction`) now has a real,
passing, on-chain proof — not just unit tests against mocked responses.
It does **not** prove a real swap end-to-end: Jupiter's aggregator only
routes against mainnet liquidity, so `build_swap_transaction`
(`solstice-execution::swap`) has been live-verified for instruction
*fetching* (previous entry) but not for an actual devnet-or-mainnet
submission — that would need either real mainnet capital or a
devnet-specific DEX with its own (non-Jupiter) liquidity, neither of which
this pass attempted.

---

## [0.1.0-alpha] - 2026-07-21 (Read-only wallet: address, balance, deposit view)

### New: `WalletFile` (`solstice-blockchain`)

Local keypair file management, same JSON format the devnet examples
already used (and interoperable with `solana-keygen`): `generate()` (never
overwrites an existing file — the point of a wallet file is that it might
hold real funds), `exists()`, `pubkey()` (safe to log/display), and
`load_keypair()` (returns the private key — used only when code is about
to sign something, never logged). Also added
`SolanaRpcClient::get_balance`, unlike `get_account` returning `Ok(0)` for
a never-funded address rather than `AccountNotFound`, matching what a
wallet balance check actually wants.

### New: read-only `/api/v1/wallet` endpoint and dashboard page

`solstice-api` gained an optional `WalletState` (public key + RPC client
only — no signing capability reaches this server at all) wired in via a
new `WALLET_KEYPAIR_PATH` env var. `GET /api/v1/wallet` returns the
address and current SOL balance, or `404` if unconfigured; an unreachable
RPC now correctly reports `502` via a new `ApiError::Upstream` variant
(previously `ApiError` had exactly one variant, `NotFound`, and nothing
used it for anything but "not found" — this is the first real use of a
distinct upstream-failure status). Dashboard gained a matching Wallet page:
balance, a copyable deposit address, and explicit copy stating this server
can only read the balance and cannot send anything.

**Explicit design boundary, not just an implementation detail**: there is
no write/send endpoint anywhere in this API, and none is planned to be
added without the user directly triggering each send. `WalletState` is
public-key-only by construction — the API server process never loads a
private key, so there's nothing here that *could* sign a transaction even
if a route existed to ask it to.

**Verified live**: ran `serve` with `WALLET_KEYPAIR_PATH` pointed at the
devnet-funded wallet from the previous entry
(`CAxwjUEH7XgataKcfihGwzNWswqXsLtVgqpHjVLR9K3f`) and `HELIUS_RPC_URL`
pointed at devnet — `/api/v1/wallet` correctly returned its real balance
(9.99999 SOL, reflecting the 1-lamport test transfer from the prior dry
run), and the dashboard's Wallet page rendered it correctly with a working
copy-address button.

**Verified end to end**: `cargo fmt --check`, `cargo clippy --workspace
--all-targets --all-features -D warnings`, and `cargo test --workspace`
all pass clean (5 new backend tests: wallet file generate/load/overwrite-
refusal/error-handling, plus wallet-endpoint 404/502 cases); `tsc -b &&
vite build` passes clean on the dashboard.

---

## [0.1.0-alpha] - 2026-07-21 (Manual live swap execution, and a real trade)

### New: `solstice_execution::execute_swap` and the `trade` CLI

`execute_swap` is the reusable core that finally connects everything built
across Phase 5 and 10.3: build a swap transaction
(`build_swap_transaction`), submit it (`jito::submit_with_fallback` —
Jito bundle first, direct RPC fallback), and confirm it landed. It takes a
`&Keypair` directly rather than a wallet-file path, and performs the
action immediately with no confirmation gate of its own — that's
deliberate: this is meant to be the function a future automated engine
calls directly, the same way `PaperTradingEngine::act_on_signal` calls into
the paper fill pipeline. Human confirmation is a call-site concern, not
something baked into the library.

That call site is the new `trade` binary (`cargo run -p solstice-execution
--bin trade`): loads a wallet file, fetches a real quote, prints the full
route/amounts/price-impact, then requires the user to type the literal
word `SEND` (not `y`/`yes` — a typo should abort, not confirm) before ever
calling `execute_swap`. `--dry-run` builds and signs the real transaction
locally without submitting, for a zero-risk check that everything's wired
correctly before committing to a real submission. There is no `--yes`/
`--force` flag, and none is planned — that would defeat the point.

### First real trade this platform has ever executed

Run by the user, not this agent (the confirmation gate is not something
this agent will type through, at any dollar amount): 0.003 SOL → USDC on
mainnet, wallet `CAxwjUEH7XgataKcfihGwzNWswqXsLtVgqpHjVLR9K3f`. The Jito
bundle path didn't land and the built-in fallback to direct RPC submission
took over automatically — exactly the behavior `submit_with_fallback` was
built for in the Phase 5 entry, now observed for real rather than only in
tests. Confirmed independently via direct RPC calls (not just the CLI's
own output, which the user couldn't easily copy off a remote-desktop
session): balance dropped from 0.01 SOL to 0.004937 SOL, and a new SPL
token account appeared holding 0.234738 USDC. Transaction:
`47cnXVup8xVaUsNoC18n1bZYQdCNLW41SxzUUZNizqGTaS6wEPuZCcHF1akoQ2Fj6kN7F5WDxbihcG6WQjizD8m8`
(finalized).

### Dashboard: unambiguous paper vs. live mode

User feedback after seeing the app: it wasn't clear which mode was active,
and the header text ("Live paper trading — no real transactions") was
actually a static string that didn't reflect real wallet state at all —
misleading now that a real wallet with real funds exists. Fixed:
- A persistent header always shows both a **Paper — simulated funds**
  badge (blue) and a **Live wallet connected — real funds** badge (amber,
  only when a wallet is actually configured) or **No live wallet
  configured** (neutral) — regardless of which page you're on.
- The sidebar nav is now split into two visually distinct labeled groups,
  "Simulation (paper)" and "Live," with the Live section using the same
  amber accent as the wallet badge.
- The Overview and Wallet pages repeat their respective badge inline, so
  the mode is unmistakable even from a screenshot of just the content area.

**Verified end to end**: `cargo fmt --check`, `cargo clippy --workspace
--all-targets --all-features -D warnings`, `cargo test --workspace` (zero
new failures), and `tsc -b && vite build` all pass clean. The dashboard
changes were visually verified in a browser against a live server with the
real wallet configured — both badges render correctly, both nav sections
render correctly, and the Wallet page's balance/address match the real
on-chain state after the trade above.

### Housekeeping: `.gitignore` gained wallet-keypair patterns

While staging this commit, found `my-wallet.json` (a real mainnet keypair,
copied there per this session's own instructions so the user could run the
CLI from a stable path) sitting untracked at the repo root with no
`.gitignore` rule protecting it. Added `*wallet*.json`/`*keypair*.json`
patterns before it could ever be accidentally staged. Flagged here since
"almost committed a private key" is exactly the kind of near-miss worth a
permanent record, not a silent fix.

---

## [0.1.0-alpha] - 2026-07-21 (Automated live trading, wired up)

### New: `LiveTradingEngine` (`solstice_execution::live`)

The user explicitly asked for this, with the explicit expectation that it
would trade automatically: the same strategy → size → risk-check pipeline
as `PaperTradingEngine`, but backed by a real wallet, calling
`execute_swap` for real when armed.

**Defaults to disabled, and that default is load-bearing, not cosmetic.**
`LiveTradingEngine::is_enabled()` starts `false`; nothing flips it except
an explicit call to `enable()`. While disabled, every tick runs the exact
same signal-generation, sizing, and risk-check logic and emits
`LiveEvent::WouldTrade` instead of touching the network — so "what would
this do" is observable with zero funds risk before anyone arms it.
`disable()` is synchronous, instant, and unconditionally available: it
never awaits anything, so there's no scenario where turning trading off is
itself blocked on network I/O. Verified by test, not just asserted in a
comment: `test_disabled_engine_never_touches_capital_on_would_trade`
confirms the capital-deployed counter is untouched and a `WouldTrade`
event fires when disabled.

**Hard, adjustable capital ceiling.** `LiveTradingConfig::max_capital_usd`
(default $50, matching the user's stated starting point) bounds total
capital deployed *independent of the wallet's actual balance* — the
wallet may hold more, and that's not what limits risk here. Adjustable at
runtime via `set_max_capital_usd`, which the position-sizing math
(`plan_signal`) reads fresh on every signal, so a change takes effect
immediately, not on next restart.

**A real bug caught before it shipped**: the original `act_on_signal`
reconstructed a price snapshot from the *existing position's* stored
price, meaning a pair with no open position yet had no price to plan
against — the engine could never have opened its first position for any
pair. Fixed by passing the tick's already-fetched `MarketSnapshot` straight
through, the same way `PaperTradingEngine` does it. Caught while writing
this entry, not by a test — flagged here as a reminder that "mirrors an
existing, working engine's structure" doesn't guarantee mirroring its
correctness in every branch.

**A `!Send` future bug caught by the compiler, not by review**: the first
version of `build_swap_transaction` took `signers: &[&dyn Signer]`.
`dyn Signer` isn't `Sync`, so holding that reference across the function's
internal `.await` made the whole future `!Send` — which only became
visible once something tried to `tokio::spawn(live.run())` in `serve.rs`
and the compiler refused. Fixed by taking `&Keypair` (concretely
`Send + Sync`) instead, constructing the `&dyn Signer` slice only for the
synchronous `build_and_sign` call inside, never held across a suspension
point.

### New API surface: `/api/v1/live/*`

`GET /status`, `POST /enable`, `POST /disable`, `POST /config`
(`{"max_capital_usd": n}`, `400` on negative/non-finite), and a
`/live/ws` WebSocket streaming `LiveEvent`s — all `404` if no wallet is
configured. `enable`/`disable` are real control-plane writes (unlike
every other endpoint in this API, which are read-only), reflecting that
the user explicitly asked for this control surface; `disable` needed no
extra scrutiny (always safe), `enable` is expected to sit behind the
dashboard's own confirmation gate, not the API's.

### Dashboard: a Live Trading control page

New page (`/live`, in the "Live" nav section alongside Wallet): trading
status, deployed/available/max capital, an editable max-capital field, a
live activity feed (would-trade previews, fills, failures, position
closes), and the kill switch itself. The **disable** button is always a
single click. The **enable** button stays disabled until the user types
the literal phrase `ENABLE LIVE TRADING` into an adjacent field — the same
"a typo should abort, not confirm" philosophy as the `trade` CLI's `SEND`
confirmation, now as a UI gate. Verified in a real browser against a real
running server with the real wallet configured: typing the wrong phrase
left the button `disabled` (checked via direct DOM inspection, not just
visually), and live `PriceUpdate`/`TickCompleted` events streamed
correctly over the new WebSocket. `enable` was **not** clicked during this
verification — arming real trading is the user's action, not this agent's,
even to prove a UI works.

### Verified end to end

Live server run against the real wallet
(`CAxwjUEH7XgataKcfihGwzNWswqXsLtVgqpHjVLR9K3f`, ~$0.78 balance) with
`WALLET_KEYPAIR_PATH` set: startup logs confirmed "Live trading engine
configured (DISABLED by default, max $50.00 capital cap)", and
`GET /api/v1/live/status` returned the expected disabled state.
`POST /api/v1/live/config` with `{"max_capital_usd": 25}` updated it,
`POST /api/v1/live/disable` was called (safe, idempotent) and confirmed
still-disabled, and `-5.0` was correctly rejected with `400`.
`cargo fmt --check`, `cargo clippy --workspace --all-targets
--all-features -D warnings`, and `cargo test --workspace` all pass clean
(8 new tests in `solstice-execution::live`, covering the kill-switch
default, enable/disable events, capital-cap enforcement, and the
disabled-mode safety invariant). Dashboard `tsc -b && vite build` clean.

---

## [0.1.0-alpha] - 2026-07-21 (Fix: real-trade failures from the first live run)

The user armed live trading with real capital and it failed twice in a row
with `FAILED SMA` events, pasting the actual dashboard activity feed
(WebSocket inspection was declined -- see below) since a Chrome Remote
Desktop / Android session made copying terminal output impractical. Both
failures were confirmed via direct Helius RPC (`getBalance`,
`getSignaturesForAddress`) to have happened *before* broadcast --
simulation/local-size rejection, not a landed-but-reverted transaction --
so no fees were lost either time. Diagnosed and fixed three separate
issues:

### Fix 1: mislabeled error hid the real failure mode

`jito::fallback::submit_with_fallback`'s direct-RPC fallback wrapped every
failure as `JitoError::Http(...)`, so a plain RPC rejection displayed as
"HTTP request to Jito Block Engine failed" -- actively misleading, since
the failure had nothing to do with Jito. Added
`JitoError::DirectSubmissionFailed(String)` and used it at that call site
instead.

### Fix 2: address lookup tables -- the actual blocker

The real cause of both failures: `build_swap_transaction` could only
assemble a legacy `Transaction`, and Solana's legacy format caps out at
1232 bytes. The user's SOL/USDC route needed 2-3 address lookup tables
(Jupiter's `/swap-instructions` response said so explicitly --
`address_lookup_table_addresses`, previously parsed and then discarded
after only logging a `warn!`). A route that needs ALTs literally cannot
fit in a legacy transaction, so every attempt against that route was
guaranteed to fail regardless of price or slippage.

Built full `VersionedTransaction` support:
- `solstice_dex::DexClient::build_swap_instructions` now returns a new
  `SwapInstructions { instructions, address_lookup_tables }` instead of a
  bare `Vec<Instruction>`, so the ALT addresses Jupiter already returns
  are surfaced to the caller instead of thrown away. `JupiterClient`
  parses them into `Pubkey`s; Orca/Raydium (which already refuse to build
  instructions at all -- see their module docs) just carry the new return
  type.
- `solstice_execution::build_swap_transaction` now takes
  `rpc: &SolanaRpcClient`, fetches each referenced ALT account
  (`rpc.get_account` + `AddressLookupTable::deserialize`), and compiles a
  `v0::Message` via `v0::Message::try_compile` when any ALTs are present
  (falling back to a bare legacy-shaped message with none when a route
  needs none). Returns `VersionedTransaction` uniformly either way.
- That type change propagated through the whole submission path:
  `jito::Bundle` now holds `Vec<VersionedTransaction>`,
  `jito::submit_with_fallback`/`try_jito` take
  `&[VersionedTransaction]`/`Option<VersionedTransaction>`, and
  `SolanaRpcClient::send_transaction` takes `&VersionedTransaction`
  (`solana_client`'s underlying `send_transaction` is already generic over
  `SerializableTransaction`, so no changes needed there). Remaining
  legacy-`Transaction` producers (the Jito tip transaction, the
  `devnet_dry_run` example, the plain sign/submit/confirm pipeline test)
  convert via `VersionedTransaction::from(legacy_tx)`.
- Finding the right API took reading the vendored crate source directly
  rather than guessing: `AddressLookupTableAccount` isn't in
  `solana-address-lookup-table-interface` (where it would seem to belong)
  -- it's defined in `solana-message` itself and re-exported as
  `solana_sdk::message::AddressLookupTableAccount`, while the *state*
  type used to deserialize a fetched account (`AddressLookupTable`) comes
  from `solana_sdk::address_lookup_table::state`. Consistent with this
  session's standing rule for money-moving code: verify against the real
  crate source, don't assume a plausible-sounding path.

### Fix 3: stale quote + tight slippage on top of everything else

Even once a route fits on-chain, the observed revert
(`JUP6Lk...` program error `0x1788` inside a `Route` instruction -- exact
meaning not independently confirmed against Jupiter's IDL, flagged as
such at the time) is consistent with the quote going stale between being
read and being submitted, combined with a tight 50bps slippage tolerance.
`execute_planned_trade` used to: fetch a quote, load the wallet keypair,
fetch Jito tip accounts over the network, *then* call `execute_swap`
(which fetches its own fresh quote again inside
`JupiterClient::build_swap_instructions`, since Jupiter's
`/swap-instructions` needs the exact quote body) -- three sequential
round trips between reading a price and trading against it.
- Reordered to load the keypair and resolve the tip account *before*
  fetching the quote that actually gets traded against, shrinking that
  window.
- Added a 5-minute tip-account cache (`cached_tip_accounts`) instead of
  querying Jito on every single trade attempt -- tip accounts don't
  rotate meaningfully faster than that, so this removes a full network
  round trip from the hot path for free.
- Raised `LiveTradingConfig::default().slippage_bps` from 50 to 150
  (0.5% → 1.5%) -- still tight for the small trade sizes ($15-50) this
  engine targets, but enough room to absorb normal price movement across
  the unavoidable re-quote inside `build_swap_instructions`.

### Verified

`cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features
-D warnings`, and `cargo test --workspace` all pass clean (the `solstice-api`
crate's `serve` binary was excluded from the build/clippy passes purely
because the user's live server was running against it at the time and
holding the file lock -- its `--lib --tests` target was checked/tested
separately and is unaffected). No live trade was attempted by this agent
-- verification was via RPC read calls and the existing test suite;
placing a real order remains the user's action alone. The running server
needs a restart to pick these fixes up.

---

## [0.1.0-alpha] - 2026-07-21 (Live monitoring fallout: sell direction, confidence scoring, threshold)

With the ALT/slippage fixes live, the user re-armed live trading and it
opened a real position (0.0641 SOL @ ~$77.99, first genuine automated
fill) -- but every subsequent live check showed it just holding, never
selling, even as price moved. Two separate issues surfaced from watching
it run for real, both fixed here.

### Fix: live engine always bought, never sold, regardless of signal

`execute_planned_trade` unconditionally built every swap as
`quote_mint -> base_mint` (spend USDC, buy SOL) no matter what
`planned.is_buy` said -- so a sell signal would still try to spend USDC,
and with the wallet holding only $0.23 USDC against 0.205 SOL, every
attempt reverted on-chain with the same Jupiter `0x1788` error, confirmed
by directly querying the wallet's USDC token account over read-only RPC.

Fixed by branching on `planned.is_buy` throughout the buy/sell paths:
- `plan_signal` now treats a sell as reducing (fully closing) existing
  exposure rather than gating it on the capital/position cap meant for
  *opening* new exposure -- but requires a position to actually exist,
  since this engine can't short.
- `execute_planned_trade` builds `base_mint -> quote_mint` for a sell,
  sized to the position's actual held quantity (never more), instead of
  an independent USD figure that could exceed what's held.
- `record_fill`'s bookkeeping now mirrors `close_position`'s stop-loss
  exit path for a sell: removes the position, computes realized PnL from
  the quote's actual proceeds, and adjusts `capital_deployed_usd`.

Along the way, a second finding worth flagging explicitly: the
`SimpleMovingAverageStrategy` this live setup runs only ever emits `Buy`
signals (crossover-up) or nothing (crossover-down) -- it never emits a
`Sell`. So in this specific single-strategy configuration, a position can
currently only close via the stop-loss, never via a strategy signal, even
with the direction bug fixed. Not addressed here since the user didn't
ask for a sell-signal-emitting SMA variant -- just noted for when it
becomes relevant.

### Feature: real SMA confidence scoring + a user-set action threshold

The user noticed every `SignalGenerated` event showed exactly 65%
confidence, always, and asked why never 85%. Looking at
`SimpleMovingAverageStrategy`, the answer was that `confidence` was a
hardcoded constant (`0.65`) on every single signal, regardless of how
strong the crossover actually was -- not a real measure of anything, and
notably the exact same number `PositionSizer`'s Kelly-fraction math uses
to decide bet size.

- Replaced the constant with `crossover_confidence(short_sma, long_sma)`:
  scales with the relative gap between the two averages, clamped to
  `[0.5, 0.95]` -- a razor-thin crossover sits near the floor, a wide,
  decisive one approaches the ceiling. Simple heuristic, not a calibrated
  probability, but at least responsive to the actual signal strength now.
- Added `LiveTradingConfig::min_confidence` (default `0.65`, matching the
  strategy's old fixed value so existing behavior doesn't silently change
  for anyone who hasn't touched it): `plan_signal` now rejects any signal
  below this threshold before sizing or risk-checking it, emitting
  `SignalSkipped`. Adjustable at runtime via the new
  `LiveTradingEngine::set_min_confidence`, mirroring
  `set_max_capital_usd`'s existing pattern exactly.
- Wired end to end: `POST /api/v1/live/config` now accepts an optional
  `min_confidence` alongside the existing optional `max_capital_usd` (the
  request DTO, `SetMaxCapitalRequest`, was renamed `LiveConfigRequest` and
  both fields made independently optional -- omitting one leaves it
  unchanged). `LiveStatusSnapshot` and the dashboard's Live Trading page
  both surface and let you edit it, next to the existing max-capital
  control.

### Verified

`cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features
-D warnings`, and `cargo test --workspace` all pass clean (8 new tests
across `solstice-strategy` and `solstice-execution`: confidence-floor,
confidence-scaling, confidence-ceiling-clamp, wider-crossover-higher-
confidence, sell-without-a-position rejected, sell-with-a-position
accepted, threshold-rejects-below-minimum, threshold-accepts-at-minimum).
`tsc -b` clean on the dashboard. The running server was stopped (safe --
capital fully deployed but no pending signals) and restarted on the new
binary each time, same wallet re-confirmed via `/api/v1/wallet`.

---

## [0.1.0-alpha] - 2026-07-21 (Wallet page: USDC + devnet balances, manual SOL⇄USDC convert)

The user asked for the Wallet page to show SOL and USDC balance for live
(mainnet), balance for devnet, and a way to convert between SOL and USDC.
Asked first whether "auto convert" meant manual (click to convert, once,
with confirmation) or fully automatic rebalancing armed once like the
live trading engine -- the user chose manual, on-demand.

### New: `SolanaRpcClient::get_token_balance`

No SPL-token balance reader existed yet. Added
`get_token_balance(owner, mint)`, mirroring `get_balance`'s failover/retry
pattern and its "no token account yet is a normal state" philosophy (`Ok(0)`,
not an error). Uses `getTokenAccountsByOwner` with `jsonParsed` encoding
(the RPC method's default) and sums every matching account's parsed
`tokenAmount.amount` -- permissive by design, since this is a best-effort
display quantity, not something that gates a real transaction.
`solana-account-decoder-client-types` added as a new direct workspace
dependency for `UiAccountData`, rather than relying on a fragile
re-export chain through `solana-client`.

### Changed: `GET /api/v1/wallet` now includes USDC balance

`WalletResponse` gained `usdc_balance_raw`/`usdc_balance`. The doc
comment's old claim that this server has "no field or endpoint that can
move funds" is no longer true as of this entry (see below), so it was
corrected rather than left stale.

### New: `GET /api/v1/wallet/devnet`

Same wallet address, same keypair, but devnet and mainnet are separate
ledgers -- this reports the devnet SOL balance side by side with mainnet
(useful here specifically because this wallet has leftover devnet SOL
from earlier faucet-funded testing in Phase 10.3).

### New: `POST /api/v1/wallet/convert` -- a real, user-initiated swap

**Executes a real, irreversible on-chain SOL<->USDC swap** using the
configured wallet's own funds, in whichever direction the caller
requests. This is the first API endpoint in this server that can
actually move funds, and it was built with the same discipline as
everywhere else real money is involved in this codebase:
- New `ConvertState` (`solstice-api::state`) holds a `WalletFile` (not a
  loaded key), an RPC client, a `JitoClient`, and a `JupiterClient`.
  `WalletFile` reads the private key from disk transiently, only at the
  moment a request is actually being signed, and only in response to a
  request this server received -- never on a timer, never proactively.
  Wrapped in `Arc<ConvertState>` (rather than making `JitoClient`/
  `JupiterClient` themselves `Clone`) purely so `AppState` stays cheaply
  cloneable for Axum's `State` extractor.
- The handler itself is a thin wrapper around the same `execute_swap`
  the live trading engine and the `trade` CLI already use -- fetch a
  quote, load the key, build/sign/submit via Jito with direct-RPC
  fallback, report the outcome. No new execution path, just a new,
  human-initiated caller of the existing one.
- **No server-side confirmation gate on the endpoint itself** -- by
  design, matching `POST /live/enable`'s precedent exactly: the
  confirmation lives in the dashboard (typing the literal phrase
  `CONVERT` before the button un-disables), not the API. Verified this
  gate actually holds in a real browser: with the amount field empty and
  no confirmation text entered, direct DOM inspection confirmed the
  Convert button's `disabled` property is `true`. The button was **not**
  clicked to actually convert anything during this verification --
  proving the gate holds isn't the same as testing a real swap, and a
  real swap remains the user's action alone, not this agent's, even to
  prove a feature works.

### Dashboard: Wallet page

Four balance tiles (mainnet SOL, mainnet USDC, devnet SOL, lamports --
the devnet tile only renders once that endpoint responds), the existing
deposit-address/copy panel (with an added note that devnet and mainnet
balances are separate ledgers on the same address), and a new Convert
panel: direction selector, amount input, and the typed-`CONVERT`
confirmation gate before the button enables. A result panel shows the
method used and transaction signature(s) on success.

### Verified

`cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features
-D warnings`, and `cargo test --workspace` all pass clean (4 new tests:
two for `get_token_balance`'s sum/empty parsing, two already covered by
existing failover tests extended to the new method). `tsc -b` clean.
Live-verified against the real running server: `/api/v1/wallet` returned
real mainnet SOL (0.0728) and USDC (25.02) balances, `/api/v1/wallet/devnet`
returned the same address's real devnet balance (~10 SOL, leftover from
Phase 10.3 faucet testing) -- confirming the token-balance parsing and
the devnet/mainnet split both work against live data, not just fixtures.
No conversion was executed by this agent.

---

## [0.1.0-alpha] - 2026-07-21 (Phase 2.2 completed: Orca and Raydium swap execution)

Orca's and Raydium's `build_swap_instructions` had returned a hard error
since Phase 2.2, on the stated grounds that this integration couldn't
confirm the required account ordering/layout without guessing at
real, funds-moving instructions. The user asked what it would actually
take to finish them. Re-investigating found both blockers were solvable
with what was already available -- nobody had gone back to check.

### Orca: wired up the crate's own generated instruction builder

`orca_whirlpools_client` (already a dependency, used for quoting) ships a
`SwapV2` instruction builder generated directly from Orca's IDL
(`generated::instructions`) -- the exact account ordering the old code
comment said couldn't be confirmed. Implementation:
- `spl-associated-token-account-interface` (new dependency) derives the
  payer's associated token accounts and builds `CreateIdempotent`
  instructions for both mints, ahead of the swap -- mirroring the same
  step already observed in real Jupiter-routed transactions this session.
- A fresh quote (tick arrays + `swap_quote_by_input_token`) is computed
  inside `build_swap_instructions` itself rather than reusing the
  caller's `Quote`, keeping `other_amount_threshold` as close to
  submission time as possible -- same rationale as Jupiter's client.
- `orca_whirlpools_client` and `spl-associated-token-account-interface`
  both pin `solana-pubkey`/`solana-instruction` on the `3.x` line, one
  major version ahead of this workspace's `solana-sdk` 2.x, so every
  value crossing that boundary needs an explicit conversion (`to_sdk_pubkey`/
  `to_sdk_instruction`) -- the same pattern already established for
  quoting, extended to cover instructions too.
- `tick_array_addresses` extracted as its own helper (previously inlined
  in the quoting path) since the instruction needs all three tick-array
  PDAs unconditionally, while quoting only needs whichever ones happen to
  be initialized on-chain.

### Raydium: hand-parsed the OpenBook market layout after confirming the crate route is dead

`raydium_amm` (already a dependency) also has its own generated
`SwapBaseIn` builder, and `AmmInfo` (already fetched for quoting) turned
out to already store `market` and `serum_dex` directly -- fields
initially assumed missing. What remained was the OpenBook/Serum market's
own bids/asks/event-queue/vault accounts, which live in a separate
account this integration wasn't fetching.

The obvious fix -- depend on `openbook_dex`/`serum_dex` for that account's
struct -- was tried and confirmed dead: added directly, it fails to
compile even in isolation (`cannot find type Pubkey`, `cannot find
associated function process_new_order_v3`, from inside the crate's own
source), on top of pinning `solana-program` 1.10. Reverted.

Instead, parsed the market account's raw bytes directly -- the classic
Serum v3 / OpenBook v1 layout, unchanged for years. Not transcribed from
memory and trusted blind: fetched the real OpenBook market for Raydium's
live SOL/USDC pool (`8BnEgHoWFysVcuFFX7QztDmzuH8r5ZFvyP3sYwn1XTh6`) over
RPC and confirmed the `own_address` field decodes back to that account's
own address at every layer of the offset table -- a self-referential
field that only matches if every prior offset is exactly right. Baked
that same real account into a committed unit test fixture. The
`vault_signer` PDA (derived from the market address + the parsed nonce)
and the `amm authority` PDA (the well-known `b"amm authority"` seed) were
similarly checked for a valid, non-erroring derivation against that same
live data before trusting them.
`raydium_amm`'s pubkey/instruction types happen to already resolve to the
same `2.x` line as `solana-sdk` (unlike Orca's `3.x`), so none of Orca's
conversion dance was needed here.

### Verified

`cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features
-D warnings`, and `cargo test --workspace` all pass clean (11 new tests:
market-layout parsing against the live fixture, short-data rejection,
vault-signer PDA derivation, ATA derivation/instruction shape). Beyond
unit tests, two new `#[ignore]`d live integration tests build real swap
instructions against Orca's and Raydium's actual live SOL/USDC
pools/whirlpool over a real mainnet RPC endpoint and check the resulting
instruction list's shape -- both pass. Every pool/program address used
(the Orca whirlpool, the Raydium pool, its OpenBook market, the Raydium
program id) was independently confirmed against Orca's and Raydium's own
public APIs before being trusted, not assumed from memory. No transaction
was signed or submitted by this agent -- this closes the "can't build
instructions" gap; actually trading through either DEX for real remains
gated the same way every other swap in this codebase is (the `trade` CLI's
`SEND` confirmation, or the live engine's armed kill switch).

---

## [0.1.0-alpha] - 2026-07-21 (Orca and Raydium wired into live trading)

Following straight on from the previous entry, the user asked for
Orca/Raydium execution to actually be wired into `LiveTradingEngine`,
not just available as capability.

### `LiveTradingEngine` now quotes/executes through a `DexAggregator`

Previously held a single `JupiterClient` directly. Now builds a
`DexAggregator` in its constructor, registering Jupiter (always -- it
needs no pre-known pool address) plus Orca and Raydium clients with
whatever pools each `LiveTradedPair` supplies:
- `LiveTradedPair` gained `raydium_pool: Option<Pubkey>` and
  `orca_pool: Option<Pubkey>` fields, mirroring `MonitoredPair` in the
  paper-trading engine exactly (same field names, same "`None` just means
  this DEX never wins the comparison for this pair" semantics).
- Both `sample_market` (price sampling) and `execute_planned_trade`/
  `close_position` (actual execution) now call `get_best_route` instead
  of a single client's `get_quote` -- every registered DEX is queried
  concurrently and the highest-output quote wins, per pair, per tick.
- At execution time, the winning quote's `route[0].dex` name (e.g.
  `"Jupiter"`, `"Orca"`, `"Raydium"` -- each client already tags its own
  quotes this way) is used to fetch that specific client back out of the
  aggregator via `get_client`, since `execute_swap` needs the concrete
  `DexClient` that produced the winning quote to build instructions
  against, not the aggregator itself.
- `serve.rs` populates the SOL/USDC pair's new fields with the same
  Raydium pool / Orca whirlpool addresses already used (and verified) in
  `solstice_simulation::demo`'s paper-trading setup.

### Rationale

Jupiter already routes through Orca/Raydium liquidity internally, so this
isn't about reaching pools Jupiter couldn't otherwise reach -- it's: (a)
occasionally a direct pool beats Jupiter's own aggregation overhead, and
(b) a live fallback if Jupiter's API has an outage, since Orca/Raydium
would still be queried independently.

### Verified

`cargo fmt --all`, `cargo clippy --workspace --lib --bins --tests
--all-features -D warnings`, and `cargo test --workspace` all pass clean
(1 new test confirming the constructor's pool-registration path doesn't
error with real pool addresses set; the aggregator's own route-selection
logic was already covered by `solstice-dex`'s existing test suite). The
running server was stopped (safe -- no capital deployed, no positions)
and rebuilt/restarted on the new binary.

---

## [0.1.0-alpha] - 2026-07-21 (Live spread arbitrage: SpreadArbitrageStrategy wired in)

The user noticed the Overview page showing paper-only `SpreadArb` fills
and asked why live trading "doesn't appear to be trading arb." Root
cause: the previous entry wired Orca/Raydium into *execution routing*
(best price for whatever SMA decides to trade) but never registered
`SpreadArbitrageStrategy` itself in the live engine -- it was SMA-only.
Separately, clarified for the user that the Overview page is *always*
the paper engine ($10k simulated capital, runs regardless of live
state) and the Wallet page's devnet balance is just informational
display, not an indication live trading touches devnet.

### `sample_market` now samples each DEX individually, not just the best route

`SpreadArbitrageStrategy` needs *more than one* price observation per
pair per tick to ever detect a spread (`MarketSnapshot::prices[pair]`
must have 2+ entries) -- but `sample_market` had been collapsed to a
single aggregator best-route price since the previous entry, meaning the
strategy could never have fired even once it was registered. Rewrote
`sample_market` to query Jupiter, Raydium, and Orca each individually
(skipping Raydium/Orca for a pair with no known pool) and insert every
successful observation, mirroring `PaperTradingEngine::sample_market`'s
exact pattern. Position `current_price` tracking now uses the
last-sampled observation rather than a single aggregator call.
`LiveEvent::PriceUpdate.pair_label` now includes which DEX each reading
came from (e.g. `"SOL/USDC (Orca)"`), since there are multiple readings
per tick now instead of one.

`SpreadArbitrageStrategy::new(10)` (0.1% Raydium/Orca spread, same
threshold as the paper engine's) registered alongside SMA in
`serve.rs`'s live `StrategyManager`.

### What this does and doesn't do

Worth being explicit about: `SpreadArbitrageStrategy` only ever emits a
`Buy` signal when the spread exceeds the threshold -- it does not itself
specify a venue or attempt a true simultaneous two-leg arbitrage (buy
cheap, sell expensive, in the same breath). It's a directional signal
("the spread widened, take a position") sized and risk-checked exactly
like an SMA signal, then executed through whichever DEX the aggregator's
`get_best_route` finds cheapest at that moment -- which in practice tends
to be favorable, but this is not risk-free arbitrage in the strict sense.

### Verified

`cargo fmt --all`, `cargo clippy --workspace --lib --bins --tests
--all-features -D warnings`, and `cargo test --workspace` all pass clean
(existing `SpreadArbitrageStrategy`/`sample_market`-adjacent tests
already covered the strategy logic itself; this change is wiring, not
new decision logic). Server startup logs confirmed both strategies
register (`Registered strategy: SMA v1.0.0`, `Registered strategy:
SpreadArb v1.0.0`) and live per-DEX price sampling flows
(`[SOL/USDC] Raydium: $77.52`, `[SOL/USDC] Orca: $77.90`-style log
lines now also emitted from the live engine, not just the paper one).

---

## [0.1.0-alpha] - 2026-07-21 (Fix: live SpreadArb failures within minutes of going live)

The user re-armed live trading with SpreadArb enabled and pasted the
actual dashboard activity feed: one real fill (`$4.00 of SOL/USDC @
$77.8445 (Direct)`), then three straight `unknown DEX` failures, then a
real on-chain revert with `Error: InvalidInstructionData` from Raydium's
program. Two separate, real bugs, both introduced by the last two
entries.

### Bug 1: winning-DEX resolution broke whenever Jupiter's own route won

`execute_planned_trade`/`close_position` resolved which registered
client to call `build_swap_instructions` on via `quote.route.first().dex`,
assuming it would read back one of the three registered names
(`"Jupiter"`/`"Orca"`/`"Raydium"`). True for Orca/Raydium's own
single-hop quotes, but Jupiter's route segments are labeled with the
*underlying venue it actually routed through* (`"AlphaQ"`, `"SolFi V2"`,
`"BisonFi"`, ...) since Jupiter is itself an aggregator -- there's no
way to recover "Jupiter" (the name it's registered under in *our*
aggregator) from that string. Every tick where Jupiter's own route won
the best-price comparison failed with `unknown DEX: <venue>`.

Fixed by adding `DexAggregator::get_best_route_with_source`, which
returns the winning *registered* client's name alongside the `Quote`
(tracked internally during the existing concurrent-query/compare loop,
rather than reconstructed after the fact from the quote itself). Always
bypasses the route cache -- a caller asking for the winning client is
about to build a real instruction against it, and a several-second-old
cached quote isn't the freshness that decision needs.

### Bug 2: Raydium's own instruction builder produces invalid on-chain data

`raydium_amm`'s codegen'd `SwapBaseIn::instruction()` prepends an 8-byte
Anchor-style discriminator to the args -- but the deployed Raydium AMM v4
program is not an Anchor program (predates it), and that encoding
reverted for real on the very first live attempt that picked Raydium,
confirmed directly in the pasted logs: `Program 675kPX9...: Error:
InvalidInstructionData`, failing at ~5,136 compute units -- consistent
with failing instruction-data deserialization before any account is even
touched. Fixed by keeping this builder's (correct) account list but
replacing the data with the actual native-program encoding: a single
leading tag byte (`9`, `SwapBaseIn`'s variant index in Raydium's real,
plain-Borsh `AmmInstruction` enum) followed by the two `u64` args,
little-endian. Not independently re-verified against a raw on-chain
instruction this session the way the OpenBook market layout was --
attempted several routes to pull one (direct pool signature history,
Jupiter's swap-instructions API, block-explorer APIs) and all either
routed through wrapper contracts or were blocked; flagged clearly in
both the code comment and here. The failure mode if this index is
somehow still wrong is identical to before fixing it: a clean
pre-broadcast simulation rejection, not a wrong or lost trade.

### Verified

`cargo fmt --all`, `cargo clippy --workspace --lib --bins --tests
--all-features -D warnings`, and `cargo test --workspace` all pass clean.
New tests: `test_best_route_with_source_returns_registered_name_not_route_label`
(a `MockDex` whose route label deliberately differs from its registered
name, mirroring Jupiter's real behavior, confirming the fix resolves the
*registered* name); the existing live Raydium integration test now also
asserts the swap instruction's data is exactly 17 bytes, starts with tag
`9`, and correctly encodes `amount_in`. Server rebuilt and restarted
(safe -- no capital deployed, no positions at the time).

---

## [0.1.0-alpha] - 2026-07-21 (Live take-profit auto-exit)

### Problem

With Orca/Raydium execution and `SpreadArbitrageStrategy` wired into live
trading, the user asked how a fully-deployed live position would ever
exit. The answer: it wouldn't, on a gain. Neither `SimpleMovingAverageStrategy`
nor `SpreadArbitrageStrategy` ever emits a `Sell` signal (both are
directional buy-only bets), and the only exit path in `LiveTradingEngine`
was `StopLossManager`, which only fires on a loss. Once capital was fully
deployed, a profitable position would simply sit, unrealized, indefinitely.

### Fix

Added `TakeProfitManager` (`crates/solstice-execution/src/risk/take_profit.rs`),
structurally mirroring `StopLossManager` exactly -- same long-only filter,
same percentage-move math, opposite sign. `LiveTradingConfig` gained a
`take_profit_percent` field (default `0.05`, i.e. 5%). Rather than caching
a fixed `TakeProfitManager` instance at construction (as `stop_loss` is,
which means changing `stop_loss_percent` post-construction currently has
no effect -- a pre-existing quirk, left alone), `evaluate_stop_losses`
builds a fresh `TakeProfitManager` from `config.take_profit_percent` on
every call, so `set_take_profit_percent` takes effect on the very next
tick, matching how `min_confidence` already behaves.

`set_take_profit_percent` mirrors `set_min_confidence`/`set_max_capital_usd`:
updates the config `Mutex` and emits a new `LiveEvent::TakeProfitPercentChanged`.
Wired end to end: `LiveStatusSnapshot.take_profit_percent`, the
`POST /api/v1/live/config` handler (validates `take_profit_percent > 0`),
the dashboard's `LiveStatusResponse`/`LiveEvent` types, `client.ts`
(`liveSetTakeProfitPercent`), and a new stat tile + input control on
`LiveTradingPage.tsx` mirroring the existing min-confidence card.

### Verified

`cargo fmt --all`, `cargo clippy --workspace --lib --bins --tests
--all-features -D warnings`, and `cargo test --workspace` (via
`--exclude solstice-api` plus `-p solstice-api --lib` separately, per the
established workaround for the running `serve.exe` binary lock) all pass
clean. `npx tsc --noEmit` in `dashboard/` passes clean. New tests:
`TakeProfitManager`'s 4 unit tests (mirroring `StopLossManager`'s),
`test_set_take_profit_percent_updates_status`, and
`test_evaluate_stop_losses_flags_position_beyond_take_profit` (inserts a
position with a +10% gain against a 5% target, calls `evaluate_stop_losses`
directly on a disabled engine, confirms a take-profit close attempt is
emitted via `LiveEvent::SignalSkipped` -- disabled so the assertion needs
no network).

At the time this landed, the live server had a real, fully-deployed $15
SOL/USDC position open. Per the established safety practice, restarting
to pick up this change was held pending explicit user confirmation rather
than done automatically, since restarting resets in-memory position
tracking (the position itself stays on-chain, just untracked and
unprotected by stop-loss/take-profit until the engine is told about it
again).

---

## [0.1.0-alpha] - 2026-07-20

### Implementation Started

**Phase 1.1 - Workspace Setup & Core Types** ✅ COMPLETE

Core Infrastructure Implementation:
- Rust workspace with multi-crate architecture
- `solstice-core` crate with base types:
  - `Price` - Asset pricing with confidence scoring
  - `Position` - Trading position tracking with P&L calculation
  - `Signal` - Strategy signal types with confidence bounds
  - `OrderBook` - Market depth data with spread/mid-price calculation
  - `Trade` - Trade execution records with fee tracking
  - `Portfolio` - Portfolio state with concentration analysis
  - `MarketEvent` - Market data event enums
  - `TokenPair` - Token pair representation
  - Error types with `Result<T>` aliases
- Logging infrastructure with `tracing` and structured JSON output
- GitHub Actions CI/CD pipeline:
  - Automated testing on push/PR
  - Cargo fmt validation
  - Clippy linting with strict warnings
  - Documentation validation
- Comprehensive unit and integration tests
- Production-grade code quality standards

**Deliverables**:
- 11 core types fully implemented with validation logic
- ~500 lines of core type definitions
- ~200 lines of error handling
- ~100 lines of logging infrastructure
- 10+ unit tests and 11 integration tests
- GitHub Actions workflow for CI/CD

**Ready for**: Phase 1.2 (Market Data Ingestion)

---

## [1.0.0-draft] - 2026-07-20

### Added (Specification - Foundation)

**Documentation Framework**
- TABLE_OF_CONTENTS.md - Complete specification index
- ARCHITECTURE.md - System architecture and design overview
- WORKSPACE.md - Rust workspace and crate organization
- DESIGN_RATIONALE.md - Key architectural decisions (15 ADRs)
- ROADMAP.md - 11-phase development roadmap (18 months)
- CHANGELOG.md - Version history (this file)

**Core Architecture Documents** (In Development)
- CONFIGURATION.md - Configuration system and parameter management
- MARKET_DATA.md - Market data ingestion architecture
- YELLOWSTONE.md - Yellowstone gRPC integration
- SOLANA_RPC.md - Solana RPC abstraction layer
- DEX_INTEGRATIONS.md - Jupiter, Raydium, Orca, Meteora, Phoenix, OpenBook

**Trading Engine Documents** (Queued)
- STRATEGY_FRAMEWORK.md - Plugin-based strategy framework
- STAT_ARBS.md - Statistical arbitrage engine
- FAIR_VALUE.md - Fair value computation
- PORTFOLIO_MANAGEMENT.md - Portfolio management and rebalancing
- RISK_MANAGEMENT.md - Risk management framework
- POSITION_SIZING.md - Position sizing algorithms

**Execution & Optimization** (Queued)
- EXECUTION.md - Execution planner and transaction builder
- SIMULATION.md - Simulation engine
- FEE_OPTIMIZATION.md - Fee optimization strategies
- JITO_INTEGRATION.md - Jito Block Engine integration
- BUNDLE_MANAGEMENT.md - Bundle management

**Data & Storage** (Queued)
- DATABASE.md - PostgreSQL + TimescaleDB schema
- REDIS_ARCHITECTURE.md - Redis caching architecture
- HISTORICAL_DATA.md - Historical data retention

**Analytics & Backtesting** (Queued)
- BACKTESTING.md - Backtesting engine
- PAPER_TRADING.md - Paper trading mode
- PERFORMANCE_ANALYTICS.md - Performance metrics

**APIs & UI** (Queued)
- REST_API.md - REST API specification
- WEBSOCKET_API.md - WebSocket API
- DASHBOARD.md - React dashboard architecture
- AUTHENTICATION.md - Authentication and authorization

**Operations** (Queued)
- LOGGING.md - Logging strategy
- MONITORING.md - Monitoring framework
- PROMETHEUS_METRICS.md - Prometheus metrics
- GRAFANA_DASHBOARDS.md - Grafana dashboards
- DEPLOYMENT.md - Docker deployment
- SECURITY.md - Security architecture
- DISASTER_RECOVERY.md - Disaster recovery procedures
- OPERATIONAL_RUNBOOKS.md - Operations procedures
- CI_CD.md - CI/CD pipeline

**Development** (Queued)
- TESTING_STRATEGY.md - Testing framework
- CODING_STANDARDS.md - Rust coding standards
- ADR_TEMPLATE.md - ADR template for new decisions
- CONTRIBUTION_GUIDELINES.md - Contribution process
- ACCEPTANCE_CRITERIA.md - Feature acceptance criteria

### Specification Content

**ARCHITECTURE.md**
- System overview and high-level design
- Architectural layers (data ingestion, strategy, execution, blockchain, storage, APIs)
- Core data flows (market event processing, trading execution)
- Workspace organization
- 8 key architectural decisions with rationale
- Component responsibilities matrix
- Failure modes and resilience strategies
- Performance targets and characteristics
- Future extension points

**WORKSPACE.md**
- Rust workspace structure (11 crates)
- Detailed crate responsibilities:
  - solstice-core: Shared types and traits
  - solstice-market-data: Market data ingestion
  - solstice-blockchain: Blockchain integration
  - solstice-dex: DEX protocol implementations
  - solstice-strategy: Strategy framework
  - solstice-execution: Execution and risk
  - solstice-storage: Data persistence
  - solstice-api: REST and WebSocket APIs
  - solstice-simulation: Backtesting and paper trading
  - solstice-cli: Command-line interface
- Inter-crate dependency graph
- Module organization guidelines
- Feature flags framework
- Testing strategy per crate

**DESIGN_RATIONALE.md**
- 16 Architecture Decision Records (ADRs):
  - ADR-001: Event-driven architecture
  - ADR-002: Async/await with Tokio
  - ADR-003: Rust language selection
  - ADR-004: Monorepo workspace
  - ADR-005: Trait-based abstractions
  - ADR-006: Plugin-based strategy framework
  - ADR-007: Jito bundles for execution
  - ADR-008: Yellowstone as primary feed
  - ADR-009: PostgreSQL + TimescaleDB
  - ADR-010: Redis for caching
  - ADR-011: Fail-safe risk management
  - ADR-012: Structured logging
  - ADR-013: Prometheus + Grafana
  - ADR-014: Specification-first development
  - ADR-015: Three-mode operation (backtest/paper/live)
  - ADR-016: Axum web framework
- Decision matrix with trade-offs
- Future decision points
- Related documents and cross-references

**ROADMAP.md**
- 11 development phases spanning 18 months:
  - Phase 1: Core infrastructure (workspace, market data, blockchain, storage)
  - Phase 2: DEX integration (Jupiter, Raydium, Orca, etc.)
  - Phase 3: Strategy framework (fair value, stat arbs, portfolio)
  - Phase 4: Execution and risk management
  - Phase 5: Jito MEV protection
  - Phase 6: Simulation and backtesting
  - Phase 7: APIs and observability
  - Phase 8: React dashboard
  - Phase 9: Testing and hardening
  - Phase 10: Production deployment
  - Phase 11: Optimization and scaling
- Per-phase milestones and deliverables
- Dependency and gating strategy
- Risk mitigation approach
- Success metrics

### Initial Git Commit

- Initial commit: "Solstice quantitative trading platform"
- GitHub repository: https://github.com/chuzby-dev/solstice

---

## Plan for Subsequent Updates

### Next: Core Configuration & Market Data (Estimated 1-2 days)

- [x] TABLE_OF_CONTENTS.md
- [x] ARCHITECTURE.md
- [x] WORKSPACE.md
- [x] DESIGN_RATIONALE.md
- [x] ROADMAP.md
- [x] CHANGELOG.md
- [ ] CONFIGURATION.md
- [ ] MARKET_DATA.md
- [ ] YELLOWSTONE.md
- [ ] SOLANA_RPC.md
- [ ] DEX_INTEGRATIONS.md

### Subsequent: Strategy & Execution (Estimated 2-3 days)

- [ ] STRATEGY_FRAMEWORK.md
- [ ] STAT_ARBS.md
- [ ] FAIR_VALUE.md
- [ ] PORTFOLIO_MANAGEMENT.md
- [ ] RISK_MANAGEMENT.md
- [ ] POSITION_SIZING.md
- [ ] EXECUTION.md
- [ ] SIMULATION.md

### Final: Operations & Testing (Estimated 2-3 days)

- [ ] DATABASE.md
- [ ] REDIS_ARCHITECTURE.md
- [ ] REST_API.md
- [ ] WEBSOCKET_API.md
- [ ] LOGGING.md
- [ ] MONITORING.md
- [ ] TESTING_STRATEGY.md
- [ ] DEPLOYMENT.md
- [ ] Plus remaining 15+ documents

---

## Specification Versioning

- **1.0.0-draft**: Initial specification draft (foundation documents)
- **1.0.0-beta**: Complete specification with all documents
- **1.0.0**: Stable specification post-implementation validation
- **1.1.0**: First revision after Phase 1 implementation learnings
- **2.0.0**: Major redesign if needed post-Phase 5

---

## Known Gaps (Intentionally Deferred)

These areas are intentionally deferred until later specification phases:

1. **Machine Learning Strategies**: Deferred to Phase 11+
2. **Cross-Chain Support**: Deferred to Phase 11+
3. **Multi-Account Support**: Deferred to Phase 11+
4. **Strategy Marketplace**: Deferred to Phase 11+
5. **Advanced Analytics**: Deferred until Phase 10 completion

---

## Maintenance & Updates

This specification is a living document:

- **Weekly Reviews**: Team syncs to ensure relevance
- **ADR Updates**: New architectural decisions added as ADRs
- **Implementation Feedback**: Specification updated based on learnings
- **Cross-Reference Maintenance**: Document links validated regularly
- **Version Bumping**: Versions updated per [Semantic Versioning](https://semver.org/)

---

## Document Status Legend

| Status | Meaning |
|--------|---------|
| ✅ Complete | Document written and validated |
| 🔄 In Progress | Currently being written |
| ⏳ Pending | Queued for writing |
| ❌ Blocked | Waiting for dependencies |
| 🔄 Review | Awaiting team review |

---

## Contributing to This Specification

See [CONTRIBUTION_GUIDELINES.md](./CONTRIBUTION_GUIDELINES.md) (TBD) for:
- How to propose changes
- ADR process for new decisions
- Specification review process
- Versioning guidelines

---

## Document Dependencies

```
CHANGELOG.md (this file)
    ↑
    └─ References changes to all other documents
       which may depend on each other
```

Detailed dependency map in [TABLE_OF_CONTENTS.md](./TABLE_OF_CONTENTS.md).

---

**Last Updated**: 2026-07-20  
**Maintainers**: Architecture Team
