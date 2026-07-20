import { describe, expect, it } from "vitest";
import { parseDestination, validateSolSend, validateSplSend, InvalidDestinationError, InsufficientBalanceError, SOL_MINT, explorerUrl } from "../src/wallet/txBuilder.js";

// Only the pure validation/parsing helpers are unit-tested here — building/signing/
// submitting a real transaction needs a live RPC connection and a funded wallet, which is
// exercised for real during the Stage 2 devnet verification (airdrop-fund, send, confirm
// on Solana Explorer), not in CI.

describe("parseDestination", () => {
  it("parses a valid base58 Solana address", () => {
    const pubkey = parseDestination("11111111111111111111111111111111");
    expect(pubkey.toBase58()).toBe("11111111111111111111111111111111");
  });

  it("throws InvalidDestinationError for garbage input", () => {
    expect(() => parseDestination("not-a-real-address")).toThrow(InvalidDestinationError);
  });

  it("throws InvalidDestinationError for an empty string", () => {
    expect(() => parseDestination("")).toThrow(InvalidDestinationError);
  });
});

describe("validateSolSend", () => {
  it("allows a send that leaves the reserve intact", () => {
    expect(() => validateSolSend(1.0, 0.5)).not.toThrow();
  });

  it("rejects a send that would breach the SOL reserve", () => {
    expect(() => validateSolSend(1.0, 0.995)).toThrow(InsufficientBalanceError); // reserve is 0.01 by default
  });

  it("rejects a zero or negative amount", () => {
    expect(() => validateSolSend(1.0, 0)).toThrow(InsufficientBalanceError);
    expect(() => validateSolSend(1.0, -1)).toThrow(InsufficientBalanceError);
  });

  it("rejects sending the entire balance (nothing left for the reserve)", () => {
    expect(() => validateSolSend(1.0, 1.0)).toThrow(InsufficientBalanceError);
  });
});

describe("validateSplSend", () => {
  it("allows a send within token balance with sufficient SOL for fees", () => {
    expect(() => validateSplSend(100, 50, 0.05)).not.toThrow();
  });

  it("rejects a send exceeding the token balance", () => {
    expect(() => validateSplSend(10, 50, 0.05)).toThrow(InsufficientBalanceError);
  });

  it("rejects when SOL balance can't cover fees, even with enough token balance", () => {
    expect(() => validateSplSend(100, 50, 0.001)).toThrow(InsufficientBalanceError);
  });

  it("rejects a zero or negative amount", () => {
    expect(() => validateSplSend(100, 0, 0.05)).toThrow(InsufficientBalanceError);
  });
});

describe("explorerUrl", () => {
  it("builds a devnet-cluster Solana Explorer link", () => {
    expect(explorerUrl("abc123")).toBe("https://explorer.solana.com/tx/abc123?cluster=devnet");
  });
});

describe("SOL_MINT", () => {
  it("matches the app's existing native-SOL constant used elsewhere (TOKEN_ALLOWLIST, priceFeed.ts)", () => {
    expect(SOL_MINT).toBe("So11111111111111111111111111111111111111");
  });
});
