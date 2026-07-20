//! Integration tests for solstice-core.

use solana_sdk::pubkey::Pubkey;
use solstice_core::{
    types::{
        MarketEvent, OrderBook, Portfolio, Position, PositionId, Signal, SignalType, TokenPair,
        Trade, TradeAction,
    },
    Result, SolsticeError,
};

#[test]
fn test_core_types_compilation() {
    // This test verifies that all core types can be instantiated and used
    let base = Pubkey::new_unique();
    let quote = Pubkey::new_unique();
    let pair = TokenPair::new(base, quote);

    assert_eq!(pair.base, base);
    assert_eq!(pair.quote, quote);
}

#[test]
fn test_position_workflow() {
    let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());

    // Create position
    let mut position = Position::new(pair, 100, 100.0);
    assert_eq!(position.quantity, 100);
    assert_eq!(position.entry_price, 100.0);
    assert_eq!(position.unrealized_pnl(), 0.0);

    // Update price
    position.current_price = 110.0;
    assert_eq!(position.unrealized_pnl(), 1000.0);
    assert!(position.unrealized_pnl_percent() > 0.0);
}

#[test]
fn test_orderbook_validation() {
    let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());

    // Valid orderbook
    let valid_book = OrderBook::new(
        pair,
        vec![(100.0, 1000), (99.0, 2000)],
        vec![(101.0, 1000), (102.0, 2000)],
    );
    assert!(valid_book.is_valid());

    // Invalid: empty bids
    let invalid_book = OrderBook::new(pair, vec![], vec![(101.0, 1000)]);
    assert!(!invalid_book.is_valid());

    // Invalid: crossing (bid >= ask)
    let crossing_book = OrderBook::new(pair, vec![(101.0, 1000)], vec![(100.0, 1000)]);
    assert!(!crossing_book.is_valid());
}

#[test]
fn test_signal_confidence_validation() {
    let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());

    // Signal with confidence > 1.0 should be clamped
    let signal = Signal::new("test_strategy".to_string(), SignalType::Buy { pair }, 1.5);
    assert_eq!(signal.confidence, 1.0);

    // Signal with negative confidence should be clamped to 0.0
    let signal = Signal::new("test_strategy".to_string(), SignalType::Buy { pair }, -0.5);
    assert_eq!(signal.confidence, 0.0);
}

#[test]
fn test_trade_creation_and_fees() {
    let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());

    let trade = Trade::new(
        PositionId::new(),
        pair,
        TradeAction::Buy,
        10000, // 10,000 tokens
        100.0, // at $100 each
        25.0,  // $25 in fees
    );

    assert_eq!(trade.total_value(), 1_000_000.0);
    // Fees: 25 / 1,000,000 * 10,000 = 0.25 basis points
    assert!((trade.fees_bps() - 0.25).abs() < 0.01);
}

#[test]
fn test_portfolio_concentration() {
    let pair1 = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
    let pair2 = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());

    let mut pos1 = Position::new(pair1, 100, 100.0);
    pos1.current_price = 100.0;

    let mut pos2 = Position::new(pair2, 50, 200.0);
    pos2.current_price = 200.0;

    let portfolio = Portfolio {
        positions: vec![pos1, pos2],
        total_value: 20000.0, // 10000 + 10000
        available_capital: 0,
        timestamp: chrono::Utc::now(),
    };

    let concentration = portfolio.concentration();
    assert_eq!(concentration.len(), 2);

    // Each position should be 50% of portfolio
    for pct in concentration.values() {
        assert!((*pct - 0.5).abs() < 0.01);
    }
}

#[test]
fn test_serialization_roundtrip() {
    let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
    let position = Position::new(pair, 100, 100.0);

    // Serialize to JSON
    let json_str = serde_json::to_string(&position).expect("serialization failed");

    // Deserialize back
    let deserialized: Position = serde_json::from_str(&json_str).expect("deserialization failed");

    // Verify equality
    assert_eq!(position.id, deserialized.id);
    assert_eq!(position.pair, deserialized.pair);
    assert_eq!(position.quantity, deserialized.quantity);
    assert_eq!(position.entry_price, deserialized.entry_price);
}

#[test]
fn test_result_type() {
    let ok_result: Result<i32> = Ok(42);
    match ok_result {
        Ok(value) => assert_eq!(value, 42),
        Err(_) => panic!("expected Ok"),
    }

    let err_result: Result<i32> = Err(SolsticeError::ConfigError("test error".to_string()));
    assert!(err_result.is_err());
}

#[test]
fn test_market_event_serialization() {
    let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());

    let event = MarketEvent::PriceUpdate {
        token_pair: pair,
        price: 100.5,
        source: "test_source".to_string(),
        timestamp: chrono::Utc::now(),
    };

    // Should serialize without errors
    let json = serde_json::to_string(&event);
    assert!(json.is_ok());
}
