# Solstice Architecture

**Purpose**: Define the overall system architecture, design philosophy, and high-level design of the Solstice quantitative trading platform.

**Scope**: This document covers system-level architecture, core design principles, component organization, and data flow patterns.

**Status**: Draft  
**Version**: 1.0.0-draft

---

## Executive Summary

Solstice is a production-grade, Rust-based quantitative trading platform designed to discover, evaluate, and execute statistical arbitrage opportunities on the Solana blockchain. The system prioritizes:

- **Reliability**: Production-quality code with comprehensive test coverage
- **Maintainability**: Modular architecture with clear separation of concerns
- **Extensibility**: Plugin-based strategy framework for customization
- **Performance**: Optimized for high-end consumer hardware with fiber internet
- **Observability**: Comprehensive logging, monitoring, and metrics collection

The platform operates in three modes:
1. **Backtesting**: Historical analysis and strategy validation
2. **Paper Trading**: Risk-free simulation against live data
3. **Live Trading**: Production execution with capital deployment

---

## Design Philosophy

### Core Principles

1. **Specification-First Development**: Complete technical documentation before implementation
2. **Strongly Typed**: Leverage Rust's type system for correctness
3. **Event-Driven**: Market events drive the system's operation
4. **Plugin Architecture**: Strategies plug into the framework without modification of core
5. **Production Quality**: No prototypes; all code meets institutional standards
6. **Modular Crates**: Each Rust crate has a single, well-defined responsibility
7. **Fail-Safe Defaults**: Systems default to conservative behavior; risk is opt-in
8. **Observable**: Every significant action is logged and metricated

### Non-Goals

- HFT/colocation-dependent performance
- Support for emerging/illiquid token trading
- Centralized exchange integration (focus on blockchain-native liquidity)
- Consumer-friendly UI (institutional/developer audience)
- Autonomous learning (all strategy parameters are explicit and tunable)

---

## System Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         SOLSTICE PLATFORM                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────┐   │
│  │   Market Data    │  │  Strategy Layer  │  │  Execution   │   │
│  │   Ingestion      │  │                  │  │  & Risk Mgmt │   │
│  ├──────────────────┤  ├──────────────────┤  ├──────────────┤   │
│  │ • Yellowstone    │  │ • Stat Arbs      │  │ • Position   │   │
│  │ • Solana RPC     │  │ • Fair Value     │  │   Sizing     │   │
│  │ • DEX Quotes     │  │ • Signal Gen     │  │ • Risk Limits│   │
│  │ • Price Feeds    │  │ • Portfolio      │  │ • Execution  │   │
│  │                  │  │   Management     │  │   Planning   │   │
│  └──────────────────┘  └──────────────────┘  └──────────────┘   │
│           │                     │                     │           │
│           └─────────────────────┴─────────────────────┘           │
│                          │                                        │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │              Event Bus & Core Services                      │  │
│  │  • State Management  • Cache Layer  • Job Queue             │  │
│  └────────────────────────────────────────────────────────────┘  │
│                          │                                        │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │              Data & Storage Layer                          │  │
│  │  • PostgreSQL + TimescaleDB  • Redis  • File Storage       │  │
│  └────────────────────────────────────────────────────────────┘  │
│                          │                                        │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │            Transaction Execution & Settlement              │  │
│  │  • Jito Bundle Engine  • MEV Protection  • Fee Optimization│  │
│  └────────────────────────────────────────────────────────────┘  │
│                          │                                        │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │            Blockchain & External Systems                   │  │
│  │  • Solana Network  • Jupiter  • DEXes  • RPC Providers     │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                   │
│  ┌──────────────┐  ┌─────────────────┐  ┌──────────────────┐   │
│  │ REST API     │  │ WebSocket API   │  │ Grafana/Prometheus
│  │              │  │                 │  │                  │   │
│  │ • Status     │  │ • Market Events │  │ • Metrics        │   │
│  │ • Config     │  │ • Positions     │  │ • Dashboards     │   │
│  │ • Trading    │  │ • Orders        │  │ • Alerts         │   │
│  └──────────────┘  └─────────────────┘  └──────────────────┘   │
│                                                                   │
└─────────────────────────────────────────────────────────────────┘
```

---

## Architectural Layers

### 1. Data Ingestion Layer

Captures market data from multiple sources:
- **Yellowstone gRPC**: Solana account state changes (atomic market data)
- **Solana RPC**: On-chain state queries and transaction status
- **DEX APIs**: Direct quote streams from DEX protocols
- **Price Feeds**: Supplementary price data from external sources

**Key Characteristics**:
- High-throughput, low-latency ingestion
- Normalized into common market event format
- Deduplicated and validated
- Persisted for historical analysis

See [MARKET_DATA.md](./MARKET_DATA.md), [YELLOWSTONE.md](./YELLOWSTONE.md)

### 2. Strategy & Analysis Layer

Transforms market data into trading signals:
- **Statistical Arbitrage Engine**: Identifies mispricing opportunities
- **Fair Value Computation**: Calculates intrinsic prices
- **Signal Generation**: Evaluates trading signals based on multiple factors
- **Portfolio Management**: Maintains position and rebalancing logic
- **Risk Analysis**: Continuous risk assessment and constraint evaluation

**Key Characteristics**:
- Plugin-based strategy framework
- Real-time computation
- Deterministic and reproducible
- Testable against historical data

See [STRATEGY_FRAMEWORK.md](./STRATEGY_FRAMEWORK.md), [STAT_ARBS.md](./STAT_ARBS.md)

### 3. Execution & Risk Layer

Executes trading decisions with risk controls:
- **Position Sizing**: Calculates trade quantities based on risk parameters
- **Execution Planning**: Determines optimal trade execution path
- **Risk Gating**: Enforces hard risk limits
- **Transaction Building**: Constructs optimized Solana transactions
- **Fee Optimization**: Minimizes transaction costs

**Key Characteristics**:
- Conservative position sizing
- Multiple validation gates
- Clear fail-safe mechanisms
- Comprehensive audit trail

See [EXECUTION.md](./EXECUTION.md), [RISK_MANAGEMENT.md](./RISK_MANAGEMENT.md)

### 4. Blockchain Integration Layer

Interacts with Solana and DEX protocols:
- **Jito Bundle Engine**: MEV-protected transaction execution
- **DEX Integration**: Jupiter, Raydium, Orca, Meteora, Phoenix, OpenBook
- **RPC Abstraction**: Unified interface to Solana state
- **Transaction Settlement**: Monitors confirmation and handles failures

**Key Characteristics**:
- Abstracted DEX interfaces
- MEV protection by default
- Automatic fallback mechanisms
- Comprehensive error handling

See [JITO_INTEGRATION.md](./JITO_INTEGRATION.md), [DEX_INTEGRATIONS.md](./DEX_INTEGRATIONS.md)

### 5. Storage Layer

Persists all platform data:
- **PostgreSQL + TimescaleDB**: Time-series metrics, trading history, market data
- **Redis**: Caching, state management, pub/sub
- **File Storage**: Configuration, logs, backups

**Key Characteristics**:
- Redundant storage for critical data
- Time-series optimized for analytics
- Sub-second query performance for hot data
- Archived data for compliance

See [DATABASE.md](./DATABASE.md), [REDIS_ARCHITECTURE.md](./REDIS_ARCHITECTURE.md)

### 6. APIs & Observability Layer

Exposes platform capabilities and operational metrics:
- **REST API**: Configuration, status, historical queries
- **WebSocket API**: Real-time market events and position updates
- **Prometheus Metrics**: Operational metrics collection
- **Grafana Dashboards**: Visual monitoring and alerting
- **Logging**: Structured, queryable event logs

**Key Characteristics**:
- OpenAPI-documented endpoints
- Authentication and rate limiting
- Real-time data streaming
- Comprehensive operational visibility

See [REST_API.md](./REST_API.md), [WEBSOCKET_API.md](./WEBSOCKET_API.md), [MONITORING.md](./MONITORING.md)

---

## Core Data Flow

### Market Event Processing

```
Market Data Sources
         ↓
  [Ingestion Adapters]
         ↓
  Normalize to Event
         ↓
  [Event Bus]
         ↓
   ┌─────┴─────┐
   ↓           ↓
Strategy    Cache Update
Analysis
   ↓
Signal Generation
   ↓
Execution Planning
   ↓
Risk Evaluation
   ↓
   ├─→ Blocked (risk limit)
   ├─→ Pending (awaiting conditions)
   └─→ Execute (send to blockchain)
```

### Trading Execution Flow

```
Trading Signal
      ↓
Position Sizing
      ↓
Risk Validation
      ↓
Execution Planning
      ↓
Transaction Builder
   (via DEX routes)
      ↓
Fee Optimization
      ↓
Jito Bundle Creation
      ↓
Bundle Submission
      ↓
Confirmation Monitoring
      ↓
Settlement & Recording
```

---

## Workspace Organization

The Solstice platform is organized as a Rust workspace with specialized crates:

```
solstice/
├── crates/
│   ├── solstice-core/          # Core types, traits, abstractions
│   ├── solstice-market-data/   # Market data ingestion
│   ├── solstice-blockchain/    # Blockchain integration (Solana RPC, Yellowstone)
│   ├── solstice-dex/           # DEX protocol implementations
│   ├── solstice-strategy/      # Strategy framework and engines
│   ├── solstice-execution/     # Execution and risk management
│   ├── solstice-storage/       # Data persistence (DB, cache, etc.)
│   ├── solstice-api/           # REST and WebSocket APIs
│   ├── solstice-simulation/    # Backtesting and paper trading
│   └── solstice-cli/           # Command-line interface
├── dashboard/                  # React frontend (TypeScript)
└── docs/                       # Technical specification
```

See [WORKSPACE.md](./WORKSPACE.md) for detailed crate responsibilities.

---

## Key Design Decisions

### 1. Event-Driven Architecture

**Decision**: The core platform operates on market events, not polling.

**Rationale**:
- Lower latency and more responsive to market changes
- Better resource utilization
- Enables high-throughput data ingestion
- Natural fit for blockchain state changes

### 2. Plugin-Based Strategy Framework

**Decision**: Strategies are plugins, not hardcoded into the platform.

**Rationale**:
- Separation of platform from business logic
- Multiple concurrent strategies without modification
- Easy to test and version strategies independently
- Enables internal and external strategy development

### 3. Fail-Safe Risk Management

**Decision**: Risk controls are hard limits that cannot be overridden at runtime.

**Rationale**:
- Prevents catastrophic losses from bugs or misconfiguration
- Maintains trust in automated system
- Conservative position sizing by default
- Clear audit trail of risk decisions

### 4. Yellowstone as Primary Market Feed

**Decision**: Yellowstone gRPC is the primary market data source.

**Rationale**:
- Atomic account state changes (ordered)
- Lower latency than polling RPC
- Natural ordering for MEV-aware execution
- Validator-native data source

### 5. Jito Bundle Engine for Execution

**Decision**: Jito bundles are the default execution mechanism.

**Rationale**:
- MEV protection by default
- Atomic execution guarantees
- Reduced failure modes
- Better fee optimization

### 6. Rust for Core Platform

**Decision**: Core platform in Rust; React TypeScript for UI.

**Rationale**:
- Type safety and memory safety for production reliability
- Performance for high-throughput data processing
- Rust ecosystem strength in systems programming
- React for accessible, maintainable UI

### 7. PostgreSQL + TimescaleDB for Historical Data

**Decision**: PostgreSQL + TimescaleDB for all persistent data.

**Rationale**:
- Time-series optimized for trading metrics
- ACID compliance for financial accuracy
- Query performance for analytics
- Single source of truth for compliance

See [DESIGN_RATIONALE.md](./DESIGN_RATIONALE.md) for complete decision matrix.

---

## Component Responsibilities

| Component | Responsibility | Key Concerns |
|-----------|-----------------|--------------|
| Market Data Ingestion | Capture and normalize market data | Latency, throughput, accuracy |
| Strategy Engine | Identify trading opportunities | Signal quality, latency |
| Risk Management | Enforce position and risk limits | Loss prevention, compliance |
| Execution Engine | Build and submit transactions | Success rate, cost optimization |
| Storage | Persist all platform data | Consistency, performance, recovery |
| APIs | External access and monitoring | Availability, security, performance |
| Observability | Operational visibility | Latency, storage, actionability |

---

## Failure Modes & Resilience

### Critical Failure Modes

1. **Market Data Loss**
   - Mitigation: Multiple redundant data sources
   - Recovery: Automatic fallback to secondary feeds
   - Impact: Strategy may pause but positions are maintained

2. **RPC Node Failure**
   - Mitigation: Multiple RPC node connections
   - Recovery: Automatic failover to backup RPC
   - Impact: Temporary query delays, no execution impact

3. **Database Unavailability**
   - Mitigation: Read replicas, connection pooling
   - Recovery: Queued operations replay on reconnection
   - Impact: Analytics unavailable, trading continues

4. **Network Partition**
   - Mitigation: Conservative timeouts, position tracking
   - Recovery: Manual reconciliation of blockchain state
   - Impact: Trading pauses until confirmed

5. **Execution Failure**
   - Mitigation: Pre-execution validation, fallback routes
   - Recovery: Automatic retry with adjusted parameters
   - Impact: Missed trade opportunity, no capital loss

See [DISASTER_RECOVERY.md](./DISASTER_RECOVERY.md) for detailed recovery procedures.

---

## Performance Characteristics

### Target Performance Metrics

| Metric | Target | Rationale |
|--------|--------|-----------|
| Market Data Latency | < 500ms | Acceptable for statistical arbitrage |
| Strategy Signal Latency | < 100ms | Ensures timely response to opportunities |
| Execution Latency | < 2s | Accounts for bundling and network RTT |
| Throughput (events/sec) | 10,000+ | Sufficient for multi-strategy operation |
| Storage Query Latency (hot) | < 100ms | UI responsiveness |
| Storage Query Latency (cold) | < 5s | Analytics acceptable range |

### Scaling Targets

- Support 100+ concurrent open positions
- Process 10,000+ market events per second
- Maintain 1 year of time-series data
- Support 5+ concurrent strategies

---

## Testing & Validation

### Test Strategy

1. **Unit Tests**: Individual component correctness
2. **Integration Tests**: Component interaction
3. **Simulation Tests**: Historical data replay
4. **Paper Trading**: Live data simulation
5. **Chaos Tests**: Failure mode recovery

See [TESTING_STRATEGY.md](./TESTING_STRATEGY.md) for complete testing framework.

---

## Security Considerations

### Threat Model

1. **Private Key Compromise**: Funds at risk
   - Mitigation: Hardware wallet support, multi-sig optional
   
2. **Transaction Interception**: MEV/sandwich attacks
   - Mitigation: Jito bundles, private RPC
   
3. **Configuration Injection**: Malicious parameter modification
   - Mitigation: Signed configuration, audit logging
   
4. **Data Breach**: Sensitive trading data exposed
   - Mitigation: Encryption at rest, TLS in transit, access controls

See [SECURITY.md](./SECURITY.md) for detailed security architecture.

---

## Operational Modes

### 1. Backtesting Mode

- Replays historical market data
- Runs strategies against past price action
- Produces performance metrics and reports
- Deterministic and reproducible
- Used for strategy validation before live trading

### 2. Paper Trading Mode

- Uses live market data
- Simulates order execution without submitting
- Measures actual latency and slippage
- Validates strategy performance in real-time
- No capital deployed

### 3. Live Trading Mode

- Monitors for trading signals in real-time
- Submits transactions to blockchain
- Monitors positions and risk metrics
- Collects performance data
- Can be paused/stopped at any time

---

## Future Extension Points

1. **Additional DEX Protocols**: Plugin architecture enables new DEX integrations
2. **Machine Learning Strategies**: Strategy framework supports ML-based signals
3. **Cross-Chain Trading**: Can extend beyond Solana to other chains
4. **Advanced Risk Models**: Extensible risk management framework
5. **External Data Feeds**: Pluggable market data sources
6. **Multi-Account Execution**: Support multiple trading accounts

---

## Related Documents

- [WORKSPACE.md](./WORKSPACE.md) - Rust workspace and crate organization
- [DESIGN_RATIONALE.md](./DESIGN_RATIONALE.md) - Detailed design decisions
- [MARKET_DATA.md](./MARKET_DATA.md) - Market data ingestion architecture
- [STRATEGY_FRAMEWORK.md](./STRATEGY_FRAMEWORK.md) - Strategy plugin framework
- [EXECUTION.md](./EXECUTION.md) - Execution and transaction building
- [MONITORING.md](./MONITORING.md) - Observability and monitoring
- [DEPLOYMENT.md](./DEPLOYMENT.md) - Production deployment

---

**Next**: [WORKSPACE.md](./WORKSPACE.md)
