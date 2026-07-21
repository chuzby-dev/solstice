//! Runnable backtest CLI: replay a historical CSV price series through the
//! SMA crossover strategy and print a performance report.
//!
//! ```sh
//! cargo run -p solstice-simulation --bin backtest -- data.csv
//! cargo run -p solstice-simulation --bin backtest -- data.csv --short 5 --long 20 --capital 50000 --out report.json
//! ```
//!
//! CSV format: two columns, `timestamp,price` (RFC3339 timestamps), e.g.:
//! ```csv
//! timestamp,price
//! 2026-01-01T00:00:00Z,98.42
//! 2026-01-01T00:01:00Z,98.55
//! ```

use solana_sdk::pubkey::Pubkey;
use solstice_core::types::TokenPair;
use solstice_execution::risk::{
    ConcentrationLimits, DailyLossLimits, ExposureLimits, OrderLimits, PositionLimits, RiskLimits,
};
use solstice_simulation::backtest::{
    load_csv, BacktestConfig, BacktestEngine, FeeModel, FillModel, PartialFillConfig, SlippageModel,
};
use solstice_strategy::strategies::sma::SimpleMovingAverageStrategy;
use solstice_strategy::{StrategyConfig, StrategyManager};
use std::path::PathBuf;
use std::sync::Arc;

struct Args {
    csv_path: PathBuf,
    short_period: usize,
    long_period: usize,
    initial_capital_usd: f64,
    out_json: Option<PathBuf>,
}

fn print_usage_and_exit() -> ! {
    eprintln!(
        "usage: backtest <csv-path> [--short N] [--long N] [--capital USD] [--out report.json]"
    );
    std::process::exit(1);
}

fn parse_args() -> Args {
    let mut args = std::env::args().skip(1);
    let Some(csv_path) = args.next().map(PathBuf::from) else {
        print_usage_and_exit();
    };

    let mut short_period = 5;
    let mut long_period = 20;
    let mut initial_capital_usd = 10_000.0;
    let mut out_json = None;

    let rest: Vec<String> = args.collect();
    let mut i = 0;
    while i < rest.len() {
        let value = || rest.get(i + 1).unwrap_or_else(|| print_usage_and_exit());
        match rest[i].as_str() {
            "--short" => {
                short_period = value().parse().unwrap_or_else(|_| print_usage_and_exit());
                i += 2;
            }
            "--long" => {
                long_period = value().parse().unwrap_or_else(|_| print_usage_and_exit());
                i += 2;
            }
            "--capital" => {
                initial_capital_usd = value().parse().unwrap_or_else(|_| print_usage_and_exit());
                i += 2;
            }
            "--out" => {
                out_json = Some(PathBuf::from(value()));
                i += 2;
            }
            _ => print_usage_and_exit(),
        }
    }

    Args {
        csv_path,
        short_period,
        long_period,
        initial_capital_usd,
        out_json,
    }
}

fn risk_limits_for(initial_capital_usd: f64) -> RiskLimits {
    RiskLimits {
        position: PositionLimits {
            max_single_position_usd: (initial_capital_usd * 0.5) as u64,
            max_position_percent: 0.5,
            min_position_size_usd: 10,
            max_open_positions: 10,
        },
        daily_loss: DailyLossLimits {
            max_daily_loss_usd: (initial_capital_usd * 0.5) as u64,
            max_daily_loss_percent: 0.5,
        },
        exposure: ExposureLimits {
            max_total_exposure_usd: (initial_capital_usd * 2.0) as u64,
            max_leverage: 2.0,
        },
        concentration: ConcentrationLimits {
            max_single_asset_percent: 1.0,
        },
        order: OrderLimits {
            max_order_size_usd: (initial_capital_usd * 0.5) as u64,
            max_slippage_percent: 0.05,
        },
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = parse_args();
    // The CSV format carries no mint addresses, so the pair is a synthetic
    // placeholder — the report's numbers are what matter, not the label.
    let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());

    let ticks = load_csv(&args.csv_path, pair).unwrap_or_else(|e| {
        eprintln!("failed to load {}: {e}", args.csv_path.display());
        std::process::exit(1);
    });
    println!(
        "Loaded {} ticks from {} ({} to {})",
        ticks.len(),
        args.csv_path.display(),
        ticks
            .first()
            .expect("load_csv rejects empty files")
            .timestamp,
        ticks
            .last()
            .expect("load_csv rejects empty files")
            .timestamp
    );

    let strategy_manager = Arc::new(StrategyManager::new(StrategyConfig::default()));
    strategy_manager
        .register_strategy(Arc::new(SimpleMovingAverageStrategy::new(
            pair,
            args.short_period,
            args.long_period,
        )))
        .await
        .expect("failed to register SMA strategy");

    let config = BacktestConfig {
        initial_capital_usd: args.initial_capital_usd,
        risk_limits: risk_limits_for(args.initial_capital_usd),
        kelly_fraction: 0.5,
        default_win_loss_ratio: 2.0,
        stop_loss_percent: 0.1,
        fill_model: FillModel {
            slippage: SlippageModel::FixedBps(10.0),
            fee: FeeModel { bps: 25.0 },
            partial_fill: PartialFillConfig::unlimited(),
        },
    };

    let engine = BacktestEngine::new(strategy_manager, config);
    let report = engine.run(&ticks).await.expect("backtest run failed");

    println!("\n{}", report.to_markdown());

    if let Some(out_path) = args.out_json {
        let json = report.to_json_pretty().expect("report serializes to JSON");
        std::fs::write(&out_path, json).expect("failed to write report JSON");
        println!("Full report written to {}", out_path.display());
    }
}
