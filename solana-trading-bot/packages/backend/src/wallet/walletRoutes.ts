import { randomUUID } from "node:crypto";
import type { FastifyInstance } from "fastify";
import { PublicKey } from "@solana/web3.js";
import { desc, eq } from "drizzle-orm";
import type { AutoSweepConfig, HotWalletKeyExport, HotWalletStatus, Network, TransferPreview, WalletSendResult, WalletTransaction } from "@trading-bot/shared";
import { config, walletSend } from "../config.js";
import { db } from "../db/client.js";
import { autoSweepConfig, trades, walletSends } from "../db/schema.js";
import { getAppMode } from "../execution/appMode.js";
import { priceCache } from "../market/priceCache.js";
import { symbolFor } from "../market/priceFeed.js";
import { broadcast } from "../ws/hub.js";
import { exportPrivateKeyBase58, generateHotWallet, getHotWalletCreatedAt, getHotWalletPublicKey, hotWalletExists, HotWalletExistsError, HotWalletNotFoundError } from "./hotWallet.js";
import { sendTransfer, previewTransfer, explorerUrl, parseDestination, InvalidDestinationError, InsufficientBalanceError } from "./txBuilder.js";
import { fetchWalletBalances } from "./walletBalance.js";

// This file has two distinct halves:
//  - GET /api/wallet/:pubkey/balance — read-only lookups (getBalance /
//    getParsedTokenAccountsByOwner) against an EXTERNALLY-SUPPLIED pubkey, for the
//    frontend's view-only wallet-adapter connect (Phantom/Solflare — see
//    wallet/WalletProvider.tsx). No keypair, no signing, always devnet regardless of the
//    bot's own trading mode — that connect flow is a separate, unrelated feature.
//  - /api/wallet/hot/* — the app's OWN server-custodied hot wallet (see hotWallet.ts,
//    txBuilder.ts). create/status/send/send-preview/balance/history — real signing, all
//    network-aware (reach whichever cluster app_mode.network currently selects). `send`
//    used to be silently hardcoded to devnet regardless of the selected network — a real
//    mainnet send would land on devnet instead, moving nothing on the network the user
//    actually intended. Fixed: every route here now reads getAppMode().network.

interface SendBody {
  tokenMint: string;
  amount: number;
  destination: string;
  /** Required (and must be true) once the send's estimated USD value exceeds
   * walletSend.maxWithoutExtraConfirmUsd — the frontend's tiered-confirmation UI sets
   * this only after the user has seen and acknowledged the "this is unusually large"
   * step. Enforced here too (not just client-side) since this route will eventually
   * carry real mainnet value unchanged. */
  acknowledgedLargeSend?: boolean;
}

export async function walletRoutes(app: FastifyInstance): Promise<void> {
  app.get("/api/wallet/hot/status", async () => {
    const status: HotWalletStatus = {
      exists: hotWalletExists(),
      pubkey: getHotWalletPublicKey(),
      createdAt: getHotWalletCreatedAt(),
    };
    return status;
  });

  app.post("/api/wallet/hot/create", async (req, reply) => {
    try {
      const { pubkey } = generateHotWallet();
      const status: HotWalletStatus = { exists: true, pubkey, createdAt: getHotWalletCreatedAt() };
      return reply.status(201).send(status);
    } catch (err) {
      if (err instanceof HotWalletExistsError) {
        return reply.status(409).send({ error: err.message });
      }
      req.log.error(err, "hot wallet creation failed");
      return reply.status(500).send({ error: "Failed to create hot wallet" });
    }
  });

  // The one route in this file that ever returns the raw private key — see
  // hotWallet.ts's exportPrivateKeyBase58 doc for why this exists at all. `confirmed`
  // mirrors strategies' activate-confirmation pattern; the frontend's own gate (typed
  // phrase + acknowledgement, see components/Wallet.tsx) is the real friction, this is
  // the server-side backstop against an accidental/automated call reaching this route.
  // Never logged: Fastify's request logger records method/url/status, never response
  // bodies, and nothing in this handler calls console.log on the result.
  app.post<{ Body: { confirmed: boolean } }>("/api/wallet/hot/export-key", async (req, reply) => {
    if (!req.body?.confirmed) {
      return reply.status(400).send({ error: "Export must be explicitly confirmed" });
    }
    try {
      const pubkey = getHotWalletPublicKey();
      const privateKeyBase58 = exportPrivateKeyBase58();
      return reply.send({ pubkey, privateKeyBase58 } satisfies HotWalletKeyExport);
    } catch (err) {
      if (err instanceof HotWalletNotFoundError) {
        return reply.status(400).send({ error: err.message });
      }
      req.log.error({ err: err instanceof Error ? err.message : "unknown" }, "wallet key export failed");
      return reply.status(500).send({ error: "Failed to export private key" });
    }
  });

  // Real fee quote + real dry-run simulation for the exact transaction a subsequent
  // POST /api/wallet/send would submit — never signs, never broadcasts (see
  // txBuilder.ts's previewTransfer doc). This is what lets the frontend show a
  // Solflare/Phantom-style review step (estimated fee, "this would fail" warning) before
  // the user commits to an irreversible send.
  app.post<{ Body: Pick<SendBody, "tokenMint" | "amount" | "destination"> }>("/api/wallet/send/preview", async (req, reply) => {
    const { tokenMint, amount, destination } = req.body;

    if (!hotWalletExists()) {
      return reply.status(400).send({ error: "No hot wallet has been created yet" });
    }
    if (!config.tokenAllowlist.includes(tokenMint)) {
      return reply.status(400).send({ error: `Token ${tokenMint} is not on the allowlist` });
    }
    if (!(amount > 0)) {
      return reply.status(400).send({ error: "Amount must be greater than zero" });
    }

    try {
      const { network } = getAppMode();
      const preview = await previewTransfer({ tokenMint, amount, destination, network });
      return reply.send(preview satisfies TransferPreview);
    } catch (err) {
      if (err instanceof InvalidDestinationError || err instanceof InsufficientBalanceError) {
        return reply.status(400).send({ error: err.message });
      }
      req.log.error(err, "wallet send preview failed");
      return reply.status(500).send({ error: err instanceof Error ? err.message : "Preview failed" });
    }
  });

  app.post<{ Body: SendBody }>("/api/wallet/send", async (req, reply) => {
    const { tokenMint, amount, destination, acknowledgedLargeSend } = req.body;

    if (!hotWalletExists()) {
      return reply.status(400).send({ error: "No hot wallet has been created yet" });
    }
    if (!config.tokenAllowlist.includes(tokenMint)) {
      return reply.status(400).send({ error: `Token ${tokenMint} is not on the allowlist` });
    }
    if (!(amount > 0)) {
      return reply.status(400).send({ error: "Amount must be greater than zero" });
    }

    const priceUsd = priceCache.latest(tokenMint)?.priceUsd ?? null;
    const usdValue = priceUsd !== null ? amount * priceUsd : null;
    if (usdValue !== null && usdValue > walletSend.maxWithoutExtraConfirmUsd && !acknowledgedLargeSend) {
      return reply.status(400).send({
        error: `This send is ~$${usdValue.toFixed(2)}, above the $${walletSend.maxWithoutExtraConfirmUsd} threshold that needs extra confirmation`,
        requiresAcknowledgement: true,
        usdValue,
      });
    }

    const { network } = getAppMode();
    try {
      const result = await sendTransfer({ tokenMint, amount, destination, network });

      // Manual sends are never a Signal, so they don't belong in `trades` — this is their
      // only persistence, and GET /api/wallet/history reads it back merged with live
      // trades for one combined view (see db/schema.ts's walletSends comment).
      const record: WalletTransaction = {
        id: randomUUID(),
        kind: "send",
        tokenSymbol: symbolFor(tokenMint),
        amount,
        network,
        txHash: result.txHash,
        confirmationSlot: result.confirmationSlot,
        timestamp: new Date().toISOString(),
        destination,
      };
      db.insert(walletSends)
        .values({ id: record.id, tokenMint, tokenSymbol: record.tokenSymbol, amount, destination, network: record.network, txHash: result.txHash, confirmationSlot: result.confirmationSlot, timestamp: record.timestamp })
        .run();
      broadcast({ type: "wallet_send", data: record });

      const response: WalletSendResult = { ...result, explorerUrl: explorerUrl(result.txHash, network) };
      return reply.send(response);
    } catch (err) {
      if (err instanceof InvalidDestinationError || err instanceof InsufficientBalanceError) {
        return reply.status(400).send({ error: err.message });
      }
      req.log.error(err, "wallet send failed");
      return reply.status(500).send({ error: err instanceof Error ? err.message : "Send failed" });
    }
  });

  // The hot wallet's OWN balance, on whichever network is currently selected (see
  // execution/appMode.ts) — distinct from the generic :pubkey route above, which is
  // always devnet for the unrelated external wallet-adapter connect feature.
  app.get("/api/wallet/hot/balance", async (req, reply) => {
    const pubkey = getHotWalletPublicKey();
    if (!pubkey) return reply.status(400).send({ error: "No hot wallet has been created yet" });

    const { network } = getAppMode();
    try {
      const { solBalance, tokenBalances } = await fetchWalletBalances(network, new PublicKey(pubkey));
      return reply.send({ pubkey, network, solBalance, tokenBalances });
    } catch (err) {
      req.log.error(err, "hot wallet balance lookup failed");
      return reply.status(502).send({ error: `Failed to reach ${network} RPC` });
    }
  });

  // Standing auto-sweep rule (see db/schema.ts's autoSweepConfig comment) — off by
  // default, checked and possibly fired once per interval from
  // execution/autoSweep.ts, wired into the engine tick loop.
  app.get("/api/wallet/auto-sweep", async (): Promise<AutoSweepConfig> => {
    const row = db.select().from(autoSweepConfig).where(eq(autoSweepConfig.id, "singleton")).get();
    return row
      ? { enabled: row.enabled, tokenMint: row.tokenMint, tokenSymbol: row.tokenSymbol, thresholdAmount: row.thresholdAmount, destination: row.destination }
      : { enabled: false, tokenMint: config.tokenAllowlist[0] ?? "", tokenSymbol: symbolFor(config.tokenAllowlist[0] ?? ""), thresholdAmount: 0, destination: "" };
  });

  app.put<{ Body: AutoSweepConfig }>("/api/wallet/auto-sweep", async (req, reply) => {
    const { enabled, tokenMint, thresholdAmount, destination } = req.body;

    if (!config.tokenAllowlist.includes(tokenMint)) {
      return reply.status(400).send({ error: `Token ${tokenMint} is not on the allowlist` });
    }
    if (!(thresholdAmount >= 0)) {
      return reply.status(400).send({ error: "Threshold must be zero or a positive number" });
    }
    if (enabled) {
      try {
        parseDestination(destination);
      } catch {
        return reply.status(400).send({ error: `"${destination}" is not a valid Solana address` });
      }
    }

    const tokenSymbol = symbolFor(tokenMint);
    const updatedAt = new Date().toISOString();
    db.insert(autoSweepConfig)
      .values({ id: "singleton", enabled, tokenMint, tokenSymbol, thresholdAmount, destination, updatedAt })
      .onConflictDoUpdate({ target: autoSweepConfig.id, set: { enabled, tokenMint, tokenSymbol, thresholdAmount, destination, updatedAt } })
      .run();

    return reply.send({ enabled, tokenMint, tokenSymbol, thresholdAmount, destination } satisfies AutoSweepConfig);
  });

  // Merged, chronological view of every real transaction this wallet has signed: manual
  // sends (wallet_sends) and live strategy trades (trades where simulated=false) — see
  // shared/src/types.ts's WalletTransaction doc.
  app.get("/api/wallet/history", async (req, reply) => {
    const sendRows = db.select().from(walletSends).orderBy(desc(walletSends.timestamp)).limit(50).all();
    const tradeRows = db
      .select()
      .from(trades)
      .where(eq(trades.simulated, false))
      .orderBy(desc(trades.timestamp))
      .limit(50)
      .all()
      .filter((t) => t.txHash !== null);

    const history: WalletTransaction[] = [
      ...sendRows.map(
        (s): WalletTransaction => ({
          id: s.id,
          kind: "send",
          tokenSymbol: s.tokenSymbol,
          amount: s.amount,
          network: s.network as Network,
          txHash: s.txHash,
          confirmationSlot: s.confirmationSlot,
          timestamp: s.timestamp,
          destination: s.destination,
        }),
      ),
      ...tradeRows.map(
        (t): WalletTransaction => ({
          id: t.id,
          kind: "trade",
          tokenSymbol: t.tokenSymbol,
          amount: t.sizeToken,
          network: (t.network ?? "mainnet") as Network,
          txHash: t.txHash!,
          confirmationSlot: t.confirmationSlot,
          timestamp: t.timestamp,
          action: t.action as WalletTransaction["action"],
          strategyId: t.strategyId as WalletTransaction["strategyId"],
        }),
      ),
    ].sort((a, b) => b.timestamp.localeCompare(a.timestamp));

    return reply.send(history.slice(0, 50));
  });

  app.get<{ Params: { pubkey: string } }>("/api/wallet/:pubkey/balance", async (req, reply) => {
    let owner: PublicKey;
    try {
      owner = new PublicKey(req.params.pubkey);
    } catch {
      return reply.status(400).send({ error: "Invalid public key" });
    }

    try {
      const { solBalance, tokenBalances } = await fetchWalletBalances("devnet", owner);
      return reply.send({ pubkey: owner.toBase58(), network: "devnet", solBalance, tokenBalances });
    } catch (err) {
      req.log.error(err, "wallet balance lookup failed");
      return reply.status(502).send({ error: "Failed to reach devnet RPC" });
    }
  });
}
