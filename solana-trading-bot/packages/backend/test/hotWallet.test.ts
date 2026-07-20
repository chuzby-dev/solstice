import { describe, expect, it, beforeEach, vi } from "vitest";
import Database from "better-sqlite3";
import { drizzle } from "drizzle-orm/better-sqlite3";
import * as schema from "../src/db/schema.js";
import { InMemorySecretStore } from "../src/wallet/secretStore.js";

// hotWallet.ts imports the real `db` singleton from db/client.ts, which opens the actual
// dev SQLite file (config.databasePath) as an import-time side effect — a test that
// exercised that unmocked would create/pollute wallet_meta rows in the real, currently-
// running dev database. Mocked here with an isolated :memory: instance holding just the
// one table this module touches, so these tests can never reach the real dev DB.
const sqlite = new Database(":memory:");
sqlite.exec("CREATE TABLE wallet_meta (id TEXT PRIMARY KEY, pubkey TEXT NOT NULL, created_at TEXT NOT NULL);");
const testDb = drizzle(sqlite, { schema });

vi.mock("../src/db/client.js", () => ({ db: testDb }));

const { generateHotWallet, hotWalletExists, getHotWalletPublicKey, getHotWalletCreatedAt, loadHotWalletKeypair, exportPrivateKeyBase58, HotWalletExistsError, HotWalletNotFoundError } =
  await import("../src/wallet/hotWallet.js");
const bs58 = (await import("bs58")).default;

beforeEach(() => {
  sqlite.exec("DELETE FROM wallet_meta");
});

describe("hotWallet — before creation", () => {
  it("reports no wallet exists", () => {
    expect(hotWalletExists()).toBe(false);
    expect(getHotWalletPublicKey()).toBeNull();
    expect(getHotWalletCreatedAt()).toBeNull();
  });

  it("throws HotWalletNotFoundError when loading with no wallet created", () => {
    const store = new InMemorySecretStore();
    expect(() => loadHotWalletKeypair(store)).toThrow(HotWalletNotFoundError);
  });
});

describe("hotWallet — generateHotWallet", () => {
  it("generates a wallet, stores the secret via the injected store, and records the pubkey", () => {
    const store = new InMemorySecretStore();
    const { pubkey } = generateHotWallet(store);

    expect(hotWalletExists()).toBe(true);
    expect(getHotWalletPublicKey()).toBe(pubkey);
    expect(getHotWalletCreatedAt()).not.toBeNull();

    const keypair = loadHotWalletKeypair(store);
    expect(keypair.publicKey.toBase58()).toBe(pubkey);
  });

  it("refuses to overwrite an existing wallet", () => {
    const store = new InMemorySecretStore();
    generateHotWallet(store);
    expect(() => generateHotWallet(store)).toThrow(HotWalletExistsError);
  });

  it("generates a different keypair each time (across separate wallets/tests)", () => {
    const storeA = new InMemorySecretStore();
    const { pubkey: pubkeyA } = generateHotWallet(storeA);
    sqlite.exec("DELETE FROM wallet_meta"); // simulate a fresh wallet slot
    const storeB = new InMemorySecretStore();
    const { pubkey: pubkeyB } = generateHotWallet(storeB);
    expect(pubkeyA).not.toBe(pubkeyB);
  });
});

describe("hotWallet — exportPrivateKeyBase58", () => {
  it("returns the exact secret key, base58-encoded (the format Phantom/Solflare's Import Private Key accepts)", () => {
    const store = new InMemorySecretStore();
    generateHotWallet(store);
    const keypair = loadHotWalletKeypair(store);

    const exported = exportPrivateKeyBase58(store);

    expect(bs58.decode(exported)).toEqual(keypair.secretKey);
  });

  it("throws HotWalletNotFoundError when no wallet has been created yet", () => {
    const store = new InMemorySecretStore();
    expect(() => exportPrivateKeyBase58(store)).toThrow(HotWalletNotFoundError);
  });
});

describe("hotWallet — fail-closed behavior", () => {
  it("throws HotWalletNotFoundError when the keychain entry is missing even though wallet_meta exists", () => {
    const store = new InMemorySecretStore();
    generateHotWallet(store);

    // A different store instance stands in for "the real keychain entry is inaccessible"
    // (e.g. a different OS user account) — wallet_meta says a wallet exists, but the
    // secret itself can't be retrieved. Must fail closed, not silently proceed unsigned.
    const inaccessibleStore = new InMemorySecretStore();
    expect(() => loadHotWalletKeypair(inaccessibleStore)).toThrow(HotWalletNotFoundError);
  });
});
