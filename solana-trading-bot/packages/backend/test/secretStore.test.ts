import { describe, expect, it } from "vitest";
import { InMemorySecretStore } from "../src/wallet/secretStore.js";

// Only the fake is unit-tested here — KeyringSecretStore is a thin wrapper around
// @napi-rs/keyring (an already-maintained, externally-tested library); re-testing it
// against the real OS keychain doesn't belong in CI. It's exercised for real during the
// manual Stage 1 verification (create a wallet via the UI, restart the server, confirm
// the pubkey and signing capability persist).

describe("InMemorySecretStore", () => {
  it("returns null for a secret that was never set", () => {
    const store = new InMemorySecretStore();
    expect(store.get("svc", "acct")).toBeNull();
  });

  it("round-trips a set secret", () => {
    const store = new InMemorySecretStore();
    store.set("svc", "acct", "sekrit");
    expect(store.get("svc", "acct")).toBe("sekrit");
  });

  it("isolates entries by service+account", () => {
    const store = new InMemorySecretStore();
    store.set("svc", "acct1", "a");
    store.set("svc", "acct2", "b");
    expect(store.get("svc", "acct1")).toBe("a");
    expect(store.get("svc", "acct2")).toBe("b");
  });

  it("overwrites an existing secret on a second set", () => {
    const store = new InMemorySecretStore();
    store.set("svc", "acct", "first");
    store.set("svc", "acct", "second");
    expect(store.get("svc", "acct")).toBe("second");
  });

  it("deletes a secret and reports success", () => {
    const store = new InMemorySecretStore();
    store.set("svc", "acct", "x");
    expect(store.delete("svc", "acct")).toBe(true);
    expect(store.get("svc", "acct")).toBeNull();
  });

  it("returns false deleting a secret that doesn't exist", () => {
    const store = new InMemorySecretStore();
    expect(store.delete("svc", "acct")).toBe(false);
  });
});
