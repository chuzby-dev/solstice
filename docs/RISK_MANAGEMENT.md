# Solstice Risk Management Framework

**Purpose**: Define risk controls, position limits, and loss prevention mechanisms.

**Scope**: Hard risk limits, risk calculation, monitoring, and breach procedures.

**Status**: Draft  
**Version**: 1.0.0-draft

---

## Risk Management Philosophy

Solstice uses a **fail-safe approach** where:
1. **Risk limits are hard stops** - Cannot be overridden at runtime
2. **Conservative defaults** - Start with small positions, increase through demonstrated success
3. **Multiple layers** - Position limits, notional limits, loss limits, concentration limits
4. **Automatic circuit breakers** - Halt trading if limits approached

---

## Risk Limit Types

### 1. Position Limits

```rust
pub struct PositionLimits {
    pub max_single_position_usd: u64,      // Max size per position
    pub max_position_percent: f64,          // Max % of portfolio
    pub min_position_size_usd: u64,         // Min viable size
    pub max_open_positions: usize,          // Max concurrent positions
}

impl PositionLimits {
    pub fn can_open(&self, 
        portfolio_value: u64,
        new_position_usd: u64,
        current_positions: usize,
    ) -> Result<()> {
        if new_position_usd > self.max_single_position_usd {
            return Err("Exceeds max single position".into());
        }
        
        let pct = (new_position_usd as f64) / (portfolio_value as f64);
        if pct > self.max_position_percent {
            return Err("Exceeds max % of portfolio".into());
        }
        
        if current_positions >= self.max_open_positions {
            return Err("Max open positions exceeded".into());
        }
        
        Ok(())
    }
}
```

**Configuration**:
```toml
[risk.position_limits]
max_single_position_usd = 100000
max_position_percent = 25.0
min_position_size_usd = 1000
max_open_positions = 50
```

### 2. Daily Loss Limits

```rust
pub struct DailyLossLimits {
    pub max_daily_loss_usd: u64,           // Absolute loss limit
    pub max_daily_loss_percent: f64,       // Loss as % of portfolio
}

impl DailyLossLimits {
    pub fn check_loss(&self,
        daily_loss: i64,
        portfolio_value: u64,
    ) -> Result<()> {
        if daily_loss.abs() as u64 > self.max_daily_loss_usd {
            return Err("Daily loss limit exceeded".into());
        }
        
        let loss_pct = (daily_loss.abs() as f64) / (portfolio_value as f64);
        if loss_pct > self.max_daily_loss_percent {
            return Err("Daily loss % limit exceeded".into());
        }
        
        Ok(())
    }
}
```

**Action on Breach**: Immediately halt all trading and alert operators.

### 3. Exposure Limits

```rust
pub struct ExposureLimits {
    pub max_total_exposure_usd: u64,       // Max capital deployed
    pub max_leverage: f64,                 // Max leverage ratio (typically 1.0)
    pub max_per_asset: f64,                // Max % per single asset
    pub max_correlated: f64,               // Max % in correlated assets
}

impl ExposureLimits {
    pub fn can_increase_exposure(&self,
        current_exposure: u64,
        additional_exposure: u64,
    ) -> Result<()> {
        let total = current_exposure.saturating_add(additional_exposure);
        
        if total > self.max_total_exposure_usd {
            return Err("Would exceed total exposure".into());
        }
        
        Ok(())
    }
}
```

### 4. Concentration Limits

```rust
pub struct ConcentrationLimits {
    pub max_single_asset_percent: f64,     // Max % in single asset
    pub max_pair_percent: f64,             // Max % in single pair
}

impl ConcentrationLimits {
    pub fn check_concentration(&self,
        positions: &[Position],
        total_value: u64,
    ) -> Result<()> {
        for position in positions {
            let pct = (position.value_usd as f64) / (total_value as f64);
            
            if pct > self.max_single_asset_percent {
                return Err(format!(
                    "Asset {} exceeds concentration limit: {:.2}%",
                    position.asset, pct * 100.0
                ).into());
            }
        }
        
        Ok(())
    }
}
```

### 5. Order Limits

```rust
pub struct OrderLimits {
    pub max_order_size_usd: u64,           // Max per order
    pub max_slippage_percent: f64,         // Max acceptable slippage
    pub require_pre_flight_simulation: bool,  // Simulate before submit
}

impl OrderLimits {
    pub fn can_submit_order(&self,
        order_size: u64,
        simulated_slippage: f64,
    ) -> Result<()> {
        if order_size > self.max_order_size_usd {
            return Err("Order exceeds max size".into());
        }
        
        if simulated_slippage > self.max_slippage_percent {
            return Err(format!(
                "Slippage {:.2}% exceeds limit {:.2}%",
                simulated_slippage * 100.0,
                self.max_slippage_percent * 100.0
            ).into());
        }
        
        Ok(())
    }
}
```

---

## Risk Monitoring

### Real-Time Risk Metrics

```rust
pub struct PortfolioRiskMetrics {
    pub timestamp: DateTime<Utc>,
    
    // Position metrics
    pub total_positions: usize,
    pub total_exposure_usd: u64,
    pub concentration_highest: f64,
    
    // P&L metrics
    pub daily_pnl: i64,
    pub daily_loss: i64,
    pub unrealized_pnl: i64,
    pub realized_pnl: i64,
    
    // Risk metrics
    pub max_drawdown: f64,
    pub sharpe_ratio: f64,
    pub var_95: f64,  // Value at Risk, 95% confidence
    
    // Limit status
    pub limits_status: RiskLimitStatus,
}

pub enum RiskLimitStatus {
    Healthy,
    Warning { limit: String, usage: f64 },  // Usage as %
    Critical { limit: String },
    Breached { limit: String },
}

pub struct RiskMonitor {
    limits: Arc<RiskLimits>,
    metrics_history: Arc<RwLock<VecDeque<PortfolioRiskMetrics>>>,
}

impl RiskMonitor {
    pub async fn update(&self, portfolio: &Portfolio) -> Result<PortfolioRiskMetrics> {
        let metrics = PortfolioRiskMetrics {
            timestamp: Utc::now(),
            total_positions: portfolio.positions.len(),
            total_exposure_usd: portfolio.total_exposure_usd(),
            // ... other metrics
            limits_status: self.check_limits(portfolio).await?,
        };
        
        // Store for historical analysis
        self.metrics_history.write().await.push_back(metrics.clone());
        
        // Alert if approaching limits
        if matches!(metrics.limits_status, 
            RiskLimitStatus::Warning { .. } | RiskLimitStatus::Critical { .. }) {
            warn!("Risk limit approaching: {:?}", metrics.limits_status);
        }
        
        // Halt trading if limit breached
        if matches!(metrics.limits_status, RiskLimitStatus::Breached { .. }) {
            error!("RISK LIMIT BREACHED: {:?}", metrics.limits_status);
            self.trigger_circuit_breaker().await?;
        }
        
        Ok(metrics)
    }
    
    async fn check_limits(&self, portfolio: &Portfolio) -> Result<RiskLimitStatus> {
        let daily_loss = portfolio.daily_loss_usd();
        
        // Check daily loss limit (hardest limit)
        if daily_loss.abs() > self.limits.daily_loss_usd as i64 {
            return Ok(RiskLimitStatus::Breached {
                limit: "daily_loss".to_string(),
            });
        }
        
        // Check if approaching (warning at 80%)
        if (daily_loss.abs() as f64) > (self.limits.daily_loss_usd as f64 * 0.8) {
            return Ok(RiskLimitStatus::Warning {
                limit: "daily_loss".to_string(),
                usage: (daily_loss.abs() as f64) / (self.limits.daily_loss_usd as f64),
            });
        }
        
        Ok(RiskLimitStatus::Healthy)
    }
    
    async fn trigger_circuit_breaker(&self) -> Result<()> {
        error!("TRIGGERING CIRCUIT BREAKER");
        
        // Immediately halt all trading
        // Can be reset only via manual intervention
        
        // Alert operators
        // Send critical alerts to PagerDuty, Slack, etc.
        
        Ok(())
    }
}
```

---

## Stop Loss Management

### Automatic Stop Loss

```rust
pub struct StopLossManager {
    stop_loss_percent: f64,  // Default: 5%
}

impl StopLossManager {
    pub fn evaluate_stops(&self, portfolio: &Portfolio) -> Vec<OrderToExecute> {
        let mut orders = vec![];
        
        for position in &portfolio.positions {
            let loss_pct = (position.current_price.value - position.entry_price.value)
                / position.entry_price.value;
            
            if loss_pct < -self.stop_loss_percent {
                // Stop loss triggered
                orders.push(OrderToExecute {
                    position_id: position.id,
                    action: OrderAction::Exit,
                    reason: format!("Stop loss triggered: {:.2}% loss", loss_pct * 100.0),
                });
            }
        }
        
        orders
    }
}
```

**Configuration**:
```toml
[risk.stop_loss]
enabled = true
default_percent = 5.0          # Exit if down 5%
max_loss_before_halt = 10.0    # Halt trading if down 10%
```

---

## Pre-Trade Risk Checks

```rust
pub struct PreTradeRiskChecker {
    limits: RiskLimits,
}

impl PreTradeRiskChecker {
    pub async fn check_before_trade(&self,
        signal: &Signal,
        portfolio: &Portfolio,
        dex_aggregator: &DexAggregator,
    ) -> Result<TradeApproval> {
        // 1. Check position limits
        if !self.check_position_limits(signal, portfolio).await? {
            return Ok(TradeApproval::Rejected {
                reason: "Position limit exceeded".to_string(),
            });
        }
        
        // 2. Check exposure limits
        if !self.check_exposure_limits(signal, portfolio).await? {
            return Ok(TradeApproval::Rejected {
                reason: "Exposure limit exceeded".to_string(),
            });
        }
        
        // 3. Check concentration limits
        if !self.check_concentration_limits(signal, portfolio).await? {
            return Ok(TradeApproval::Rejected {
                reason: "Concentration limit exceeded".to_string(),
            });
        }
        
        // 4. Check slippage
        let quote = dex_aggregator.get_best_route(&signal.input_mint, &signal.output_mint).await?;
        if quote.slippage > self.limits.max_slippage_percent {
            return Ok(TradeApproval::Rejected {
                reason: format!("Slippage {:.2}% exceeds limit", quote.slippage * 100.0),
            });
        }
        
        // 5. Check daily loss limit
        if !self.check_daily_loss_limit(portfolio).await? {
            return Ok(TradeApproval::Rejected {
                reason: "Daily loss limit exceeded".to_string(),
            });
        }
        
        Ok(TradeApproval::Approved)
    }
}

pub enum TradeApproval {
    Approved,
    Rejected { reason: String },
}
```

---

## Risk Reporting

### Daily Risk Report

```rust
pub struct DailyRiskReport {
    pub date: Date,
    pub starting_value: u64,
    pub ending_value: u64,
    pub daily_pnl: i64,
    pub daily_loss: i64,
    pub return_percent: f64,
    
    pub positions_opened: usize,
    pub positions_closed: usize,
    pub avg_position_size: u64,
    pub max_position_size: u64,
    
    pub max_drawdown_daily: f64,
    pub sharpe_ratio: f64,
    pub win_rate: f64,
    pub avg_win: i64,
    pub avg_loss: i64,
    
    pub limit_breaches: Vec<String>,
    pub critical_alerts: Vec<String>,
}

impl DailyRiskReport {
    pub fn generate(
        metrics_history: &VecDeque<PortfolioRiskMetrics>,
        trades: &[Trade],
    ) -> Self {
        // Calculate all metrics from historical data
        // ...
    }
}
```

---

## Configuration

```toml
[risk]
# Global circuit breaker
circuit_breaker_enabled = true

# Position Limits
[risk.position_limits]
max_single_position_usd = 100000
max_position_percent = 25.0
min_position_size_usd = 1000
max_open_positions = 50

# Daily Loss Limits
[risk.daily_limits]
max_daily_loss_usd = 50000          # HARD STOP
max_daily_loss_percent = 5.0
max_daily_trades = 1000
warning_threshold_percent = 80      # Alert at 80%

# Exposure
[risk.exposure]
max_total_exposure_usd = 500000
max_leverage = 1.0
max_per_asset_percent = 30.0
max_correlated_percent = 50.0

# Concentration
[risk.concentration]
max_single_asset_percent = 30.0
max_pair_percent = 50.0

# Orders
[risk.orders]
max_order_size_usd = 50000
max_slippage_percent = 2.0
require_pre_flight_simulation = true
max_slippage_absolute_basis_points = 20

# Stop Loss
[risk.stop_loss]
enabled = true
default_percent = 5.0
trailing_stop_enabled = false
take_profit_enabled = false
```

---

## Testing

```rust
#[test]
fn test_position_limits() {
    let limits = PositionLimits {
        max_single_position_usd: 100_000,
        max_position_percent: 0.25,
        min_position_size_usd: 1000,
        max_open_positions: 50,
    };
    
    // Valid position
    assert!(limits.can_open(1_000_000, 50_000, 10).is_ok());
    
    // Exceeds single position
    assert!(limits.can_open(1_000_000, 150_000, 10).is_err());
    
    // Exceeds portfolio %
    assert!(limits.can_open(1_000_000, 300_000, 10).is_err());
}

#[tokio::test]
async fn test_daily_loss_limit() {
    let monitor = RiskMonitor::new(test_limits());
    let portfolio = create_test_portfolio_with_loss(30_000);  // $30k loss
    
    let metrics = monitor.update(&portfolio).await.unwrap();
    
    assert!(matches!(metrics.limits_status, RiskLimitStatus::Warning { .. }));
    
    // Increase loss beyond limit
    let portfolio = create_test_portfolio_with_loss(60_000);  // $60k loss
    let metrics = monitor.update(&portfolio).await.unwrap();
    
    assert!(matches!(metrics.limits_status, RiskLimitStatus::Breached { .. }));
}
```

---

## Related Documents

- [EXECUTION.md](./EXECUTION.md) - Pre-trade risk checks
- [POSITION_SIZING.md](./POSITION_SIZING.md) - Position size calculation
- [OPERATIONAL_RUNBOOKS.md](./OPERATIONAL_RUNBOOKS.md) - Response to limit breaches
- [CONFIGURATION.md](./CONFIGURATION.md) - Risk parameter configuration

---

**Complete**: Core strategy and risk management layer  
**Next Phase**: Execution engine and transaction building

Specification progress: 13/45 documents complete (29%)
