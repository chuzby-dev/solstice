# Solstice Configuration System

**Purpose**: Define configuration management, parameter system, and operational settings.

**Scope**: Configuration architecture, parameter types, validation, and runtime modification.

**Status**: Draft  
**Version**: 1.0.0-draft

---

## Overview

Solstice uses a hierarchical configuration system that balances:
- **Flexibility**: Parameters easily adjustable for different strategies and markets
- **Safety**: Conservative defaults and validation prevent misconfiguration
- **Auditability**: All configuration changes logged and versioned
- **Consistency**: Shared configuration across all platform components

---

## Configuration Architecture

```
Configuration Hierarchy (Lowest to Highest Priority)
    ↓
1. Built-in Defaults (hardcoded)
    ↓
2. config/default.toml (repository baseline)
    ↓
3. config/[environment].toml (environment-specific)
    ↓
4. config/local.toml (local machine overrides)
    ↓
5. Environment Variables (CI/CD, secrets)
    ↓
6. Runtime Parameters (API/CLI adjustments)
```

Each level overrides previous levels. Runtime parameters are the highest priority but are temporary (reset on restart).

---

## Configuration Format

Solstice uses **TOML** (Tom's Obvious, Minimal Language) for configuration files.

**Rationale for TOML**:
- Human-readable and editable
- Type-safe (strings, integers, floats, booleans, tables)
- Section-based organization (`[section]`)
- Comments supported
- Wide ecosystem support
- Less verbose than YAML, safer than JSON

### Directory Structure

```
solstice/
├── config/
│   ├── default.toml              # Default configuration (always loaded)
│   ├── development.toml          # Development overrides
│   ├── paper_trading.toml        # Paper trading settings
│   ├── staging.toml              # Staging environment
│   ├── production.toml           # Production settings (encrypted in repo)
│   ├── backtest.toml             # Backtesting defaults
│   └── local.toml                # Machine-local overrides (not in git)
├── schemas/
│   └── config.schema.toml        # Configuration schema with types
└── templates/
    └── example.toml              # Example configuration
```

---

## Configuration Sections

### 1. Platform Section

```toml
[platform]
name = "Solstice"
version = "1.0.0"
environment = "development"        # development, staging, production
mode = "paper"                     # paper, live, backtest
log_level = "info"                 # trace, debug, info, warn, error
enable_metrics = true
metrics_port = 9090
```

**Parameters**:
- `environment`: Controls which additional config files to load
- `mode`: Operational mode (see [PAPER_TRADING.md](./PAPER_TRADING.md))
- `log_level`: Global logging level

---

### 2. Market Data Section

```toml
[market_data]
# Primary feed
primary_feed = "yellowstone"       # yellowstone, rpc_polling, hybrid

# Yellowstone Configuration
[market_data.yellowstone]
grpc_endpoints = [
    "grpcs://api.mainnet-beta.solana.com:10900"
]
max_subscriptions = 100
subscription_buffer = 10000
timeout_seconds = 30
retry_enabled = true
retry_max_attempts = 3

# RPC Fallback Configuration
[market_data.rpc]
endpoints = [
    "https://api.mainnet-beta.solana.com",
    "https://solana-api.projectserum.com"
]
polling_interval_ms = 500
max_concurrent_queries = 50
timeout_seconds = 15
retry_enabled = true

# DEX Quote Sources
[market_data.dex_quotes]
enabled_dexes = ["jupiter", "raydium", "orca"]
quote_cache_ttl_seconds = 5
update_interval_ms = 100
```

**Parameters**:
- `primary_feed`: Which data source to prefer
- `grpc_endpoints`: Multiple Yellowstone endpoints for redundancy
- `polling_interval_ms`: How often to poll RPC (when using RPC feed)
- `quote_cache_ttl_seconds`: How long to cache DEX quotes

---

### 3. Blockchain Section

```toml
[blockchain]
# Network
network = "mainnet-beta"            # mainnet-beta, devnet, testnet
commitment = "confirmed"            # processed, confirmed, finalized

# RPC Endpoints
[blockchain.rpc]
endpoints = [
    "https://api.mainnet-beta.solana.com",
    "https://solana-api.projectserum.com"
]
connection_timeout_seconds = 10
request_timeout_seconds = 30
max_retries = 3
retry_delay_ms = 100

# Transaction Settings
[blockchain.transactions]
default_priority_fee_lamports = 5000
simulate_before_send = true
confirmation_timeout_seconds = 60
max_age_confirmations = 15
```

**Parameters**:
- `commitment`: Block confirmation level
- `default_priority_fee_lamports`: Transaction priority fee
- `simulate_before_send`: Pre-flight simulation for safety
- `confirmation_timeout_seconds`: How long to wait for confirmation

---

### 4. Jito Section

```toml
[jito]
enabled = true                      # Enable Jito bundle execution

# Bundle Engine
[jito.bundle_engine]
# Private RPC endpoint for Jito bundles
private_rpc = "http://localhost:8899"  # or Jito endpoint
public_rpc = "https://api.mainnet-beta.solana.com"

# Bundle settings
max_bundle_size = 5                 # Max transactions per bundle
tip_strategy = "adaptive"           # fixed, adaptive, market
base_tip_lamports = 10000
max_tip_lamports = 100000
tip_account = "Tj6rhAXabqdwqMLvxvYXrjvjSXvqeDQvanYsmKaD5aD"

# Failover
fallback_to_direct = true
fallback_delay_seconds = 2
```

**Parameters**:
- `enabled`: Whether to use Jito bundles
- `tip_strategy`: How to calculate bundle tip
- `fallback_to_direct`: Fall back to direct submission if Jito fails
- `max_bundle_size`: Limit transactions per bundle

---

### 5. Strategy Section

```toml
[strategy]
# Strategy framework
plugin_directory = "./strategies"   # Where to load strategy plugins
max_concurrent_strategies = 5
evaluation_interval_ms = 100
signal_batch_size = 100

# Default strategy parameters
[strategy.defaults]
min_confidence = 0.65               # 0.0 to 1.0
max_slippage_percent = 1.5
min_spread_basis_points = 5
position_decay_hours = 24

# Per-strategy overrides (example)
[strategy.stat_arbs]
enabled = true
name = "Statistical Arbitrage"
min_confidence = 0.70
max_positions = 20
correlation_threshold = 0.85
mean_reversion_window = 100
signal_lookback_bars = 200
```

**Parameters**:
- `plugin_directory`: Where strategy plugins are loaded from
- `evaluation_interval_ms`: How often to evaluate strategies
- `min_confidence`: Minimum signal confidence to trade

---

### 6. Risk Management Section

```toml
[risk]
# Position Limits
[risk.position_limits]
max_position_size_usd = 100000
max_position_percent_portfolio = 25
max_single_leg_percent = 50
min_position_size_usd = 100

# Daily Limits
[risk.daily_limits]
max_daily_loss_usd = 50000          # Stop trading if exceeded
max_daily_trades = 1000
max_daily_exposure_usd = 500000

# Account Limits
[risk.account_limits]
max_leverage = 1.0                  # No margin trading initially
max_open_positions = 50
concentration_limit_percent = 30    # Max % in single asset

# Order Limits
[risk.order_limits]
max_slippage_percent = 2.0
max_order_size_usd = 50000
require_order_simulation = true
```

**Parameters**:
- All limits are hard stops; cannot be overridden at runtime
- `max_daily_loss_usd`: Circuit breaker; stop trading if hit
- `max_leverage`: Position leverage limit
- `require_order_simulation`: Pre-flight check all orders

---

### 7. Execution Section

```toml
[execution]
# Execution method
execution_engine = "jito"           # jito, direct_rpc, hybrid
enable_partial_fills = true
allow_slippage_retry = true

# Position Sizing
[execution.position_sizing]
method = "kelly_fraction"           # fixed, kelly_fraction, volatility_based
kelly_fraction = 0.25               # Fractional Kelly for safety
min_position_size_usd = 100
max_position_size_usd = 100000

# Slippage Tolerance
[execution.slippage]
default_tolerance_percent = 1.5
aggressive_tolerance_percent = 3.0
conservative_tolerance_percent = 0.5
slippage_model = "empirical"        # empirical, analytical

# Retry Policy
[execution.retry]
max_attempts = 3
initial_backoff_ms = 100
max_backoff_ms = 5000
backoff_multiplier = 2.0
```

**Parameters**:
- `execution_engine`: Primary execution method
- `kelly_fraction`: Fraction of Kelly criterion for sizing
- `slippage_model`: How to estimate slippage

---

### 8. Storage Section

```toml
[storage]
# PostgreSQL
[storage.postgres]
host = "localhost"
port = 5432
database = "solstice"
username = "solstice_user"
password = "${DATABASE_PASSWORD}"  # From environment variable
pool_size = 20
connection_timeout_seconds = 10
query_timeout_seconds = 30
ssl_mode = "prefer"

# Data Retention
[storage.retention]
market_data_days = 365              # Keep 1 year of market data
trades_days = 730                   # Keep 2 years of trades
positions_days = 365
delete_job_interval_hours = 24

# Redis Cache
[storage.redis]
endpoints = ["localhost:6379"]
db = 0
password = ""                        # Empty for default
connection_pool_size = 10
cache_ttl_default_seconds = 300
connection_timeout_seconds = 5
```

**Parameters**:
- `pool_size`: Database connection pool size
- `cache_ttl_default_seconds`: Default cache expiration
- `retention`: How long to keep different data types

---

### 9. API Section

```toml
[api]
# HTTP Server
[api.http]
host = "0.0.0.0"
port = 8080
tls_enabled = false
cert_path = "/etc/solstice/cert.pem"
key_path = "/etc/solstice/key.pem"
request_timeout_seconds = 30

# Rate Limiting
[api.rate_limiting]
enabled = true
requests_per_second = 100
burst_size = 1000

# CORS
[api.cors]
allowed_origins = ["http://localhost:3000"]
allowed_methods = ["GET", "POST"]
max_age_seconds = 3600

# WebSocket
[api.websocket]
enabled = true
port = 8081
max_connections = 1000
message_queue_size = 10000
```

**Parameters**:
- `port`: HTTP server port
- `tls_enabled`: Enable TLS for HTTPS
- `requests_per_second`: Rate limit per client
- `max_connections`: Max concurrent WebSocket connections

---

### 10. Monitoring Section

```toml
[monitoring]
# Prometheus Metrics
[monitoring.prometheus]
enabled = true
port = 9090
scrape_interval_seconds = 15
metric_prefix = "solstice_"

# Log Output
[monitoring.logging]
level = "info"                      # trace, debug, info, warn, error
format = "json"                     # json, text
output = "stdout"                   # stdout, file, both
file_path = "/var/log/solstice/solstice.log"
max_file_size_mb = 100
max_backup_files = 10

# Alerting
[monitoring.alerts]
enabled = true
slack_webhook_url = "${SLACK_WEBHOOK_URL}"
email_recipients = ["ops@company.com"]
critical_email_recipients = ["oncall@company.com"]
alert_grace_period_seconds = 300
```

**Parameters**:
- `metric_prefix`: Prometheus metric prefix
- `log_level`: Logging verbosity
- `slack_webhook_url`: Slack integration for alerts
- `alert_grace_period_seconds`: Delay before alerting to avoid noise

---

### 11. Simulation Section

```toml
[simulation]
# Backtesting
[simulation.backtest]
default_start_date = "2025-01-01"
default_end_date = "2026-01-01"
initial_capital_usd = 100000
slippage_model = "empirical"
commissions_bps = 1.0              # Basis points
slippage_bps = 0.5

# Paper Trading
[simulation.paper_trading]
enabled = true
slippage_simulation = true
commission_simulation = true
latency_simulation_ms = 100
```

**Parameters**:
- `initial_capital_usd`: Starting capital for backtest
- `slippage_model`: How to model slippage
- `latency_simulation_ms`: Simulate network latency

---

## Environment Variables

Sensitive values and environment-specific settings use environment variables:

```bash
# Database
DATABASE_PASSWORD=...
DATABASE_URL=postgresql://...

# API Keys (if needed)
JITO_PRIVATE_KEY=...
RPC_API_KEY=...

# Monitoring
SLACK_WEBHOOK_URL=...
SENTRY_DSN=...

# Feature Flags
SOLSTICE_LOG_LEVEL=info
SOLSTICE_MODE=paper
SOLSTICE_ENVIRONMENT=staging
```

**Best Practices**:
- Never commit actual secrets to repository
- Use `.env.example` to show required variables
- Use separate `.env` files per environment
- Rotate secrets regularly

---

## Configuration Validation

### Schema Validation

A JSON Schema validates all configurations:

```toml
# config/schemas/config.schema.toml
[schema]
version = "1.0.0"

[schema.platform]
environment = { type = "string", enum = ["development", "staging", "production"] }
mode = { type = "string", enum = ["paper", "live", "backtest"] }
log_level = { type = "string", enum = ["trace", "debug", "info", "warn", "error"] }

[schema.market_data.yellowstone]
grpc_endpoints = { type = "array", items = { type = "string" } }
max_subscriptions = { type = "integer", minimum = 1, maximum = 1000 }

[schema.risk.daily_limits]
max_daily_loss_usd = { type = "number", minimum = 0 }
```

### Runtime Validation

```rust
// Pseudo-code in solstice-core
pub struct Config {
    pub platform: PlatformConfig,
    pub market_data: MarketDataConfig,
    // ... other sections
}

impl Config {
    pub fn validate(&self) -> Result<(), ValidationError> {
        // Validate individual sections
        self.platform.validate()?;
        self.market_data.validate()?;
        self.risk.validate()?;
        
        // Cross-section validation
        self.validate_consistency()?;
        
        Ok(())
    }
    
    fn validate_consistency(&self) -> Result<()> {
        // e.g., confirm max_position > min_position
        if self.risk.position_limits.max_position_size_usd 
            < self.risk.position_limits.min_position_size_usd {
            return Err(ValidationError::InconsistentLimits);
        }
        Ok(())
    }
}
```

---

## Configuration in Different Modes

### Development Mode

```toml
[platform]
environment = "development"
mode = "backtest"
log_level = "debug"

[market_data]
primary_feed = "rpc_polling"        # Faster to set up

[risk.daily_limits]
max_daily_loss_usd = 10000          # Conservative

[storage.postgres]
database = "solstice_dev"
```

### Paper Trading Mode

```toml
[platform]
environment = "staging"
mode = "paper"
log_level = "info"

[market_data]
primary_feed = "yellowstone"        # Use production feed

[risk.daily_limits]
max_daily_loss_usd = 50000          # Higher for testing

[simulation.paper_trading]
enabled = true
latency_simulation_ms = 100         # Realistic latency
```

### Production Mode

```toml
[platform]
environment = "production"
mode = "live"
log_level = "warn"                  # Less noise

[market_data]
primary_feed = "yellowstone"

[jito]
enabled = true                      # Use MEV protection

[risk.daily_limits]
max_daily_loss_usd = 200000         # Appropriate limit

[risk.account_limits]
max_open_positions = 20             # Conservative
```

---

## Runtime Parameter Adjustment

**Unsafe runtime adjustments** (require restart):
- Market data sources
- Blockchain endpoints
- Database connection
- Jito configuration

**Adjustable at runtime** (via API):
- Risk limits (tighter only, not looser)
- Strategy parameters
- API rate limits
- Logging level (within reason)

**Example API call** to adjust risk limit:

```bash
POST /api/v1/config/risk/daily_limits
{
    "max_daily_loss_usd": 30000    # Tighten limit
}
```

**Constraint**: Runtime adjustments can only make limits more conservative, never looser.

---

## Configuration Persistence & Versioning

### Configuration Versions

All configuration changes are versioned:

```rust
pub struct ConfigVersion {
    pub timestamp: DateTime<Utc>,
    pub version: String,
    pub hash: String,               // SHA256 of config
    pub changes: Vec<ConfigChange>,
}

pub struct ConfigChange {
    pub section: String,
    pub key: String,
    pub old_value: String,
    pub new_value: String,
    pub changed_by: String,         // Username or "API"
    pub reason: Option<String>,     // Optional change justification
}
```

### Audit Trail

All configuration modifications are logged:

```json
{
    "timestamp": "2026-07-20T14:30:00Z",
    "event": "config_change",
    "section": "risk.daily_limits",
    "change": {
        "key": "max_daily_loss_usd",
        "old_value": 50000,
        "new_value": 30000
    },
    "changed_by": "api_client_prod",
    "source": "/api/v1/config",
    "reason": "Increased volatility detected"
}
```

---

## Configuration Best Practices

1. **Use Descriptive Names**: Config keys should be self-documenting
2. **Validate Early**: Validate all configuration on startup
3. **Fail Fast**: Invalid config should crash platform immediately
4. **Comment Well**: Explain non-obvious settings with inline comments
5. **Environment Separation**: Keep dev/staging/prod configs separate
6. **Secret Management**: Use environment variables for secrets
7. **Version Control**: Commit config changes with code
8. **Audit Trail**: Log all configuration modifications
9. **Conservative Defaults**: Safe by default
10. **Document Changes**: Include config change rationale in commits

---

## Configuration Loading Flow

```
1. Load built-in defaults
    ↓
2. Load config/default.toml
    ↓
3. Load config/[environment].toml (if exists)
    ↓
4. Load config/local.toml (if exists)
    ↓
5. Override with environment variables
    ↓
6. Validate complete configuration
    ↓
7. Apply runtime parameter adjustments
    ↓
8. Log final configuration (sanitized)
    ↓
9. Start platform with validated config
```

---

## Related Documents

- [ARCHITECTURE.md](./ARCHITECTURE.md) - System architecture
- [WORKSPACE.md](./WORKSPACE.md) - Crate organization
- [DEPLOYMENT.md](./DEPLOYMENT.md) - Production deployment
- [OPERATIONAL_RUNBOOKS.md](./OPERATIONAL_RUNBOOKS.md) - Operational procedures
- [SECURITY.md](./SECURITY.md) - Security considerations

---

**Next**: [MARKET_DATA.md](./MARKET_DATA.md)
