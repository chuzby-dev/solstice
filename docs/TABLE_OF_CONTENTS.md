# Solstice Technical Specification - Table of Contents

**Status**: In Development  
**Last Updated**: 2026-07-20  
**Specification Version**: 1.0.0-draft

---

## Core Architecture

1. [ARCHITECTURE.md](./ARCHITECTURE.md) - System architecture overview, design philosophy, and high-level design
2. [WORKSPACE.md](./WORKSPACE.md) - Rust workspace structure and crate organization
3. [DESIGN_RATIONALE.md](./DESIGN_RATIONALE.md) - Key architectural decisions and trade-offs

## Market Data & Blockchain Integration

4. [MARKET_DATA.md](./MARKET_DATA.md) - Market data ingestion pipeline
5. [YELLOWSTONE.md](./YELLOWSTONE.md) - Yellowstone gRPC architecture and integration
6. [SOLANA_RPC.md](./SOLANA_RPC.md) - Solana RPC abstraction layer
7. [DEX_INTEGRATIONS.md](./DEX_INTEGRATIONS.md) - Jupiter, Raydium, Orca, Meteora, Phoenix, OpenBook integrations

## Trading Engine

8. [STRATEGY_FRAMEWORK.md](./STRATEGY_FRAMEWORK.md) - Plugin-based strategy framework
9. [STAT_ARBS.md](./STAT_ARBS.md) - Statistical arbitrage engine and signal generation
10. [FAIR_VALUE.md](./FAIR_VALUE.md) - Fair value computation and pricing models
11. [PORTFOLIO_MANAGEMENT.md](./PORTFOLIO_MANAGEMENT.md) - Portfolio management and rebalancing
12. [RISK_MANAGEMENT.md](./RISK_MANAGEMENT.md) - Risk management framework and constraints
13. [POSITION_SIZING.md](./POSITION_SIZING.md) - Position sizing algorithms

## Execution & Optimization

14. [EXECUTION.md](./EXECUTION.md) - Execution planner and transaction builder
15. [SIMULATION.md](./SIMULATION.md) - Simulation engine and backtesting
16. [FEE_OPTIMIZATION.md](./FEE_OPTIMIZATION.md) - Fee optimization strategies
17. [JITO_INTEGRATION.md](./JITO_INTEGRATION.md) - Jito Block Engine integration and MEV protection
18. [BUNDLE_MANAGEMENT.md](./BUNDLE_MANAGEMENT.md) - Bundle creation, management, and monitoring

## Storage & Data

19. [DATABASE.md](./DATABASE.md) - Database schema and PostgreSQL + TimescaleDB design
20. [REDIS_ARCHITECTURE.md](./REDIS_ARCHITECTURE.md) - Redis architecture and caching strategy
21. [HISTORICAL_DATA.md](./HISTORICAL_DATA.md) - Historical data collection and retention

## Analytics & Backtesting

22. [BACKTESTING.md](./BACKTESTING.md) - Backtesting engine and simulation framework
23. [PAPER_TRADING.md](./PAPER_TRADING.md) - Paper trading mode and validation
24. [PERFORMANCE_ANALYTICS.md](./PERFORMANCE_ANALYTICS.md) - Performance metrics and analytics

## APIs & User Interface

25. [REST_API.md](./REST_API.md) - REST API specification and endpoints
26. [WEBSOCKET_API.md](./WEBSOCKET_API.md) - WebSocket API and real-time subscriptions
27. [DASHBOARD.md](./DASHBOARD.md) - React dashboard architecture and components
28. [AUTHENTICATION.md](./AUTHENTICATION.md) - Authentication and authorization

## Operations & Observability

29. [LOGGING.md](./LOGGING.md) - Logging strategy and structured logging
30. [MONITORING.md](./MONITORING.md) - Monitoring and observability framework
31. [PROMETHEUS_METRICS.md](./PROMETHEUS_METRICS.md) - Prometheus metrics and metric strategy
32. [GRAFANA_DASHBOARDS.md](./GRAFANA_DASHBOARDS.md) - Grafana dashboards and visualization
33. [CONFIGURATION.md](./CONFIGURATION.md) - Configuration system and management
34. [DEPLOYMENT.md](./DEPLOYMENT.md) - Docker deployment and container architecture

## Development & Testing

35. [TESTING_STRATEGY.md](./TESTING_STRATEGY.md) - Testing strategy and test framework
36. [CODING_STANDARDS.md](./CODING_STANDARDS.md) - Rust coding standards and conventions
37. [CI_CD.md](./CI_CD.md) - CI/CD pipeline and automation

## Governance & Operations

38. [SECURITY.md](./SECURITY.md) - Security architecture and threat model
39. [DISASTER_RECOVERY.md](./DISASTER_RECOVERY.md) - Disaster recovery and business continuity
40. [OPERATIONAL_RUNBOOKS.md](./OPERATIONAL_RUNBOOKS.md) - Operational runbooks and procedures
41. [ADR_TEMPLATE.md](./ADR_TEMPLATE.md) - Architecture Decision Record template
42. [CONTRIBUTION_GUIDELINES.md](./CONTRIBUTION_GUIDELINES.md) - Contribution guidelines and review process
43. [ACCEPTANCE_CRITERIA.md](./ACCEPTANCE_CRITERIA.md) - Acceptance criteria for features

## Project Management

44. [ROADMAP.md](./ROADMAP.md) - Development roadmap and milestones
45. [CHANGELOG.md](./CHANGELOG.md) - Changelog and version history

---

## How to Use This Specification

- **Architecture Overview**: Start with [ARCHITECTURE.md](./ARCHITECTURE.md)
- **Understanding Crates**: See [WORKSPACE.md](./WORKSPACE.md)
- **Key Decisions**: Review [DESIGN_RATIONALE.md](./DESIGN_RATIONALE.md)
- **Implementation Planning**: Reference [TESTING_STRATEGY.md](./TESTING_STRATEGY.md) and [CODING_STANDARDS.md](./CODING_STANDARDS.md)
- **Operational Setup**: See [DEPLOYMENT.md](./DEPLOYMENT.md) and [OPERATIONAL_RUNBOOKS.md](./OPERATIONAL_RUNBOOKS.md)

---

## Document Status

| Document | Status | Completed |
|----------|--------|-----------|
| TABLE_OF_CONTENTS.md | ✅ Complete | 2026-07-20 |
| ARCHITECTURE.md | ✅ Complete | 2026-07-20 |
| WORKSPACE.md | ✅ Complete | 2026-07-20 |
| DESIGN_RATIONALE.md | ✅ Complete | 2026-07-20 |
| ROADMAP.md | ✅ Complete | 2026-07-20 |
| CHANGELOG.md | ✅ Complete | 2026-07-20 |
| CONFIGURATION.md | ✅ Complete | 2026-07-20 |
| MARKET_DATA.md | ✅ Complete | 2026-07-20 |
| YELLOWSTONE.md | ✅ Complete | 2026-07-20 |
| SOLANA_RPC.md | ✅ Complete | 2026-07-20 |
| DEX_INTEGRATIONS.md | ✅ Complete | 2026-07-20 |
| STRATEGY_FRAMEWORK.md | ✅ Complete | 2026-07-20 |
| RISK_MANAGEMENT.md | ✅ Complete | 2026-07-20 |
| STAT_ARBS.md | ⏳ Pending | - |
| FAIR_VALUE.md | ⏳ Pending | - |
| PORTFOLIO_MANAGEMENT.md | ⏳ Pending | - |
| RISK_MANAGEMENT.md | ⏳ Pending | - |
| POSITION_SIZING.md | ⏳ Pending | - |
| EXECUTION.md | ⏳ Pending | - |
| SIMULATION.md | ⏳ Pending | - |
| FEE_OPTIMIZATION.md | ⏳ Pending | - |
| JITO_INTEGRATION.md | ⏳ Pending | - |
| BUNDLE_MANAGEMENT.md | ⏳ Pending | - |
| DATABASE.md | ⏳ Pending | - |
| REDIS_ARCHITECTURE.md | ⏳ Pending | - |
| HISTORICAL_DATA.md | ⏳ Pending | - |
| BACKTESTING.md | ⏳ Pending | - |
| PAPER_TRADING.md | ⏳ Pending | - |
| PERFORMANCE_ANALYTICS.md | ⏳ Pending | - |
| REST_API.md | ⏳ Pending | - |
| WEBSOCKET_API.md | ⏳ Pending | - |
| DASHBOARD.md | ⏳ Pending | - |
| AUTHENTICATION.md | ⏳ Pending | - |
| LOGGING.md | ⏳ Pending | - |
| MONITORING.md | ⏳ Pending | - |
| PROMETHEUS_METRICS.md | ⏳ Pending | - |
| GRAFANA_DASHBOARDS.md | ⏳ Pending | - |
| DEPLOYMENT.md | ⏳ Pending | - |
| TESTING_STRATEGY.md | ⏳ Pending | - |
| CODING_STANDARDS.md | ⏳ Pending | - |
| CI_CD.md | ⏳ Pending | - |
| SECURITY.md | ⏳ Pending | - |
| DISASTER_RECOVERY.md | ⏳ Pending | - |
| OPERATIONAL_RUNBOOKS.md | ⏳ Pending | - |
| ADR_TEMPLATE.md | ⏳ Pending | - |
| CONTRIBUTION_GUIDELINES.md | ⏳ Pending | - |
| ACCEPTANCE_CRITERIA.md | ⏳ Pending | - |

---

## Related Documents

- [Project README](../README.md)
- [Contributing](../CONTRIBUTING.md)
- [License](../LICENSE)
