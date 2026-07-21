import { useState } from 'react';
import { api } from '../api/client';
import { usePolling } from '../api/usePolling';
import { StatTile } from '../components/StatTile';
import type { ConvertDirection, ConvertResponse } from '../api/types';

const CONVERT_CONFIRM_PHRASE = 'CONVERT';

export function WalletPage() {
  const { data: wallet, error, loading } = usePolling(api.wallet, 10000);
  const { data: devnet } = usePolling(api.walletDevnet, 15000);
  const [copied, setCopied] = useState(false);

  const [direction, setDirection] = useState<ConvertDirection>('sol_to_usdc');
  const [amountInput, setAmountInput] = useState('');
  const [confirmText, setConfirmText] = useState('');
  const [converting, setConverting] = useState(false);
  const [convertError, setConvertError] = useState<string | null>(null);
  const [convertResult, setConvertResult] = useState<ConvertResponse | null>(null);

  const copyAddress = async () => {
    if (!wallet) return;
    await navigator.clipboard.writeText(wallet.address);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const handleConvert = async () => {
    const amount = Number(amountInput);
    if (!Number.isFinite(amount) || amount <= 0) {
      setConvertError('Enter a valid positive amount.');
      return;
    }
    if (confirmText !== CONVERT_CONFIRM_PHRASE) return;

    setConverting(true);
    setConvertError(null);
    setConvertResult(null);
    try {
      const result = await api.walletConvert({ direction, amount });
      setConvertResult(result);
      setAmountInput('');
      setConfirmText('');
    } catch (err) {
      setConvertError(err instanceof Error ? err.message : String(err));
    } finally {
      setConverting(false);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <h1 className="text-lg font-semibold">Wallet</h1>
        <span className="inline-flex items-center gap-1.5 rounded-full border border-[var(--status-warning)]/40 bg-[var(--status-warning)]/15 px-2.5 py-1 text-xs font-semibold uppercase tracking-wide text-[var(--status-warning)]">
          <span className="h-1.5 w-1.5 rounded-full bg-[var(--status-warning)]" aria-hidden />
          Live — real funds
        </span>
      </div>
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
          <div className="grid grid-cols-1 gap-4 md:grid-cols-4">
            <StatTile label="SOL balance (mainnet)" value={`${wallet.balance_sol.toFixed(4)} SOL`} />
            <StatTile label="USDC balance (mainnet)" value={`${wallet.usdc_balance.toFixed(2)} USDC`} />
            {devnet && (
              <StatTile label="SOL balance (devnet)" value={`${devnet.balance_sol.toFixed(4)} SOL`} />
            )}
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
              The same address holds a separate balance on devnet and mainnet — they're
              independent ledgers, shown separately above.
            </p>
          </div>

          <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
            <div className="mb-3 text-sm font-medium text-[var(--text-secondary)]">
              Convert SOL ⇄ USDC
            </div>
            <p className="mb-3 text-xs text-[var(--text-muted)]">
              <strong className="text-[var(--status-warning)]">
                Executes a real, irreversible on-chain swap
              </strong>{' '}
              using this wallet's own mainnet funds — there is no undo. Nothing happens
              until you type the confirmation phrase and click Convert.
            </p>

            <div className="flex flex-col gap-3">
              <div className="flex items-center gap-2">
                <select
                  value={direction}
                  onChange={(e) => setDirection(e.target.value as ConvertDirection)}
                  className="rounded-md border border-[var(--border)] bg-[var(--page)] px-3 py-2 text-sm"
                >
                  <option value="sol_to_usdc">SOL → USDC</option>
                  <option value="usdc_to_sol">USDC → SOL</option>
                </select>
                <input
                  type="number"
                  min="0"
                  step="any"
                  value={amountInput}
                  onChange={(e) => setAmountInput(e.target.value)}
                  placeholder={direction === 'sol_to_usdc' ? 'Amount in SOL' : 'Amount in USDC'}
                  className="flex-1 rounded-md border border-[var(--border)] bg-[var(--page)] px-3 py-2 text-sm"
                />
              </div>

              <label className="text-xs text-[var(--text-muted)]">
                Type <code>{CONVERT_CONFIRM_PHRASE}</code> to confirm:
              </label>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={confirmText}
                  onChange={(e) => setConfirmText(e.target.value)}
                  placeholder={CONVERT_CONFIRM_PHRASE}
                  className="flex-1 rounded-md border border-[var(--border)] bg-[var(--page)] px-3 py-2 text-sm"
                />
                <button
                  type="button"
                  onClick={handleConvert}
                  disabled={converting || amountInput === '' || confirmText !== CONVERT_CONFIRM_PHRASE}
                  className="shrink-0 rounded-md bg-[var(--status-warning)] px-4 py-2 text-sm font-semibold text-black hover:opacity-90 disabled:opacity-40"
                >
                  {converting ? 'Converting…' : 'Convert'}
                </button>
              </div>
            </div>

            {convertError && (
              <p className="mt-3 text-sm text-[var(--status-critical)]">{convertError}</p>
            )}
            {convertResult && (
              <div className="mt-3 rounded-md border border-[var(--status-good)]/40 bg-[var(--status-good)]/10 p-3 text-sm">
                <div className="text-[var(--status-good)]">
                  Converted {convertResult.input_amount} → {convertResult.output_amount.toFixed(6)}{' '}
                  ({convertResult.method})
                </div>
                {convertResult.signatures.map((sig) => (
                  <div key={sig} className="mt-1 break-all text-xs text-[var(--text-muted)]">
                    {sig}
                  </div>
                ))}
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
