# Solstice Changelog

**Purpose**: Track specification document changes, releases, and version history.

**Format**: This changelog follows [Keep a Changelog](https://keepachangelog.com/).

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
