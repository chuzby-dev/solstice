import { Keypair, Transaction, VersionedTransaction } from "@solana/web3.js";
import bs58 from "bs58";
import { eq } from "drizzle-orm";
import { db } from "../db/client.js";
import { walletMeta } from "../db/schema.js";
import { secretStore as defaultSecretStore, type SecretStore } from "./secretStore.js";

// The app's own dedicated, server-generated hot wallet — used for autonomous signing once
// live trading is enabled (see docs/ARCHITECTURE.md "Real hot wallet + live trading").
// Distinct from wallet/WalletProvider.tsx on the frontend, which is a read-only external
// wallet-adapter connection (Phantom/Solflare) — that code is untouched by this module.
//
// Custody: the private key is generated once, in-process, and immediately written to the
// OS keychain (wallet/secretStore.ts) — it is never imported from an existing wallet
// (so it never crosses the browser->backend boundary), never stored in this SQLite
// database, never logged, never returned over the wire by any route. `wallet_meta` holds
// only the public key and creation timestamp — non-secret pointer data.
//
// No key rotation/replacement/deletion flow exists yet (explicit non-goal for this pass) —
// generateHotWallet() refuses to overwrite an existing wallet rather than silently
// replacing it, since that would be one of the few genuinely catastrophic mistakes
// possible here (orphaning funds at the old address).

const KEYCHAIN_SERVICE = "solana-trading-bot";
const KEYCHAIN_ACCOUNT = "hot-wallet-secret-key";

export class HotWalletExistsError extends Error {
  constructor() {
    super("A hot wallet already exists for this app. Key rotation/replacement isn't supported in this phase.");
    this.name = "HotWalletExistsError";
  }
}

export class HotWalletNotFoundError extends Error {
  constructor() {
    super("No hot wallet has been created yet, or its OS keychain entry is inaccessible.");
    this.name = "HotWalletNotFoundError";
  }
}

function getMetaRow() {
  return db.select().from(walletMeta).where(eq(walletMeta.id, "singleton")).get();
}

export function hotWalletExists(): boolean {
  return !!getMetaRow();
}

/** Non-secret: reads only the DB pointer, never touches the keychain. */
export function getHotWalletPublicKey(): string | null {
  return getMetaRow()?.pubkey ?? null;
}

export function getHotWalletCreatedAt(): string | null {
  return getMetaRow()?.createdAt ?? null;
}

/** Generates a brand-new keypair, stores its secret key in the OS keychain, and records
 * the (non-secret) public key + creation time. Refuses to run if a wallet already exists —
 * see class doc. `store` is injectable so tests use `InMemorySecretStore` instead of the
 * real OS keychain. */
export function generateHotWallet(store: SecretStore = defaultSecretStore): { pubkey: string } {
  if (hotWalletExists()) throw new HotWalletExistsError();

  const keypair = Keypair.generate();
  const secretBase64 = Buffer.from(keypair.secretKey).toString("base64");
  store.set(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, secretBase64);

  const pubkey = keypair.publicKey.toBase58();
  const createdAt = new Date().toISOString();
  db.insert(walletMeta).values({ id: "singleton", pubkey, createdAt }).run();

  return { pubkey };
}

/** Loads the full keypair (including the private key) from the OS keychain. Callers must
 * never log, serialize, or transmit the returned `Keypair` — only ever use it to sign a
 * transaction locally. Throws `HotWalletNotFoundError` if no wallet exists or the keychain
 * entry is missing/inaccessible (e.g. a different OS user account) — callers on the live
 * execution path must treat this as fail-closed (pause live trading), never fall back to
 * paper mode silently. */
export function loadHotWalletKeypair(store: SecretStore = defaultSecretStore): Keypair {
  if (!hotWalletExists()) throw new HotWalletNotFoundError();
  const secretBase64 = store.get(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT);
  if (!secretBase64) throw new HotWalletNotFoundError();
  const secretKey = new Uint8Array(Buffer.from(secretBase64, "base64"));
  return Keypair.fromSecretKey(secretKey);
}

/** Signs a legacy `Transaction` in place (used by the manual send/transfer path). */
export function signLegacyTransaction(tx: Transaction, store: SecretStore = defaultSecretStore): void {
  tx.partialSign(loadHotWalletKeypair(store));
}

/** Signs a `VersionedTransaction` in place (used by the Jupiter swap path, which returns
 * versioned transactions). */
export function signVersionedTransaction(tx: VersionedTransaction, store: SecretStore = defaultSecretStore): void {
  tx.sign([loadHotWalletKeypair(store)]);
}

/** THE ONE DELIBERATE EXCEPTION to every other function in this module's rule ("never log,
 * serialize, or transmit the key — only ever use it to sign locally"). Exists solely
 * because the user explicitly asked to be able to import this exact wallet into another
 * app (Phantom/Solflare's "Import Private Key" accepts base58-encoded secret key bytes,
 * the format returned here). There is no BIP39 recovery phrase to export alongside it —
 * this wallet was created via `Keypair.generate()` (see generateHotWallet's doc: a fresh
 * random keypair, never derived from a mnemonic), so a raw private key is the only valid
 * export format.
 *
 * Callers MUST NOT log this return value under any circumstance (not even at debug level,
 * not in an error message, not in a stack trace) — see routes/wallet.ts's export route,
 * which is the only caller and is itself gated behind an explicit user confirmation. */
export function exportPrivateKeyBase58(store: SecretStore = defaultSecretStore): string {
  const keypair = loadHotWalletKeypair(store);
  return bs58.encode(keypair.secretKey);
}
