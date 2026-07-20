import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Database from "better-sqlite3";
import { drizzle } from "drizzle-orm/better-sqlite3";
import * as schema from "../src/db/schema.js";

// execution/autoSweep.ts moves REAL funds with no per-transfer confirmation once armed —
// this is exercised entirely against mocks (db, wallet balance lookup, sendTransfer,
// WS broadcast), never a real RPC connection or the real dev DB, same reasoning as every
// other wallet-touching test file in this suite.

const sqlite = new Database(":memory:");
sqlite.exec(`
  CREATE TABLE wallet_meta (id TEXT PRIMARY KEY, pubkey TEXT NOT NULL, created_at TEXT NOT NULL);
  CREATE TABLE app_mode (id TEXT PRIMARY KEY, trading_mode TEXT NOT NULL DEFAULT 'paper', network TEXT NOT NULL DEFAULT 'devnet', updated_at TEXT NOT NULL);
  CREATE TABLE auto_sweep_config (
    id TEXT PRIMARY KEY, enabled INTEGER NOT NULL DEFAULT 0, token_mint TEXT NOT NULL, token_symbol TEXT NOT NULL,
    threshold_amount REAL NOT NULL, destination TEXT NOT NULL, updated_at TEXT NOT NULL
  );
  CREATE TABLE wallet_sends (
    id TEXT PRIMARY KEY, token_mint TEXT NOT NULL, token_symbol TEXT NOT NULL, amount REAL NOT NULL,
    destination TEXT NOT NULL, network TEXT NOT NULL, tx_hash TEXT NOT NULL, confirmation_slot INTEGER, timestamp TEXT NOT NULL
  );
  INSERT INTO app_mode (id, trading_mode, network, updated_at) VALUES ('singleton', 'paper', 'devnet', '2000-01-01T00:00:00.000Z');
`);
const testDb = drizzle(sqlite, { schema });
vi.mock("../src/db/client.js", () => ({ db: testDb }));

const SOL_MINT = "So11111111111111111111111111111111111111";
const USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const DESTINATION = "11111111111111111111111111111111"; // valid base58, system program

const fetchWalletBalancesMock = vi.fn();
vi.mock("../src/wallet/walletBalance.js", () => ({ fetchWalletBalances: fetchWalletBalancesMock }));

const sendTransferMock = vi.fn();
vi.mock("../src/wallet/txBuilder.js", () => ({ sendTransfer: sendTransferMock, SOL_MINT }));

const broadcastMock = vi.fn();
vi.mock("../src/ws/hub.js", () => ({ broadcast: broadcastMock }));

const { checkAndRunAutoSweep } = await import("../src/execution/autoSweep.js");

function seedWallet(): void {
  sqlite.exec("DELETE FROM wallet_meta");
  sqlite.prepare("INSERT INTO wallet_meta (id, pubkey, created_at) VALUES ('singleton', ?, ?)").run(DESTINATION, new Date().toISOString());
}

function seedSweepConfig(overrides: Partial<{ enabled: boolean; tokenMint: string; tokenSymbol: string; thresholdAmount: number; destination: string }> = {}): void {
  const cfg = { enabled: true, tokenMint: USDC_MINT, tokenSymbol: "USDC", thresholdAmount: 100, destination: DESTINATION, ...overrides };
  sqlite.exec("DELETE FROM auto_sweep_config");
  sqlite
    .prepare("INSERT INTO auto_sweep_config (id, enabled, token_mint, token_symbol, threshold_amount, destination, updated_at) VALUES ('singleton', ?, ?, ?, ?, ?, ?)")
    .run(cfg.enabled ? 1 : 0, cfg.tokenMint, cfg.tokenSymbol, cfg.thresholdAmount, cfg.destination, new Date().toISOString());
}

// `lastCheckedAt` inside autoSweep.ts is module-private state that persists across every
// test in this file (the module is only imported once) — each test must start at a fresh
// system time more than CHECK_INTERVAL_MS past whatever the previous test left it at, or
// it'll look throttled. A monotonically-increasing base per test, far apart, guarantees
// that regardless of which earlier tests actually advanced the internal state.
let testTime = 0;

beforeEach(() => {
  vi.useFakeTimers();
  testTime += 10_000_000;
  vi.setSystemTime(testTime);
  fetchWalletBalancesMock.mockReset();
  sendTransferMock.mockReset();
  broadcastMock.mockReset();
  sqlite.exec("DELETE FROM auto_sweep_config; DELETE FROM wallet_meta; DELETE FROM wallet_sends;");
});

afterEach(() => {
  vi.useRealTimers();
});

describe("checkAndRunAutoSweep", () => {
  it("does nothing when disabled (the default)", async () => {
    seedWallet();
    seedSweepConfig({ enabled: false });
    await checkAndRunAutoSweep();
    expect(fetchWalletBalancesMock).not.toHaveBeenCalled();
    expect(sendTransferMock).not.toHaveBeenCalled();
  });

  it("does nothing when no hot wallet has been created yet", async () => {
    seedSweepConfig();
    await checkAndRunAutoSweep();
    expect(fetchWalletBalancesMock).not.toHaveBeenCalled();
    expect(sendTransferMock).not.toHaveBeenCalled();
  });

  it("does nothing when the balance is at or below the threshold", async () => {
    seedWallet();
    seedSweepConfig({ thresholdAmount: 100 });
    fetchWalletBalancesMock.mockResolvedValue({ solBalance: 1, tokenBalances: [{ mint: USDC_MINT, amount: 100 }] });

    await checkAndRunAutoSweep();
    expect(sendTransferMock).not.toHaveBeenCalled();
  });

  it("sweeps exactly the excess above threshold, persists it, and broadcasts it", async () => {
    seedWallet();
    seedSweepConfig({ thresholdAmount: 100, tokenMint: USDC_MINT, tokenSymbol: "USDC" });
    fetchWalletBalancesMock.mockResolvedValue({ solBalance: 1, tokenBalances: [{ mint: USDC_MINT, amount: 150 }] });
    sendTransferMock.mockResolvedValue({ txHash: "real-sweep-tx", confirmationSlot: 42 });

    await checkAndRunAutoSweep();

    expect(sendTransferMock).toHaveBeenCalledWith({ tokenMint: USDC_MINT, amount: 50, destination: DESTINATION, network: "devnet" });

    const rows = sqlite.prepare("SELECT * FROM wallet_sends").all() as { amount: number; tx_hash: string }[];
    expect(rows).toHaveLength(1);
    expect(rows[0]!.amount).toBe(50);
    expect(rows[0]!.tx_hash).toBe("real-sweep-tx");

    expect(broadcastMock).toHaveBeenCalledTimes(1);
    expect(broadcastMock.mock.calls[0]![0]).toMatchObject({ type: "wallet_send", data: { amount: 50, txHash: "real-sweep-tx", kind: "send" } });
  });

  it("sweeps native SOL correctly (uses solBalance, not tokenBalances)", async () => {
    seedWallet();
    seedSweepConfig({ thresholdAmount: 2, tokenMint: SOL_MINT, tokenSymbol: "SOL" });
    fetchWalletBalancesMock.mockResolvedValue({ solBalance: 5, tokenBalances: [] });
    sendTransferMock.mockResolvedValue({ txHash: "sol-sweep-tx", confirmationSlot: 1 });

    await checkAndRunAutoSweep();
    expect(sendTransferMock).toHaveBeenCalledWith({ tokenMint: SOL_MINT, amount: 3, destination: DESTINATION, network: "devnet" });
  });

  it("throttles repeated calls within the check interval", async () => {
    seedWallet();
    seedSweepConfig({ thresholdAmount: 100 });
    fetchWalletBalancesMock.mockResolvedValue({ solBalance: 1, tokenBalances: [{ mint: USDC_MINT, amount: 150 }] });
    sendTransferMock.mockResolvedValue({ txHash: "tx-1", confirmationSlot: 1 });

    await checkAndRunAutoSweep();
    expect(sendTransferMock).toHaveBeenCalledTimes(1);

    await checkAndRunAutoSweep(); // immediately again — should be throttled
    expect(sendTransferMock).toHaveBeenCalledTimes(1);

    vi.setSystemTime(testTime + 61_000); // past the 60s check interval
    await checkAndRunAutoSweep();
    expect(sendTransferMock).toHaveBeenCalledTimes(2);
  });

  it("does not throw when sendTransfer rejects (e.g. insufficient balance for fees) — fails closed, logs, moves on", async () => {
    seedWallet();
    seedSweepConfig({ thresholdAmount: 100 });
    fetchWalletBalancesMock.mockResolvedValue({ solBalance: 1, tokenBalances: [{ mint: USDC_MINT, amount: 150 }] });
    sendTransferMock.mockRejectedValue(new Error("Insufficient SOL for fees"));

    await expect(checkAndRunAutoSweep()).resolves.toBeUndefined();
    expect(broadcastMock).not.toHaveBeenCalled();
  });
});
