# Solstice Design Rationale

**Purpose**: Explain key architectural decisions, trade-offs, and rationale for design choices.

**Scope**: Major decisions affecting platform design, technology selection, and operational mode.

**Status**: Draft  
**Version**: 1.0.0-draft

---

## Decision Matrix

This document captures significant architectural decisions using the ADR (Architecture Decision Record) format.

---

## ADR-001: Event-Driven Architecture for Core Platform

**Status**: ✅ Accepted

**Context**:
The platform needs to process market data from Solana blockchain in real-time and make trading decisions quickly. Possible approaches include:
1. Polling-based: Periodically query market state
2. Event-driven: React to market events as they occur

**Decision**:
Adopt event-driven architecture where the core platform is driven by market events from Yellowstone gRPC and other data sources.

**Rationale**:
- **Lower Latency**: React immediately to market changes rather than waiting for poll intervals
- **Resource Efficiency**: No wasted CPU cycles polling when no events occurred
- **Natural Data Model**: Blockchain events are inherently event-based
- **Backpressure Handling**: Event queue enables rate limiting and flow control
- **Testability**: Events can be replayed for simulation and testing
- **Ordering Guarantees**: Event ordering is preserved from Yellowstone

**Consequences**:
- (+) Responsive to market changes
- (+) Lower CPU utilization
- (+) Natural fit with blockchain data
- (-) Requires async/await architecture throughout
- (-) Complexity in state management (eventual consistency)
- (-) Testing requires event sequencing logic

**Related Decision**: ADR-002 (Async/Await throughout)

**See Also**: [ARCHITECTURE.md](./ARCHITECTURE.md), [MARKET_DATA.md](./MARKET_DATA.md)

---

## ADR-002: Async/Await with Tokio Runtime

**Status**: ✅ Accepted

**Context**:
Supporting event-driven architecture requires efficient concurrency for:
- Listening to multiple market data sources
- Processing strategy evaluations
- Managing multiple concurrent orders
- Handling API requests

Options:
1. OS threads (std::thread)
2. Async/await with Tokio
3. Hybrid approach

**Decision**:
Use Tokio async runtime throughout the platform for all I/O and concurrency.

**Rationale**:
- **Scalability**: Thousands of concurrent tasks on single machine
- **Efficiency**: Light-weight tasks vs. OS threads
- **Ecosystem**: Rich Tokio ecosystem (http, database, etc.)
- **Performance**: Lower context-switching overhead
- **Rust Best Practice**: Async/await is standard for systems programming

**Consequences**:
- (+) Scalable to many concurrent operations
- (+) Efficient resource utilization
- (-) Requires async-aware code throughout
- (-) Async Rust has learning curve
- (-) Some libraries lack async support
- (-) Stack traces harder to debug

**Constraints**:
- All I/O operations must be async-compatible
- Blocking operations must use `task::spawn_blocking()`

**Related Decisions**: ADR-001 (Event-Driven), ADR-003 (Rust Language)

**See Also**: [WORKSPACE.md](./WORKSPACE.md)

---

## ADR-003: Rust Programming Language

**Status**: ✅ Accepted

**Context**:
Building a production financial system requires:
- Memory safety and correctness
- High performance
- Type safety
- Reliability

Evaluated options:
1. Rust
2. Go
3. C/C++
4. Python

**Decision**:
Implement core platform in Rust, with TypeScript frontend for web UI.

**Rationale**:
- **Memory Safety**: Rust's borrow checker prevents entire categories of bugs
- **Type Safety**: Strong static typing catches errors at compile time
- **Performance**: Zero-cost abstractions, no garbage collector
- **Concurrency**: Excellent async/await and thread safety guarantees
- **Ecosystem**: Strong libraries for blockchain, HTTP, databases
- **Financial Appropriateness**: Used by major exchanges and financial systems
- **No Runtime Overhead**: Compiled to native code

**Consequences**:
- (+) Extremely high code quality and reliability
- (+) Compile-time verification of safety properties
- (+) High performance
- (-) Steeper learning curve
- (-) Slower development iteration (compilation time)
- (-) Smaller ecosystem than Go/Python (but growing rapidly)
- (-) Hiring pool smaller than Go/Python

**Trade-off Rationale**:
The 3-5% slower development vs. benefits in safety and performance is worthwhile for a financial system where correctness is paramount.

**Frontend Exception**:
React + TypeScript chosen for web UI to leverage:
- Ecosystem maturity for dashboards
- Fast development iteration
- Developer familiarity
- Access to component libraries

**Related Decisions**: ADR-004 (Workspace Structure)

**See Also**: [CODING_STANDARDS.md](./CODING_STANDARDS.md)

---

## ADR-004: Monorepo Workspace Structure

**Status**: ✅ Accepted

**Context**:
The platform consists of multiple independent subsystems (market data, strategy, execution, etc.). These could be organized as:
1. Monorepo (single git repository)
2. Polyrepo (multiple repositories)
3. Mono-service (single crate)

**Decision**:
Use Rust workspace monorepo with multiple specialized crates.

**Rationale**:
- **Atomic Consistency**: All code changes commit together
- **Shared Dependencies**: Single `Cargo.lock` ensures consistent versions
- **Easy Refactoring**: Moving code between crates is straightforward
- **Clear Boundaries**: Each crate has single responsibility
- **Parallel Compilation**: Crates compile independently
- **Testing**: Integration tests span multiple crates
- **Deployment**: Ship compiled binaries without source

**Consequences**:
- (+) Clear separation of concerns
- (+) Easy to understand module boundaries
- (+) Facilitates parallel development
- (+) Single source of truth for versions
- (-) Requires discipline to prevent circular dependencies
- (-) Larger codebase to manage
- (-) Can be complex for new developers

**Related Decision**: ADR-005 (Trait-Based Abstraction)

**See Also**: [WORKSPACE.md](./WORKSPACE.md), [CI_CD.md](./CI_CD.md)

---

## ADR-005: Trait-Based Abstraction for Core Components

**Status**: ✅ Accepted

**Context**:
Different components need to swap implementations (e.g., different DEX protocols, different market data sources, different strategies). Options:
1. Trait objects (dynamic dispatch)
2. Enum-based dispatch
3. Generic trait bounds (static dispatch)
4. Hardcoded implementations

**Decision**:
Use trait objects and generic trait bounds for abstraction, with enum dispatch only where performance is critical.

**Rationale**:
- **Modularity**: New implementations can be added without changing existing code
- **Testability**: Mock implementations are trivial to create
- **Extensibility**: Strategy framework enables third-party extensions
- **Maintainability**: Clear interface contracts

**Consequences**:
- (+) Extensible architecture
- (+) Easy to test with mocks
- (+) Plugin system possible
- (-) Trait objects have runtime overhead
- (-) Requires careful error handling (downcasting)
- (-) Generic code can be harder to reason about

**Performance Optimization**:
For hot paths (market data ingestion), use:
- Static dispatch (generic parameters)
- Inline hints
- Pre-allocated buffers
- Avoid trait object allocation

**Related Decision**: ADR-006 (Plugin-Based Strategy Framework)

**See Also**: [STRATEGY_FRAMEWORK.md](./STRATEGY_FRAMEWORK.md)

---

## ADR-006: Plugin-Based Strategy Framework

**Status**: ✅ Accepted

**Context**:
The platform needs to support multiple concurrent trading strategies without modifying core platform. Options:
1. Hardcoded strategies
2. Script-based strategies (Python, Lua)
3. Plugin system (dynamically loaded Rust crates)
4. Strategy specification language

**Decision**:
Implement strategy framework with strategy trait that pluggable strategies implement.

**Rationale**:
- **Type Safety**: Strategies are Rust code, compile-time checked
- **Performance**: No interpretation overhead, native compiled code
- **Flexibility**: Strategies can use all Rust ecosystem
- **Distribution**: Strategies distributed as compiled crates
- **No Script Language Dependency**: Reduces attack surface
- **Modularity**: Each strategy is independently testable

**Consequences**:
- (+) Type-safe, fully auditable strategies
- (+) High performance
- (+) Full ecosystem access
- (+) Clear plugin interface
- (-) Requires Rust knowledge for strategy development
- (-) Longer development cycle (must recompile platform)
- (-) Smaller strategy developer community initially

**Future Evolution**:
Could add:
- Strategy DSL for common patterns
- Lua/Python bridge for quick iterations (with safety caveats)
- Pre-built strategy library
- Strategy marketplace

**Related Decision**: ADR-005 (Trait-Based Abstraction)

**See Also**: [STRATEGY_FRAMEWORK.md](./STRATEGY_FRAMEWORK.md)

---

## ADR-007: Jito Bundle Engine for Execution

**Status**: ✅ Accepted

**Context**:
Executing trades on Solana requires choosing an execution method:
1. Direct RPC submission
2. Jito Bundle Engine (MEV-aware execution)
3. Flashbot-style private RPCs

**Decision**:
Use Jito Bundle Engine as default execution mechanism.

**Rationale**:
- **MEV Protection**: Bundles are atomic to Jito block; no sandwich attacks
- **Cost Optimization**: Jito handles optimal fee bidding
- **Atomic Execution**: Multi-leg trades execute atomically
- **Ordering Priority**: Determined by bidding, not mempool ordering
- **Reliability**: Jito has proved reliability track record
- **Ecosystem**: Primary tool in Solana MEV-aware world

**Consequences**:
- (+) MEV protection by default
- (+) Better fee optimization
- (+) Atomic multi-leg execution
- (+) Deterministic ordering
- (-) Fee premium for bundle inclusion
- (-) Dependent on Jito infrastructure
- (-) Must handle bundle rejection gracefully

**Fallback Strategy**:
If Jito unavailable, fall back to:
1. Private RPC endpoint
2. Direct submission with backoff-retry

**Related Decision**: ADR-008 (Yellowstone as Primary Feed)

**See Also**: [JITO_INTEGRATION.md](./JITO_INTEGRATION.md), [EXECUTION.md](./EXECUTION.md)

---

## ADR-008: Yellowstone gRPC as Primary Market Feed

**Status**: ✅ Accepted

**Context**:
Market data can come from:
1. Solana RPC polling
2. Yellowstone gRPC streaming
3. Third-party API aggregators
4. DEX protocol direct queries

**Decision**:
Use Yellowstone gRPC as primary real-time market data source.

**Rationale**:
- **Atomic State Changes**: Account state updates are ordered atomically
- **Lower Latency**: Streaming vs. polling
- **Ordering Guarantees**: Preserves transaction ordering
- **Bandwidth Efficiency**: Only pushes changed accounts
- **MEV-Awareness**: Accounts changed in block order
- **Validator-Native**: Direct connection to validator

**Consequences**:
- (+) Lowest latency market data
- (+) Atomic ordering
- (+) Natural fit with blockchain structure
- (-) Requires Yellowstone RPC endpoint (cost)
- (-) gRPC complexity
- (-) Account subscription management

**Supplementary Sources**:
Use DEX APIs to supplement:
- Order book snapshots
- Liquidity information
- Price aggregation

**RPC Fallback**:
If Yellowstone unavailable:
1. Fall back to polling RPC
2. Maintain last-known state
3. Resume Yellowstone when available

**Related Decision**: ADR-007 (Jito for Execution)

**See Also**: [MARKET_DATA.md](./MARKET_DATA.md), [YELLOWSTONE.md](./YELLOWSTONE.md)

---

## ADR-009: PostgreSQL + TimescaleDB for Historical Data

**Status**: ✅ Accepted

**Context**:
Platform needs to persist:
- Historical market data
- Trade history
- Position history
- Performance metrics

Options:
1. PostgreSQL + TimescaleDB
2. MongoDB/NoSQL
3. ClickHouse
4. File-based (Parquet)

**Decision**:
Use PostgreSQL + TimescaleDB for all persistent data.

**Rationale**:
- **ACID Compliance**: Financial accuracy requires transactional consistency
- **Time-Series Optimized**: TimescaleDB specifically for time-series data
- **Query Power**: SQL enables complex analytical queries
- **Performance**: Excellent for both OLTP and OLAP workloads
- **Open Source**: No vendor lock-in
- **Mature**: Proven in production at scale
- **Integration**: Excellent Rust library support (sqlx, tokio-postgres)

**Consequences**:
- (+) ACID consistency for financial data
- (+) Excellent query performance
- (+) Time-series specific optimizations
- (+) Strong analytical capabilities
- (-) Requires operational complexity
- (-) Vertical scaling limits
- (-) Schema changes require migration planning

**Scaling Strategy**:
For multi-terabyte scale:
1. Compression (TimescaleDB native)
2. Data archival
3. Read replicas for analytics
4. Sharding (if necessary, future)

**Related Decision**: ADR-010 (Redis for Caching)

**See Also**: [DATABASE.md](./DATABASE.md)

---

## ADR-010: Redis for Caching & State

**Status**: ✅ Accepted

**Context**:
Need fast access to:
- Recent market data
- Current positions
- Open orders
- Strategy state
- Cache for external queries

Options:
1. In-memory (HashMap in Rust)
2. Redis
3. Memcached
4. External cache (CDN)

**Decision**:
Use Redis for caching and temporary state management.

**Rationale**:
- **Sub-millisecond Latency**: In-memory key-value store
- **Persistence Options**: Can persist to disk for state recovery
- **Pub/Sub**: Built-in publish/subscribe for real-time updates
- **Data Structures**: Strings, lists, sets, sorted sets, hashes
- **Cluster Support**: Can scale horizontally if needed
- **Operational Maturity**: Widely deployed in production

**Consequences**:
- (+) Extremely fast access
- (+) Good for temporary state
- (+) Built-in pub/sub for events
- (+) Cluster support for HA
- (-) Requires separate infrastructure
- (-) Memory-limited (must fit in RAM)
- (-) Not primary data source
- (-) Operational complexity

**Caching Strategy**:
- Hot market data: 5-60 second TTL
- Position snapshots: 1 minute TTL
- Computed values: 10-60 seconds TTL
- Session data: 24 hour TTL

**Primary vs. Cache**:
- Redis: Fast, temporary, loss-tolerable
- PostgreSQL: Persistent, authoritative source of truth
- On-Disk: Permanent archive

**Related Decision**: ADR-009 (PostgreSQL)

**See Also**: [REDIS_ARCHITECTURE.md](./REDIS_ARCHITECTURE.md)

---

## ADR-011: Fail-Safe Risk Management

**Status**: ✅ Accepted

**Context**:
Risk management must prevent catastrophic losses. Options:
1. Soft limits (warnings only)
2. Hard limits (block execution)
3. Gradual position reduction
4. No risk management (trust strategy)

**Decision**:
Implement multiple layers of hard risk limits that cannot be overridden at runtime.

**Rationale**:
- **Loss Prevention**: Prevents catastrophic losses from bugs
- **Trust in Automation**: Operators can trust system won't blow up
- **Regulatory Compliance**: Financial institutions require hard stops
- **Correctness Verification**: Limits are conservative by default
- **Audit Trail**: All risk decisions logged

**Consequences**:
- (+) Prevents catastrophic losses
- (+) Maintains operator confidence
- (+) Enforceable guarantees
- (-) May miss profitable opportunities
- (-) Requires careful limit configuration
- (-) Cannot be easily adjusted during trading

**Risk Limit Tiers**:
1. **Position Limits**: Max position size per asset
2. **Notional Limits**: Max capital exposed
3. **Loss Limits**: Max daily/monthly loss
4. **Concentration Limits**: Max % of portfolio in one asset
5. **Leverage Limits**: Max margin/leverage

**Operator Adjustments**:
Risk limits can only be changed:
1. Via configuration file restart
2. Requires manual approval
3. Logged for audit trail
4. Take effect on next trading session

**Related Decision**: ADR-012 (Comprehensive Logging)

**See Also**: [RISK_MANAGEMENT.md](./RISK_MANAGEMENT.md)

---

## ADR-012: Comprehensive Structured Logging

**Status**: ✅ Accepted

**Context**:
Production system needs operational visibility. Options:
1. Minimal logging (errors only)
2. Structured logging (JSON, queryable)
3. Verbose logging (everything)
4. No logging (performance)

**Decision**:
Implement comprehensive structured logging throughout platform.

**Rationale**:
- **Debuggability**: Can reproduce issues from logs
- **Compliance**: Audit trail for regulatory requirements
- **Observability**: Understand what system is doing
- **Structured Data**: Queryable logs enable analytics
- **Performance Monitoring**: Baseline latencies, throughput
- **Incident Response**: Reconstruct trading activity

**Consequences**:
- (+) Complete audit trail
- (+) Easy debugging
- (+) Regulatory compliance
- (+) Performance monitoring
- (-) Storage overhead
- (-) Log query complexity
- (-) Privacy considerations (logs contain sensitive data)

**Logging Levels**:
- **ERROR**: Failures requiring intervention
- **WARN**: Degraded conditions, resource constraints
- **INFO**: Significant events (trades, position changes)
- **DEBUG**: Component-level actions (market data events)
- **TRACE**: Detailed internal state (rarely used)

**Structured Fields**:
All logs include:
- Timestamp
- Level
- Component/Module
- Message
- Contextual fields (prices, positions, etc.)
- Request/trace ID

**Related Decision**: ADR-011 (Risk Management), ADR-013 (Prometheus Metrics)

**See Also**: [LOGGING.md](./LOGGING.md), [MONITORING.md](./MONITORING.md)

---

## ADR-013: Prometheus + Grafana for Observability

**Status**: ✅ Accepted

**Context**:
Monitoring production platform requires:
- Real-time metrics collection
- Alerting on anomalies
- Historical trend analysis
- Performance dashboards

Options:
1. Prometheus + Grafana
2. ELK Stack (logs only)
3. Datadog/New Relic (SaaS)
4. Custom solution

**Decision**:
Use Prometheus for metrics collection and Grafana for visualization.

**Rationale**:
- **Time-Series Database**: Prometheus built for metrics
- **Operational Maturity**: Proven in production at scale
- **Open Source**: No vendor lock-in
- **Pull Model**: Servers pull metrics (no firewall issues)
- **Rich Queries**: PromQL for complex queries
- **Grafana Integration**: Excellent dashboard capabilities
- **Alerting**: Built-in alert rules
- **Cost**: Self-hosted, minimal cost

**Consequences**:
- (+) Comprehensive real-time metrics
- (+) Excellent query and visualization
- (+) Open source stack
- (-) Operational complexity
- (-) Storage and retention planning
- (-) Requires expertise to set up well
- (-) Pull model adds latency

**Key Metrics**:
- Market data ingestion rate
- Strategy signal latency
- Execution success rate
- Position tracking accuracy
- API response times
- Database query latency
- Resource utilization

**Related Decision**: ADR-012 (Structured Logging)

**See Also**: [MONITORING.md](./MONITORING.md), [PROMETHEUS_METRICS.md](./PROMETHEUS_METRICS.md)

---

## ADR-014: Specification-First Development

**Status**: ✅ Accepted

**Context**:
Building complex system like Solstice could follow:
1. Specification-first (document before code)
2. Test-driven (tests before code)
3. Agile iteration (minimal upfront design)

**Decision**:
Complete technical specification before implementation.

**Rationale**:
- **Alignment**: Team shared understanding before coding
- **Architecture Validation**: Design issues caught early
- **Scope Clarity**: All stakeholders know what's being built
- **Future Reference**: Specification serves as reference
- **Quality**: Disciplined approach yields better design
- **Parallelization**: Teams can work from specification

**Consequences**:
- (+) Clear architecture and requirements
- (+) Reduced scope creep
- (+) Better quality code
- (+) Easier onboarding
- (-) Slower initial development
- (-) Specification can become stale
- (-) Requires discipline to maintain

**Living Documentation**:
Specification is maintained as living document:
- Updates as discoveries are made
- Architecture Decision Records capture changes
- Weekly syncs to ensure relevance
- Code comments reference specification

**Related Decision**: ADR-003 (Rust)

**See Also**: [TABLE_OF_CONTENTS.md](./TABLE_OF_CONTENTS.md), [CONTRIBUTION_GUIDELINES.md](./CONTRIBUTION_GUIDELINES.md)

---

## ADR-015: Three-Mode Operation (Live/Paper/Backtest)

**Status**: ✅ Accepted

**Context**:
Platform development and deployment involves testing at multiple levels. Options:
1. Paper trading only before live
2. Simulation only before live
3. Multiple modes (backtest, paper, live)

**Decision**:
Support three operational modes: backtesting, paper trading, and live trading.

**Rationale**:
- **Risk Reduction**: Test thoroughly before risking capital
- **Confidence Building**: Paper trading provides real-time validation
- **Strategy Validation**: Backtest finds fundamental issues
- **Monitoring Preparation**: Live mode immediately after paper
- **Rapid Iteration**: Can iterate strategies safely
- **Operator Confidence**: Three layers of validation

**Consequences**:
- (+) Risk-free testing path
- (+) Confidence in strategies
- (+) Rapid iteration
- (-) Code complexity (three paths)
- (-) Testing complexity increases
- (-) Simulation/reality gap possible

**Mode Characteristics**:
| Mode | Data | Execution | Risk | Use Case |
|------|------|-----------|------|----------|
| Backtest | Historical | Simulated | None | Strategy validation |
| Paper | Live | Simulated | None | Real-time testing |
| Live | Live | Real | Actual | Production trading |

**Seamless Transition**:
Paper trading designed as drop-in replacement for live trading:
- Same code path
- Same API calls
- Just replace execution backend
- Enables quick transition to live

**Related Decision**: ADR-006 (Plugin-Based Strategies)

**See Also**: [BACKTESTING.md](./BACKTESTING.md), [PAPER_TRADING.md](./PAPER_TRADING.md)

---

## ADR-016: Axum Web Framework for APIs

**Status**: ✅ Accepted

**Context**:
Need HTTP server for REST API and WebSocket upgrades. Rust options:
1. Axum (high-level)
2. Hyper (low-level)
3. Actix-web (heavyweight)
4. Rocket (simpler, less flexible)

**Decision**:
Use Axum for both REST and WebSocket APIs.

**Rationale**:
- **Modern Async**: Built on Tokio, full async/await
- **Composable**: Middleware-based architecture
- **Type Safety**: Strong typing for route handlers
- **Performance**: Minimal overhead, high throughput
- **WebSocket Support**: Built-in upgrades
- **Error Handling**: Type-safe error responses
- **Community**: Strong backing from Tokio team

**Consequences**:
- (+) Modern, well-designed framework
- (+) Excellent performance
- (+) Composable middleware
- (+) Type-safe routes
- (-) Smaller community than Actix
- (-) Fewer third-party integrations
- (-) Requires understanding of Rust futures

**Related Decision**: ADR-002 (Async/Await with Tokio)

**See Also**: [REST_API.md](./REST_API.md), [WEBSOCKET_API.md](./WEBSOCKET_API.md)

---

## Summary: Key Trade-offs

| Decision | Gained | Sacrificed |
|----------|--------|-----------|
| **Rust** | Safety, Performance | Dev Speed, Hiring Pool |
| **Async/Await** | Scalability, Efficiency | Debugging Complexity |
| **Monorepo** | Atomic Changes, Clear Boundaries | Repo Complexity |
| **Traits** | Modularity, Testability | Runtime Overhead |
| **Plugins** | Extensibility, Modularity | Compilation Overhead |
| **Jito Bundles** | MEV Protection | Fee Premium |
| **Yellowstone** | Latency, Ordering | Cost, Complexity |
| **PostgreSQL** | Consistency, Query Power | Operational Complexity |
| **Redis** | Latency, Pub/Sub | Memory Constraints |
| **Risk Limits** | Loss Prevention | Flexibility |
| **Logging** | Debuggability, Compliance | Storage Overhead |
| **Prometheus** | Observability, Alerting | Operational Complexity |
| **Spec-First** | Clarity, Quality | Time-to-Code |
| **Three Modes** | Confidence, Iteration | Complexity |

---

## Future Decision Points

Decisions to revisit as platform evolves:

1. **Cross-Chain Support**: When to expand beyond Solana
2. **Machine Learning Strategies**: When to add non-parametric strategies
3. **Strategy Marketplace**: Mechanism for third-party strategies
4. **Multi-Account**: Supporting multiple trading accounts
5. **Sharding**: When single PostgreSQL instance insufficient
6. **Autonomous Learning**: If/when to add adaptive parameters

---

## Related Documents

- [ARCHITECTURE.md](./ARCHITECTURE.md) - System-level view
- [WORKSPACE.md](./WORKSPACE.md) - Crate organization
- [ADR_TEMPLATE.md](./ADR_TEMPLATE.md) - How to write new ADRs

---

**Next**: [CONFIGURATION.md](./CONFIGURATION.md)
