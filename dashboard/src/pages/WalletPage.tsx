import { useState } from 'react';
import { api } from '../api/client';
import { usePolling } from '../api/usePolling';
import { StatTile } from '../components/StatTile';

export function WalletPage() {
  const { data: wallet, error, loading } = usePolling(api.wallet, 10000);
  const [copied, setCopied] = useState(false);

  const copyAddress = async () => {
    if (!wallet) return;
    await navigator.clipboard.writeText(wallet.address);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="flex flex-col gap-4">
      <h1 className="text-lg font-semibold">Wallet</h1>
      {error && <p className="text-sm text-[var(--status-critical)]">{error}</p>}

      {!loading && !wallet && !error && (
        <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-6 text-sm text-[var(--text-secondary)]">
          No wallet configured on this server. Set <code>WALLET_KEYPAIR_PATH</code> to a
          keypair file (e.g. one generated via{' '}
          <code>cargo run -p solstice-blockchain --example gen_devnet_keypair</code>) and
          restart <code>solstice-api</code>.
        </div>
      )}

      {wallet && (
        <>
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            <StatTile label="Balance" value={`${wallet.balance_sol.toFixed(4)} SOL`} />
            <StatTile label="Lamports" value={wallet.balance_lamports.toLocaleString()} />
          </div>

          <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
            <div className="mb-2 text-sm font-medium text-[var(--text-secondary)]">
              Deposit address
            </div>
            <div className="flex items-center gap-2">
              <code className="flex-1 overflow-x-auto whitespace-nowrap rounded-md bg-[var(--page)] px-3 py-2 text-sm">
                {wallet.address}
              </code>
              <button
                type="button"
                onClick={copyAddress}
                className="shrink-0 rounded-md border border-[var(--border)] px-3 py-2 text-sm font-medium text-[var(--text-primary)] hover:bg-[var(--border)]"
              >
                {copied ? 'Copied' : 'Copy'}
              </button>
            </div>
            <p className="mt-3 text-xs text-[var(--text-muted)]">
              This server can only read this wallet's balance — it has no endpoint that can
              sign or send anything. Sending funds anywhere from this wallet requires the
              private key file directly, on the machine it lives on.
            </p>
          </div>
        </>
      )}
    </div>
  );
}
