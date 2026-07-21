// Mirrors solstice-api's DTOs (crates/solstice-api/src/dto.rs) and
// solstice-simulation's EngineEvent (crates/solstice-simulation/src/engine.rs).
// Kept in sync by hand for now; if the API grows an OpenAPI spec, generate
// these instead.

export interface WalletResponse {
  address: string;
  balance_lamports: number;
  balance_sol: number;
}

export interface StatusResponse {
  status: string;
  monitored_pairs: string[];
  open_positions: number;
  total_value_usd: number;
  circuit_breaker_tripped: boolean;
}

export interface PositionSnapshot {
  pair_label: string;
  base_mint: string;
  quote_mint: string;
  quantity: number;
  entry_price: number;
  current_price: number;
  unrealized_pnl: number;
}

export interface PositionsResponse {
  positions: PositionSnapshot[];
}

export interface PerformanceResponse {
  cash_usd: number;
  realized_pnl_usd: number;
  unrealized_pnl_usd: number;
  total_value_usd: number;
}

export type OrderStatus =
  | 'submitted'
  | 'partially_filled'
  | 'filled'
  | 'failed'
  | 'cancelled';

export interface TradeResponse {
  order_id: string;
  strategy: string;
  status: OrderStatus;
  size_usd: number;
  filled_amount: number;
  base_mint: string;
  quote_mint: string;
  approved: boolean;
  rejection_reason: string | null;
  failure_reason: string | null;
  created_at: string;
  updated_at: string;
}

export interface TradesResponse {
  trades: TradeResponse[];
}

export type EngineEvent =
  | {
      type: 'PriceUpdate';
      pair_label: string;
      dex: string;
      price: number;
      timestamp: string;
    }
  | {
      type: 'SignalGenerated';
      strategy: string;
      pair_label: string;
      confidence: number;
    }
  | {
      type: 'OrderFilled';
      order_id: string;
      strategy: string;
      pair_label: string;
      size_usd: number;
      price: number;
    }
  | {
      type: 'TickCompleted';
      timestamp: string;
      signal_count: number;
    };

export interface LivePositionSnapshot {
  pair_label: string;
  quantity_raw: number;
  entry_price: number;
  current_price: number;
  allocated_usd: number;
  unrealized_pnl_usd: number;
}

export interface LiveStatusResponse {
  enabled: boolean;
  wallet_address: string;
  max_capital_usd: number;
  min_confidence: number;
  capital_deployed_usd: number;
  capital_available_usd: number;
  realized_pnl_usd: number;
  positions: LivePositionSnapshot[];
}

export type LiveEvent =
  | { type: 'PriceUpdate'; pair_label: string; price: number; timestamp: string }
  | { type: 'SignalGenerated'; strategy: string; pair_label: string; confidence: number }
  | {
      type: 'WouldTrade';
      strategy: string;
      pair_label: string;
      size_usd: number;
      is_buy: boolean;
    }
  | { type: 'SignalSkipped'; strategy: string; pair_label: string; reason: string }
  | {
      type: 'OrderFilled';
      strategy: string;
      pair_label: string;
      size_usd: number;
      price: number;
      method: string;
      signature: string | null;
    }
  | { type: 'OrderFailed'; strategy: string; pair_label: string; reason: string }
  | { type: 'PositionClosed'; pair_label: string; realized_pnl_usd: number; reason: string }
  | { type: 'LiveTradingEnabled' }
  | { type: 'LiveTradingDisabled' }
  | { type: 'MaxCapitalChanged'; max_capital_usd: number }
  | { type: 'MinConfidenceChanged'; min_confidence: number }
  | { type: 'TickCompleted'; timestamp: string; signal_count: number };
