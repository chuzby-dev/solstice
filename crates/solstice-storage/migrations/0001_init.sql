-- Initial Solstice storage schema.
--
-- Requires the TimescaleDB extension for hypertable support. If the
-- extension is unavailable, the tables are created as ordinary Postgres
-- tables and `create_hypertable` is skipped (see the DO block at the end).

CREATE EXTENSION IF NOT EXISTS timescaledb;

-- Time-series price observations, one row per (time, base, quote) sample.
CREATE TABLE IF NOT EXISTS market_snapshots (
    time        TIMESTAMPTZ      NOT NULL,
    base_mint   TEXT             NOT NULL,
    quote_mint  TEXT             NOT NULL,
    price       DOUBLE PRECISION NOT NULL,
    confidence  DOUBLE PRECISION NOT NULL,
    source      TEXT             NOT NULL,
    PRIMARY KEY (time, base_mint, quote_mint, source)
);

CREATE INDEX IF NOT EXISTS idx_market_snapshots_pair_time
    ON market_snapshots (base_mint, quote_mint, time DESC);

-- Executed trades. `id` is the application-generated trade id (UUID string).
CREATE TABLE IF NOT EXISTS trades (
    id               TEXT             PRIMARY KEY,
    position_id      UUID             NOT NULL,
    base_mint        TEXT             NOT NULL,
    quote_mint       TEXT             NOT NULL,
    action           TEXT             NOT NULL CHECK (action IN ('buy', 'sell')),
    quantity         BIGINT           NOT NULL CHECK (quantity >= 0),
    execution_price  DOUBLE PRECISION NOT NULL,
    fees             DOUBLE PRECISION NOT NULL,
    executed_at      TIMESTAMPTZ      NOT NULL,
    tx_signature     TEXT
);

CREATE INDEX IF NOT EXISTS idx_trades_position_id ON trades (position_id);
CREATE INDEX IF NOT EXISTS idx_trades_executed_at ON trades (executed_at DESC);

-- Position state history: one row per recorded update to a position.
CREATE TABLE IF NOT EXISTS position_updates (
    id             BIGSERIAL        PRIMARY KEY,
    position_id    UUID             NOT NULL,
    base_mint      TEXT             NOT NULL,
    quote_mint     TEXT             NOT NULL,
    quantity       BIGINT           NOT NULL,
    entry_price    DOUBLE PRECISION NOT NULL,
    current_price  DOUBLE PRECISION NOT NULL,
    opened_at      TIMESTAMPTZ      NOT NULL,
    close_at       TIMESTAMPTZ,
    recorded_at    TIMESTAMPTZ      NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_position_updates_position_id
    ON position_updates (position_id, recorded_at DESC);

-- Raw on-chain account state snapshots (from the Yellowstone adapter),
-- retained for a limited window per REDIS_ARCHITECTURE-style TTL policy at
-- the application layer; the table itself has no built-in expiry.
CREATE TABLE IF NOT EXISTS account_snapshots (
    time      TIMESTAMPTZ NOT NULL,
    address   TEXT        NOT NULL,
    owner     TEXT        NOT NULL,
    lamports  BIGINT      NOT NULL,
    data      BYTEA       NOT NULL,
    slot      BIGINT      NOT NULL,
    PRIMARY KEY (time, address)
);

CREATE INDEX IF NOT EXISTS idx_account_snapshots_address_time
    ON account_snapshots (address, time DESC);

-- Convert the time-series tables to hypertables when TimescaleDB is
-- available. `if_not_exists` makes this idempotent across re-runs.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'timescaledb') THEN
        PERFORM create_hypertable('market_snapshots', 'time', if_not_exists => TRUE);
        PERFORM create_hypertable('account_snapshots', 'time', if_not_exists => TRUE);
    END IF;
END $$;
