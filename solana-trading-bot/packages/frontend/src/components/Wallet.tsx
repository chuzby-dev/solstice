import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { AutoSweepConfig, BacktestVerdict, HotWalletKeyExport, TradingMode } from "@trading-bot/shared";
import { api, type ApiError } from "../lib/api.js";
import { TOKEN_ALLOWLIST } from "../lib/tokens.js";
import { useLiveFeed } from "../hooks/useWebSocket.js";

function NotificationToggle(): JSX.Element | null {
  const live = useLiveFeed();
  const [dismissed, setDismissed] = useState(false);
  const supported = typeof Notification !== "undefined";

  if (!supported || live.notificationsEnabled || dismissed) return null;

  return (
    <div className="flex items-center justify-between rounded-lg border border-slate-800 bg-slate-900 px-4 py-3 text-sm">
      <span className="text-slate-400">Get a browser notification for live trades, sends, and auto-sweeps as they happen.</span>
      <div className="flex shrink-0 gap-2">
        <button onClick={live.enableNotifications} className="rounded bg-slate-800 px-3 py-1.5 text-xs font-medium hover:bg-slate-700">
          Enable Notifications
        </button>
        <button onClick={() => setDismissed(true)} className="rounded px-3 py-1.5 text-xs text-slate-500 hover:bg-slate-800">
          Not now
        </button>
      </div>
    </div>
  );
}

export function Wallet(): JSX.Element {
  const queryClient = useQueryClient();
  const status = useQuery({ queryKey: ["hot-wallet-status"], queryFn: api.getHotWalletStatus });
  const [copied, setCopied] = useState(false);

  const createMutation = useMutation({
    mutationFn: api.createHotWallet,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["hot-wallet-status"] }),
  });

  // Shares a cache entry with ModeControl's own useQuery(["app-mode"]) below — React
  // Query dedupes by key, so this doesn't add a second poll.
  const mode = useQuery({ queryKey: ["app-mode"], queryFn: api.getMode });

  // Network-aware (see api.ts's getHotWalletBalance doc) — was api.getWalletBalance(pubkey),
  // which is hardcoded to devnet and silently showed the wrong balance after switching to
  // mainnet.
  const balance = useQuery({
    queryKey: ["hot-wallet-balance"],
    queryFn: api.getHotWalletBalance,
    enabled: !!status.data?.pubkey,
    refetchInterval: 15_000,
  });

  if (status.isLoading) return <p className="text-slate-400">Loading wallet status…</p>;
  if (status.error) return <p className="text-red-400">Failed to load wallet status.</p>;

  if (!status.data?.exists) {
    return (
      <div className="space-y-4">
        <NotificationToggle />
        <ModeControl hotWalletExists={false} />
        <section className="rounded-lg border border-slate-800 bg-slate-900 p-6">
          <h2 className="mb-2 text-lg font-semibold">Bot Trading Wallet</h2>
          <p className="mb-4 text-sm text-slate-400">
            This app can generate its own dedicated Solana wallet, used later for autonomous live trading. The private key
            is generated on this machine and stored in the OS keychain (Windows Credential Manager) — it never touches
            disk in readable form, is never displayed, logged, or sent anywhere. There's no import option: this is
            deliberately a fresh, dedicated wallet, not an existing one you already hold other assets in.
          </p>
          <p className="mb-4 text-sm text-amber-400">
            Creating the wallet itself never touches real funds — it only generates a keypair. The address it gets can
            later hold real funds on Mainnet or worthless test funds on Devnet, depending on the network selected in
            Trading Mode above.
          </p>
          <button
            onClick={() => createMutation.mutate()}
            disabled={createMutation.isPending}
            className="rounded bg-emerald-700 px-4 py-2 text-sm font-medium hover:bg-emerald-600 disabled:opacity-50"
          >
            {createMutation.isPending ? "Creating…" : "Create Wallet"}
          </button>
          {createMutation.isError && <p className="mt-3 text-sm text-red-400">{(createMutation.error as Error).message}</p>}
        </section>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <NotificationToggle />
      <ModeControl hotWalletExists={true} />
      <section className="rounded-lg border border-slate-800 bg-slate-900 p-6">
        <h2 className="mb-3 text-lg font-semibold">Bot Trading Wallet</h2>
        <p className="mb-1 text-xs text-slate-500">
          Deposit address ({mode.data?.network === "mainnet" ? "mainnet" : "devnet"}) — created {new Date(status.data.createdAt!).toLocaleString()}
        </p>
        <div className="mb-4 flex items-center gap-2">
          <code className="break-all rounded bg-slate-800 px-3 py-2 text-sm text-slate-200">{status.data.pubkey}</code>
          <button
            onClick={() => {
              navigator.clipboard.writeText(status.data!.pubkey!);
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            }}
            className="shrink-0 rounded bg-slate-800 px-3 py-2 text-xs hover:bg-slate-700"
          >
            {copied ? "Copied!" : "Copy"}
          </button>
        </div>
        <p className="mb-4 text-xs text-slate-500">
          {mode.data?.network === "mainnet" ? (
            <>Fund this address with a real SOL transfer to start testing real signed transactions. Both SOL (gas) and USDC will eventually be needed once autonomous trading is wired in.</>
          ) : (
            <>
              Fund this address with devnet SOL (e.g. via <code>solana airdrop</code> or a devnet faucet) to start testing
              real signed transactions. Both SOL (gas) and USDC will eventually be needed once autonomous trading is wired
              in.
            </>
          )}
        </p>

        {balance.data && (
          <div className="rounded border border-slate-800 bg-slate-950 p-3 text-sm">
            <p className="text-slate-200">{balance.data.solBalance.toFixed(4)} SOL</p>
            {balance.data.tokenBalances.map((t) => (
              <p key={t.mint} className="text-slate-400">
                {t.amount.toFixed(2)} {TOKEN_ALLOWLIST.find((tok) => tok.mint === t.mint)?.symbol ?? t.mint.slice(0, 4)}
              </p>
            ))}
            {balance.data.tokenBalances.length === 0 && balance.data.solBalance === 0 && (
              <p className="text-slate-500">
                No funds yet — deposit {mode.data?.network === "mainnet" ? "real" : "devnet"} SOL to this address to begin.
              </p>
            )}
          </div>
        )}
      </section>

      <SendForm
        onSent={() => {
          queryClient.invalidateQueries({ queryKey: ["hot-wallet-balance"] });
          queryClient.invalidateQueries({ queryKey: ["wallet-history"] });
        }}
      />
      <TransactionHistory />
      <AutoSweepPanel />
      <ExportKeyPanel />
    </div>
  );
}

const REQUIRED_EXPORT_PHRASE = "EXPORT PRIVATE KEY";

/** The most sensitive control in the app — reveals the raw private key that gives total,
 * irreversible control of this wallet's real funds. Gated behind a typed confirmation
 * phrase + acknowledgement (server-side backstop: routes/wallet.ts requires
 * `confirmed: true`). The revealed key is held only in this component's own local state —
 * never written to localStorage, never cached by React Query (this uses useMutation, not
 * useQuery, specifically so nothing here is retained or refetched) — and disappears the
 * instant the user clicks Hide or navigates away (component unmount). There is no
 * recovery phrase for this wallet (see hotWallet.ts's exportPrivateKeyBase58 doc): it was
 * generated as a raw keypair, not derived from a BIP39 mnemonic, so the private key below
 * is the complete, singular credential. */
function ExportKeyPanel(): JSX.Element {
  const [editing, setEditing] = useState(false);
  const [phrase, setPhrase] = useState("");
  const [ack, setAck] = useState(false);
  const [revealed, setRevealed] = useState<HotWalletKeyExport | null>(null);
  const [copied, setCopied] = useState(false);

  const mutation = useMutation({
    mutationFn: api.exportHotWalletKey,
    onSuccess: (result) => {
      setRevealed(result);
      setEditing(false);
      setPhrase("");
      setAck(false);
    },
  });

  const canSubmit = phrase.trim() === REQUIRED_EXPORT_PHRASE && ack;

  function hide(): void {
    setRevealed(null);
    setCopied(false);
  }

  return (
    <section className="rounded-lg border border-red-900 bg-red-950/10 p-6">
      <h2 className="mb-3 text-lg font-semibold text-red-300">Export Private Key</h2>
      <p className="mb-3 text-xs text-slate-500">
        For importing this exact wallet into Phantom, Solflare, or another Solana wallet app. There's no recovery
        phrase to go with it — this wallet was generated directly rather than from a seed phrase — so the private key
        below is the complete, singular credential for it.
      </p>

      {revealed ? (
        <div className="space-y-3">
          <div className="rounded border border-red-800 bg-red-950/40 p-3 text-sm">
            <p className="mb-2 font-medium text-red-300">
              Anyone who sees this key has permanent, total control of this wallet's funds. Never share it, never
              paste it into a website — only into a wallet app's own "Import Private Key" field.
            </p>
            <code className="block break-all rounded bg-slate-950 p-2 text-xs text-slate-200">{revealed.privateKeyBase58}</code>
          </div>
          <div className="flex flex-wrap gap-2">
            <button
              onClick={() => {
                navigator.clipboard.writeText(revealed.privateKeyBase58);
                setCopied(true);
                setTimeout(() => setCopied(false), 1500);
              }}
              className="rounded bg-slate-800 px-4 py-2 text-sm font-medium hover:bg-slate-700"
            >
              {copied ? "Copied!" : "Copy"}
            </button>
            <button onClick={hide} className="rounded bg-red-700 px-4 py-2 text-sm font-medium hover:bg-red-600">
              Hide
            </button>
          </div>
        </div>
      ) : !editing ? (
        <button onClick={() => setEditing(true)} className="rounded bg-slate-800 px-4 py-2 text-sm font-medium hover:bg-slate-700">
          Export Private Key
        </button>
      ) : (
        <div className="space-y-3">
          <div className="rounded border border-red-800 bg-red-950/30 p-3 text-sm text-red-300">
            <p className="mb-2">
              This will display the wallet's raw private key on screen. Type{" "}
              <code className="rounded bg-slate-800 px-1">{REQUIRED_EXPORT_PHRASE}</code> to confirm you understand the
              risk.
            </p>
            <input
              type="text"
              value={phrase}
              onChange={(e) => setPhrase(e.target.value)}
              placeholder={REQUIRED_EXPORT_PHRASE}
              className="mb-2 w-full rounded bg-slate-800 p-2 font-mono text-xs text-slate-100"
            />
            <label className="flex items-start gap-2">
              <input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} className="mt-0.5" />
              I understand anyone who sees this key gets full control of this wallet's funds, and I'm on a screen no
              one else can see or record.
            </label>
          </div>
          <div className="flex flex-wrap gap-2">
            <button
              disabled={!canSubmit || mutation.isPending}
              onClick={() => mutation.mutate()}
              className="rounded bg-red-700 px-4 py-2 text-sm font-medium hover:bg-red-600 disabled:opacity-50"
            >
              {mutation.isPending ? "Exporting…" : "Reveal Private Key"}
            </button>
            <button
              type="button"
              onClick={() => {
                setEditing(false);
                setPhrase("");
                setAck(false);
              }}
              className="rounded px-4 py-2 text-sm text-slate-400 hover:bg-slate-800"
            >
              Cancel
            </button>
          </div>
          {mutation.isError && <p className="text-sm text-red-400">{(mutation.error as Error).message}</p>}
        </div>
      )}
    </section>
  );
}

/** A standing rule that moves real funds with NO per-transfer confirmation once armed —
 * deliberately never pre-filled: every field starts empty/disabled, and turning it on
 * requires an explicit acknowledgement checkbox, mirroring ModeControl's own gating
 * philosophy. Enforced server-side too (routes/wallet.ts validates the destination and
 * threshold on every PUT) — this UI is belt, not suspenders. */
function AutoSweepPanel(): JSX.Element {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: ["auto-sweep"], queryFn: api.getAutoSweep });
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<AutoSweepConfig | null>(null);
  const [ack, setAck] = useState(false);

  const mutation = useMutation({
    mutationFn: api.setAutoSweep,
    onSuccess: (result) => {
      queryClient.setQueryData(["auto-sweep"], result);
      setEditing(false);
      setDraft(null);
      setAck(false);
    },
  });

  if (query.isLoading) return <p className="text-slate-400">Loading auto-sweep settings…</p>;
  if (!query.data) return <p className="text-red-400">Failed to load auto-sweep settings.</p>;

  const current = query.data;
  const effective = draft ?? current;
  const selectedToken = TOKEN_ALLOWLIST.find((t) => t.mint === effective.tokenMint) ?? TOKEN_ALLOWLIST[0];

  function startEditing(): void {
    setDraft(current.destination ? current : { ...current, tokenMint: TOKEN_ALLOWLIST[0].mint, tokenSymbol: TOKEN_ALLOWLIST[0].symbol });
    setEditing(true);
    setAck(false);
  }

  const turningOn = effective.enabled;
  const canSubmit = !turningOn || (effective.destination.trim().length > 0 && effective.thresholdAmount >= 0 && ack);

  return (
    <section className={`rounded-lg border p-6 ${current.enabled ? "border-amber-800 bg-amber-950/10" : "border-slate-800 bg-slate-900"}`}>
      <h2 className="mb-3 text-lg font-semibold">Auto-Sweep</h2>

      {!editing ? (
        <div>
          <p className="mb-3 text-xs text-slate-500">
            {current.enabled
              ? `ON — keeps ${current.thresholdAmount} ${current.tokenSymbol} in the wallet; anything above that is automatically sent to ${current.destination.slice(0, 4)}…${current.destination.slice(-4)} with no per-transfer confirmation.`
              : "Off. Optionally keep a fixed working balance in this wallet and automatically sweep anything above it to an address you choose — useful for skimming trading profit out as it accrues."}
          </p>
          <button onClick={startEditing} className="rounded bg-slate-800 px-4 py-2 text-sm font-medium hover:bg-slate-700">
            {current.enabled ? "Change" : "Configure"}
          </button>
        </div>
      ) : (
        <div className="space-y-3">
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
            <label className="block text-xs text-slate-400">
              Token to monitor
              <select
                value={effective.tokenMint}
                onChange={(e) => {
                  const t = TOKEN_ALLOWLIST.find((tok) => tok.mint === e.target.value)!;
                  setDraft({ ...effective, tokenMint: t.mint, tokenSymbol: t.symbol });
                }}
                className="mt-1 w-full rounded bg-slate-800 p-2 text-sm text-slate-100"
              >
                {TOKEN_ALLOWLIST.map((t) => (
                  <option key={t.mint} value={t.mint}>
                    {t.symbol}
                  </option>
                ))}
              </select>
            </label>
            <label className="block text-xs text-slate-400">
              Keep this much in the wallet ({selectedToken.symbol})
              <input
                type="number"
                step="any"
                min={0}
                value={effective.thresholdAmount}
                onChange={(e) => setDraft({ ...effective, thresholdAmount: Number(e.target.value) })}
                className="mt-1 w-full rounded bg-slate-800 p-2 text-sm text-slate-100"
              />
            </label>
          </div>
          <label className="block text-xs text-slate-400">
            Send excess to
            <input
              type="text"
              value={effective.destination}
              onChange={(e) => setDraft({ ...effective, destination: e.target.value })}
              placeholder="Destination Solana address"
              className="mt-1 w-full rounded bg-slate-800 p-2 font-mono text-xs text-slate-100"
            />
          </label>
          <label className="flex items-center gap-2 text-xs text-slate-400">
            <input type="checkbox" checked={effective.enabled} onChange={(e) => setDraft({ ...effective, enabled: e.target.checked })} />
            Enabled
          </label>

          {turningOn && (
            <label className="flex items-start gap-2 rounded border border-amber-800 bg-amber-950/30 p-3 text-sm text-amber-300">
              <input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} className="mt-0.5" />
              I understand this will automatically send real funds above the threshold to the address above, with no confirmation per transfer, checked roughly once a minute.
            </label>
          )}

          <div className="flex flex-wrap gap-2">
            <button
              disabled={!canSubmit || mutation.isPending}
              onClick={() => mutation.mutate(effective)}
              className={`rounded px-4 py-2 text-sm font-medium disabled:opacity-50 ${turningOn ? "bg-amber-700 hover:bg-amber-600" : "bg-emerald-700 hover:bg-emerald-600"}`}
            >
              {mutation.isPending ? "Saving…" : "Save"}
            </button>
            <button
              type="button"
              onClick={() => {
                setEditing(false);
                setDraft(null);
                setAck(false);
              }}
              className="rounded px-4 py-2 text-sm text-slate-400 hover:bg-slate-800"
            >
              Cancel
            </button>
          </div>

          {mutation.isError && <p className="text-sm text-red-400">{(mutation.error as Error).message}</p>}
        </div>
      )}
    </section>
  );
}

function TransactionHistory(): JSX.Element {
  const history = useQuery({ queryKey: ["wallet-history"], queryFn: api.getWalletHistory, refetchInterval: 15_000 });

  return (
    <section className="rounded-lg border border-slate-800 bg-slate-900 p-6">
      <h2 className="mb-3 text-lg font-semibold">Transaction History</h2>
      <p className="mb-4 text-xs text-slate-500">Every real, on-chain transaction this wallet has signed — manual sends and live strategy trades.</p>

      {history.isLoading && <p className="text-sm text-slate-500">Loading…</p>}
      {history.data && history.data.length === 0 && <p className="text-sm text-slate-500">No real transactions yet.</p>}

      <ul className="space-y-2">
        {history.data?.map((tx) => (
          <li key={tx.id} className="rounded border border-slate-800 bg-slate-950 p-3 text-sm">
            <div className="flex items-center justify-between">
              <span className="text-slate-200">
                {tx.kind === "send" ? "Sent" : tx.action === "sell" ? "Sold" : "Bought"} {tx.amount.toFixed(6)} {tx.tokenSymbol}
                {tx.kind === "trade" && <span className="ml-2 text-xs text-slate-500">via {tx.strategyId}</span>}
              </span>
              <span className="rounded-full bg-slate-800 px-2 py-0.5 text-xs text-slate-400">{tx.network}</span>
            </div>
            {tx.kind === "send" && <p className="mt-1 break-all font-mono text-xs text-slate-500">to {tx.destination}</p>}
            <div className="mt-1 flex items-center justify-between text-xs text-slate-500">
              <span>{new Date(tx.timestamp).toLocaleString()}</span>
              <a
                href={`https://explorer.solana.com/tx/${tx.txHash}?cluster=${tx.network === "mainnet" ? "mainnet-beta" : "devnet"}`}
                target="_blank"
                rel="noreferrer"
                className="text-emerald-400 underline"
              >
                {tx.confirmationSlot === null ? "unconfirmed — view" : `slot ${tx.confirmationSlot}`}
              </a>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}

function SendForm({ onSent }: { onSent: () => void }): JSX.Element {
  // Shares a cache entry with ModeControl's/Wallet's own useQuery(["app-mode"]) — dedupes,
  // no extra poll — just needed here so the network label and the preview call target the
  // network the send will actually go to.
  const mode = useQuery({ queryKey: ["app-mode"], queryFn: api.getMode });
  const [tokenMint, setTokenMint] = useState<string>(TOKEN_ALLOWLIST[0].mint);
  const [destination, setDestination] = useState("");
  const [amount, setAmount] = useState("");
  const [reviewing, setReviewing] = useState(false);
  const [needsAck, setNeedsAck] = useState<{ usdValue: number } | null>(null);
  const [acked, setAcked] = useState(false);

  // Real fee quote + real dry-run simulation, fetched the moment the user asks to review —
  // same idea as Phantom/Solflare's own send-review screen. Never signs or submits
  // anything on its own (see api.ts/routes/wallet.ts's send/preview doc).
  const previewMutation = useMutation({ mutationFn: api.previewSend });

  const sendMutation = useMutation({
    mutationFn: api.sendFromWallet,
    onSuccess: () => {
      setReviewing(false);
      setNeedsAck(null);
      setAcked(false);
      setDestination("");
      setAmount("");
      previewMutation.reset();
      onSent();
    },
    onError: (err: ApiError) => {
      if (err.requiresAcknowledgement) setNeedsAck({ usdValue: err.usdValue ?? 0 });
    },
  });

  const selectedToken = TOKEN_ALLOWLIST.find((t) => t.mint === tokenMint) ?? TOKEN_ALLOWLIST[0];
  const parsedAmount = Number(amount);
  const canReview = destination.trim().length > 0 && parsedAmount > 0;

  function startReview(): void {
    setReviewing(true);
    previewMutation.mutate({ tokenMint, amount: parsedAmount, destination: destination.trim() });
  }

  function cancelReview(): void {
    setReviewing(false);
    setNeedsAck(null);
    setAcked(false);
    previewMutation.reset();
  }

  function confirmSend(): void {
    sendMutation.mutate({ tokenMint, amount: parsedAmount, destination: destination.trim(), acknowledgedLargeSend: acked || undefined });
  }

  const simulationError = previewMutation.data?.simulationError ?? null;
  // Blocks on a known-bad simulation rather than just warning — the whole point of
  // previewing is to stop a send that's already known to fail before it wastes a real fee
  // attempt. Also blocks while the preview itself is still loading/failed, so there's
  // never a window where "Confirm Send" is clickable with no fee/simulation info shown yet.
  const canConfirm = !sendMutation.isPending && (!needsAck || acked) && !!previewMutation.data && !simulationError;

  return (
    <section className="rounded-lg border border-slate-800 bg-slate-900 p-6">
      <h2 className="mb-3 text-lg font-semibold">Send</h2>
      <p className="mb-4 text-xs text-slate-500">
        A real, on-chain, irreversible signed transfer from this wallet ({mode.data?.network ?? "devnet"}).
      </p>

      {!reviewing ? (
        <div className="space-y-3">
          <label className="block text-xs text-slate-400">
            Token
            <select
              value={tokenMint}
              onChange={(e) => setTokenMint(e.target.value)}
              className="mt-1 w-full rounded bg-slate-800 p-2 text-sm text-slate-100"
            >
              {TOKEN_ALLOWLIST.map((t) => (
                <option key={t.mint} value={t.mint}>
                  {t.symbol}
                </option>
              ))}
            </select>
          </label>
          <label className="block text-xs text-slate-400">
            Destination address
            <input
              type="text"
              value={destination}
              onChange={(e) => setDestination(e.target.value)}
              placeholder="Recipient's Solana address"
              className="mt-1 w-full rounded bg-slate-800 p-2 font-mono text-xs text-slate-100"
            />
          </label>
          <label className="block text-xs text-slate-400">
            Amount ({selectedToken.symbol})
            <input
              type="number"
              step="any"
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
              className="mt-1 w-full rounded bg-slate-800 p-2 text-sm text-slate-100"
            />
          </label>
          <button
            disabled={!canReview}
            onClick={startReview}
            className="rounded bg-slate-800 px-4 py-2 text-sm font-medium hover:bg-slate-700 disabled:opacity-50"
          >
            Review Send
          </button>
        </div>
      ) : (
        <div className="space-y-3">
          <div className="rounded border border-amber-800 bg-amber-950/30 p-3 text-sm">
            <p className="text-amber-300">
              Send {amount} {selectedToken.symbol} to:
            </p>
            <p className="mt-1 break-all font-mono text-xs text-slate-300">{destination}</p>
            <p className="mt-2 text-xs text-slate-500">This is irreversible once confirmed on-chain. Double-check the destination address.</p>
          </div>

          <div className="rounded border border-slate-800 bg-slate-950 p-3 text-sm">
            {previewMutation.isPending && <p className="text-slate-500">Estimating fee and simulating…</p>}
            {previewMutation.isError && <p className="text-red-400">Couldn't preview this send: {(previewMutation.error as Error).message}</p>}
            {previewMutation.data && (
              <>
                <p className="text-slate-300">Estimated network fee: ~{previewMutation.data.estimatedFeeSol.toFixed(6)} SOL</p>
                {simulationError ? (
                  <p className="mt-1 text-red-400">{simulationError} — this send would fail. Double-check the amount, destination, and balance.</p>
                ) : (
                  <p className="mt-1 text-emerald-400">Simulation succeeded — this transaction is expected to go through.</p>
                )}
              </>
            )}
          </div>

          {needsAck && (
            <label className="flex items-start gap-2 rounded border border-red-800 bg-red-950/30 p-3 text-sm text-red-300">
              <input type="checkbox" checked={acked} onChange={(e) => setAcked(e.target.checked)} className="mt-0.5" />
              This is an unusually large transfer (~${needsAck.usdValue.toFixed(2)}) — I've double-checked the destination
              address and want to proceed.
            </label>
          )}

          <div className="flex flex-wrap gap-2">
            <button
              disabled={!canConfirm}
              onClick={confirmSend}
              className="rounded bg-red-700 px-4 py-2 text-sm font-medium hover:bg-red-600 disabled:opacity-50"
            >
              {sendMutation.isPending ? "Sending…" : "Confirm Send"}
            </button>
            <button type="button" onClick={cancelReview} className="rounded px-4 py-2 text-sm text-slate-400 hover:bg-slate-800">
              Cancel
            </button>
          </div>

          {sendMutation.isError && !needsAck && <p className="text-sm text-red-400">{(sendMutation.error as Error).message}</p>}
        </div>
      )}

      {sendMutation.isSuccess && (
        <div className="mt-4 rounded border border-emerald-800 bg-emerald-950/30 p-3 text-sm">
          <p className="text-emerald-300">
            Sent. {sendMutation.data.confirmationSlot === null ? "Submitted, not yet confirmed — check the link below." : `Confirmed at slot ${sendMutation.data.confirmationSlot}.`}
          </p>
          <a href={sendMutation.data.explorerUrl} target="_blank" rel="noreferrer" className="mt-1 block break-all text-xs text-emerald-400 underline">
            {sendMutation.data.explorerUrl}
          </a>
        </div>
      )}
    </section>
  );
}

const VERDICT_STYLES: Record<BacktestVerdict, string> = {
  profitable: "bg-emerald-900 text-emerald-300",
  "not-profitable": "bg-red-900 text-red-300",
  untested: "bg-amber-900 text-amber-300",
};

const VERDICT_LABELS: Record<BacktestVerdict, string> = {
  profitable: "Backtest: profitable",
  "not-profitable": "Backtest: not profitable",
  untested: "Backtest: untested",
};

/** Two modes, not two independently-variable toggles — like switching between two
 * separate accounts. Paper always means Devnet/simulated, Live always means
 * Mainnet/real, and there's no third combination reachable through this UI or the
 * server (see execution/appMode.ts — `network` is derived from `tradingMode`, never set
 * independently). Switching either direction requires typing the target mode's name
 * ("PAPER" or "LIVE") to confirm; switching TO Live additionally requires reviewing
 * every currently-active strategy's own backtest verdict and acknowledging it (most
 * strategies in this app have no confirmed profitable backtest — see
 * docs/ARCHITECTURE.md). */
function ModeControl({ hotWalletExists }: { hotWalletExists: boolean }): JSX.Element {
  const queryClient = useQueryClient();
  const modeQuery = useQuery({ queryKey: ["app-mode"], queryFn: api.getMode });
  const [switchingTo, setSwitchingTo] = useState<TradingMode | null>(null);
  const [phrase, setPhrase] = useState("");
  const [liveAck, setLiveAck] = useState(false);

  const activeStrategiesQuery = useQuery({
    queryKey: ["mode-active-strategies"],
    queryFn: async () => {
      const [configs, catalog] = await Promise.all([api.getStrategies(), api.getStrategyCatalog()]);
      const catalogById = new Map(catalog.map((m) => [m.id, m]));
      return configs.filter((c) => c.active).map((c) => ({ config: c, meta: catalogById.get(c.strategyId) }));
    },
    enabled: switchingTo === "live",
  });

  const setModeMutation = useMutation({
    mutationFn: api.setMode,
    onSuccess: (result) => {
      queryClient.setQueryData(["app-mode"], result);
      // Balance/history are network-aware (see api.ts's getHotWalletBalance doc) — without
      // this, switching modes would keep showing the OLD network's balance for up to 15s
      // (the poll interval) instead of updating immediately.
      queryClient.invalidateQueries({ queryKey: ["hot-wallet-balance"] });
      queryClient.invalidateQueries({ queryKey: ["wallet-history"] });
      setSwitchingTo(null);
      setPhrase("");
      setLiveAck(false);
    },
  });

  if (modeQuery.isLoading) return <p className="text-slate-400">Loading trading mode…</p>;
  if (!modeQuery.data) return <p className="text-red-400">Failed to load trading mode.</p>;

  const current = modeQuery.data.tradingMode;

  function startSwitch(target: TradingMode): void {
    if (target === current) return;
    setSwitchingTo(target);
    setPhrase("");
    setLiveAck(false);
  }

  function cancelSwitch(): void {
    setSwitchingTo(null);
    setPhrase("");
    setLiveAck(false);
  }

  const requiredPhrase = switchingTo === "live" ? "LIVE" : "PAPER";
  const needsWallet = switchingTo === "live" && !hotWalletExists;
  const canConfirm = !needsWallet && phrase.trim() === requiredPhrase && (switchingTo !== "live" || liveAck);

  return (
    <section className={`rounded-lg border p-6 ${current === "live" ? "border-red-800 bg-red-950/20" : "border-slate-800 bg-slate-900"}`}>
      <div className="mb-3 flex items-center justify-between">
        <h2 className="text-lg font-semibold">Trading Mode</h2>
        {/* Reads the actual stored network rather than assuming Paper=Devnet/Live=Mainnet
            holds — that invariant is enforced for every new write (see
            execution/appMode.ts), but this badge staying honest about the real value
            means any future mismatch (e.g. a pre-migration row) surfaces immediately
            instead of being silently masked by a hardcoded label. */}
        <span className={`rounded-full px-3 py-1 text-xs font-medium ${current === "live" ? "bg-red-900 text-red-300" : "bg-slate-800 text-slate-300"}`}>
          {current === "live" ? "LIVE" : "PAPER"} · {modeQuery.data.network === "mainnet" ? "Mainnet" : "Devnet"}
        </span>
      </div>

      <p className="mb-4 text-xs text-slate-500">
        {current === "live"
          ? "Everything — wallet balance, sends, active strategies — is real, on Mainnet. Trades execute autonomously with no per-trade confirmation."
          : "Everything — wallet balance, sends, active strategies — is simulated, on Devnet test funds. No real money is ever at risk."}
      </p>

      <div className="mb-4 grid grid-cols-2 gap-2">
        <button
          disabled={current === "paper"}
          onClick={() => startSwitch("paper")}
          className={`rounded px-4 py-3 text-sm font-medium ${current === "paper" ? "cursor-default bg-slate-800 text-slate-500" : "bg-slate-800 hover:bg-slate-700"}`}
        >
          Paper{current === "paper" && " (current)"}
        </button>
        <button
          disabled={current === "live"}
          onClick={() => startSwitch("live")}
          className={`rounded px-4 py-3 text-sm font-medium ${current === "live" ? "cursor-default bg-red-900 text-red-300" : "bg-slate-800 hover:bg-red-900 hover:text-red-200"}`}
        >
          Live{current === "live" && " (current)"}
        </button>
      </div>

      {switchingTo && (
        <div className="space-y-3">
          {needsWallet && (
            <p className="rounded border border-amber-800 bg-amber-950/30 p-3 text-sm text-amber-300">
              Create a hot wallet first (below) before switching to Live — there's nothing to sign with yet.
            </p>
          )}

          {switchingTo === "live" && !needsWallet && (
            <div className="rounded border border-red-800 bg-red-950/30 p-3 text-sm">
              <p className="mb-2 text-red-300">
                Switching to Live moves everything to Mainnet: real wallet balance, real sends, and active strategies
                trading real funds autonomously — no per-trade confirmation. Review their backtest track record:
              </p>
              {activeStrategiesQuery.isLoading && <p className="text-slate-500">Loading active strategies…</p>}
              {activeStrategiesQuery.data && activeStrategiesQuery.data.length === 0 && (
                <p className="text-slate-500">No active strategies right now — Live mode would sit idle until one is activated.</p>
              )}
              <ul className="mb-3 space-y-1">
                {activeStrategiesQuery.data?.map(({ config, meta }) => (
                  <li key={config.id} className="flex items-center justify-between rounded bg-slate-900 px-3 py-2">
                    <span className="text-slate-200">
                      {meta?.name ?? config.strategyId} — {config.tokenSymbol}
                    </span>
                    <span className={`rounded-full px-2 py-0.5 text-xs ${VERDICT_STYLES[meta?.backtestVerdict ?? "untested"]}`}>
                      {VERDICT_LABELS[meta?.backtestVerdict ?? "untested"]}
                    </span>
                  </li>
                ))}
              </ul>
              <label className="flex items-start gap-2 text-red-300">
                <input type="checkbox" checked={liveAck} onChange={(e) => setLiveAck(e.target.checked)} className="mt-0.5" />
                I understand active strategies will trade real funds autonomously, most have no confirmed profitable
                backtest, and I'm switching to Live deliberately.
              </label>
            </div>
          )}

          {switchingTo === "paper" && (
            <p className="rounded border border-slate-700 bg-slate-950 p-3 text-sm text-slate-300">
              Switching back to Paper stops all live trading and returns everything — wallet view, sends — to Devnet
              test funds.
            </p>
          )}

          {!needsWallet && (
            <label className="block text-xs text-slate-400">
              Type <code className="rounded bg-slate-800 px-1">{requiredPhrase}</code> to confirm
              <input
                type="text"
                value={phrase}
                onChange={(e) => setPhrase(e.target.value)}
                placeholder={requiredPhrase}
                className="mt-1 w-full rounded bg-slate-800 p-2 font-mono text-xs text-slate-100"
              />
            </label>
          )}

          <div className="flex flex-wrap gap-2">
            <button
              disabled={!canConfirm || setModeMutation.isPending}
              onClick={() => setModeMutation.mutate(switchingTo)}
              className={`rounded px-4 py-2 text-sm font-medium disabled:opacity-50 ${switchingTo === "live" ? "bg-red-700 hover:bg-red-600" : "bg-emerald-700 hover:bg-emerald-600"}`}
            >
              {setModeMutation.isPending ? "Switching…" : `Confirm Switch to ${switchingTo === "live" ? "Live" : "Paper"}`}
            </button>
            <button type="button" onClick={cancelSwitch} className="rounded px-4 py-2 text-sm text-slate-400 hover:bg-slate-800">
              Cancel
            </button>
          </div>

          {setModeMutation.isError && <p className="text-sm text-red-400">{(setModeMutation.error as Error).message}</p>}
        </div>
      )}
    </section>
  );
}
